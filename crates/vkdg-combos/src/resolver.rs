use std::sync::Arc;

use parking_lot::RwLock;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId};

use crate::plan::Combo;

/// Resolves a model name or request metadata to a Combo.
///
/// Resolution order (matches `OmniRoute`'s behavior):
/// 1. Exact combo ID match ("coding-fast" literal)
/// 2. Combo `match_patterns` glob ("code:*", "combo/*")
/// 3. Falls through to bare model routing
///
/// The table is swapped whole by [`ComboResolver::replace`], so combos edited
/// in the admin API apply to the next request without a restart.
pub struct ComboResolver {
    combos: RwLock<Arc<Vec<Combo>>>,
}

impl ComboResolver {
    pub fn new(combos: Vec<Combo>) -> Self {
        Self {
            combos: RwLock::new(Arc::new(combos)),
        }
    }

    /// Resolve a model name or combo name to a Combo, if any matches.
    pub fn resolve(&self, name: &str) -> Option<Combo> {
        let combos = self.all();
        // 1. Exact combo ID
        if let Some(c) = combos.iter().find(|c| c.id == name) {
            return Some(c.clone());
        }
        // 2. Pattern match
        combos
            .iter()
            .find(|c| vkdg_core::glob::matches_any(&c.match_patterns, name))
            .cloned()
    }

    pub fn all(&self) -> Arc<Vec<Combo>> {
        Arc::clone(&self.combos.read())
    }

    pub fn replace(&self, combos: Vec<Combo>) {
        *self.combos.write() = Arc::new(combos);
    }

    /// The combos as routes for the router, in the resolver's order: first one
    /// id-only route per combo, then one pattern route per combo that has
    /// patterns. The router takes the first match, so `code:python` goes to
    /// the combo with that id even when an earlier combo's `code:*` matches it.
    pub fn routes(&self) -> Vec<RouteConfig> {
        let combos = self.all();
        // A combo without targets carries only policies (budget, compression);
        // as a route it would match first and then fail with no connection.
        let routable = || combos.iter().filter(|c| !c.targets.is_empty());
        let by_id = routable().map(|c| route_of(c, vec![c.id.clone()]));
        let by_pattern = routable()
            .filter(|c| !c.match_patterns.is_empty())
            .map(|c| route_of(c, c.match_patterns.clone()));
        by_id.chain(by_pattern).collect()
    }
}

/// Route id of a combo, as it appears in routing decisions and request history.
pub fn combo_route_id(combo_id: &str) -> RouteId {
    RouteId(format!("combo:{combo_id}"))
}

fn route_of(c: &Combo, match_models: Vec<String>) -> RouteConfig {
    RouteConfig {
        id: combo_route_id(&c.id),
        match_models,
        strategy: c.strategy.clone(),
        targets: c.targets.clone(),
        plugin_hooks: PluginHooks::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Combo;
    use vkdg_routing::StrategyKind;

    fn make_combo(id: &str, patterns: &[&str]) -> Combo {
        Combo {
            id: id.into(),
            match_patterns: patterns.iter().map(|s| (*s).to_string()).collect(),
            strategy: StrategyKind::FallbackChain,
            targets: vec![],
            compression: None,
            cache: None,
            budget: None,
            mode_pack: None,
            model: None,
        }
    }

    // Plausible wrong impl: resolver returns first combo regardless of name
    #[test]
    fn exact_id_match() {
        let r = ComboResolver::new(vec![
            make_combo("coding-fast", &[]),
            make_combo("quality-first", &[]),
        ]);
        assert_eq!(
            r.resolve("coding-fast").map(|c| c.id).as_deref(),
            Some("coding-fast")
        );
        assert_eq!(
            r.resolve("quality-first").map(|c| c.id).as_deref(),
            Some("quality-first")
        );
    }

    // Plausible wrong impl: glob patterns ignored, only exact matches work
    #[test]
    fn glob_pattern_match() {
        let r = ComboResolver::new(vec![make_combo("code-combo", &["code:*"])]);
        assert!(r.resolve("code:python").is_some());
        assert!(r.resolve("code:rust").is_some());
        assert!(r.resolve("vision:photo").is_none());
    }

    // Plausible wrong impl: bare model name incorrectly resolves to a combo
    #[test]
    fn no_match_returns_none() {
        let r = ComboResolver::new(vec![make_combo("coding-fast", &["code:*"])]);
        assert!(r.resolve("claude-3-5-haiku-20241022").is_none());
    }

    // Plausible wrong impl: exact ID checked after patterns, shadowed by glob
    #[test]
    fn exact_id_wins_over_glob() {
        let r = ComboResolver::new(vec![
            make_combo("code:python", &[]),       // exact id
            make_combo("catch-all", &["code:*"]), // glob
        ]);
        assert_eq!(
            r.resolve("code:python").map(|c| c.id).as_deref(),
            Some("code:python")
        );
    }

    // Plausible wrong impl: '*' at end of pattern doesn't match empty suffix
    #[test]
    fn glob_star_matches_empty_suffix() {
        let r = ComboResolver::new(vec![make_combo("combo", &["prefix*"])]);
        assert!(r.resolve("prefix").is_some());
        assert!(r.resolve("prefix-extended").is_some());
    }

    // The router takes the first matching route. If a combo's patterns came
    // before another combo's id, the router and the resolver would pick
    // different combos: targets from one, budget and compression from the other.
    #[tokio::test]
    async fn router_order_matches_resolver_order() {
        let mut glob = make_combo("catch-all", &["code:*"]);
        glob.targets = vec![vkdg_core::ConnectionId("c-glob".into())];
        let mut exact = make_combo("code:python", &[]);
        exact.targets = vec![vkdg_core::ConnectionId("c-exact".into())];
        let r = ComboResolver::new(vec![glob, exact]);
        let router = vkdg_routing::Router::new(vec![]);
        router.replace_combo_routes(r.routes());

        let envelope = |model: &str| vkdg_core::RequestEnvelope {
            request_id: vkdg_core::RequestId::new(),
            client_id: vkdg_core::ClientId("t".into()),
            tenant_id: vkdg_core::TenantId("t".into()),
            session_key: None,
            api_type: vkdg_core::ApiType::AnthropicMessages,
            model_requested: model.into(),
            deadline: None,
            mode_pack_override: None,
            compression_override: None,
            cache_bypass: false,
            include_think_tags: false,
            client_ip: None,
        };
        let (f, h) = (
            vkdg_routing::EligibilityFilter::default(),
            vkdg_routing::RoutingHints::default(),
        );
        let got = router
            .route(&envelope("code:python"), &f, &h)
            .await
            .unwrap();
        assert_eq!(got.route_id, combo_route_id("code:python"));
        assert_eq!(r.resolve("code:python").unwrap().id, "code:python");
        let got = router.route(&envelope("code:rust"), &f, &h).await.unwrap();
        assert_eq!(got.route_id, combo_route_id("catch-all"));
    }
}
