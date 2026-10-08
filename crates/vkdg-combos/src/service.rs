//! The one place combos change: validate, persist, then apply to the live
//! resolver and router, under one lock so concurrent admin writes never lose
//! an edit or race on the temp file.

use std::sync::Arc;

use parking_lot::Mutex;
use vkdg_routing::{Router, StrategyKind};

use crate::{Combo, ComboResolver, ComboStore};

pub struct ComboService {
    store: ComboStore,
    resolver: Arc<ComboResolver>,
    router: Option<Arc<Router>>,
    write: Mutex<()>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ComboError {
    Invalid(String),
    NotFound,
    Exists,
    Storage(String),
}

impl ComboService {
    /// Load combos from `store` and apply them to `resolver` and `router`.
    pub fn open(
        store: ComboStore,
        resolver: Arc<ComboResolver>,
        router: Option<Arc<Router>>,
    ) -> Result<Self, String> {
        let combos = store.load()?;
        let svc = Self {
            store,
            resolver,
            router,
            write: Mutex::new(()),
        };
        svc.apply(combos);
        Ok(svc)
    }

    pub fn resolver(&self) -> &Arc<ComboResolver> {
        &self.resolver
    }

    pub fn list(&self) -> Arc<Vec<Combo>> {
        self.resolver.all()
    }

    pub fn create(&self, combo: Combo) -> Result<Combo, ComboError> {
        validate(&combo)?;
        self.edit(|all| {
            if all.iter().any(|c| c.id == combo.id) {
                return Err(ComboError::Exists);
            }
            all.push(combo.clone());
            Ok(combo.clone())
        })
    }

    /// Replace the routing fields of combo `id` (patterns, strategy, targets,
    /// model). Policies the edit does not carry (compression, cache, mode
    /// pack, and the budget when `combo.budget` is `None`) are kept: merged
    /// under the write lock so a concurrent edit cannot slip in between.
    pub fn update(&self, id: &str, mut combo: Combo) -> Result<Combo, ComboError> {
        combo.id = id.to_owned();
        validate(&combo)?;
        self.edit(|all| {
            let slot = all
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or(ComboError::NotFound)?;
            slot.match_patterns = combo.match_patterns;
            slot.strategy = combo.strategy;
            slot.targets = combo.targets;
            slot.model = combo.model;
            if combo.budget.is_some() {
                slot.budget = combo.budget;
            }
            Ok(slot.clone())
        })
    }

    pub fn delete(&self, id: &str) -> Result<(), ComboError> {
        self.edit(|all| {
            let before = all.len();
            all.retain(|c| c.id != id);
            if all.len() == before {
                Err(ComboError::NotFound)
            } else {
                Ok(())
            }
        })
    }

    /// Read-modify-write under the lock. Disk first: an edit that cannot be
    /// saved is not applied, so a restart never undoes what the operator saw.
    fn edit<T>(
        &self,
        f: impl FnOnce(&mut Vec<Combo>) -> Result<T, ComboError>,
    ) -> Result<T, ComboError> {
        let _w = self.write.lock();
        let mut all = self.resolver.all().as_ref().clone();
        let out = f(&mut all)?;
        self.store.save(&all).map_err(ComboError::Storage)?;
        self.apply(all);
        Ok(out)
    }

    fn apply(&self, combos: Vec<Combo>) {
        self.resolver.replace(combos);
        if let Some(router) = &self.router {
            router.replace_combo_routes(self.resolver.routes());
        }
    }
}

/// Strategies the router can run for a combo. Anything else would be saved
/// and then fail every request that matches it.
fn validate(c: &Combo) -> Result<(), ComboError> {
    let bad = |m: &str| Err(ComboError::Invalid(m.to_owned()));
    if c.id.trim().is_empty() || c.id.contains(['*', '?']) {
        return bad("id must be a non-empty name without * or ?");
    }
    if c.targets.is_empty() {
        return bad("a combo needs at least one target connection");
    }
    if c.model.as_deref().map_or(true, |m| m.trim().is_empty()) {
        return bad(
            "model is required: it is what the provider receives when a request names the combo",
        );
    }
    match c.strategy {
        StrategyKind::RoundRobin
        | StrategyKind::FallbackChain
        | StrategyKind::LowestLatency
        | StrategyKind::PowerOfTwoChoices
        | StrategyKind::Fusion { .. } => Ok(()),
        StrategyKind::Scored { ref mode_pack } if !mode_pack.trim().is_empty() => Ok(()),
        _ => bad("strategy must be round_robin, fallback_chain, lowest_latency, power_of_two_choices, scored or fusion"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::ConnectionId;

    fn combo(id: &str, targets: &[&str]) -> Combo {
        Combo {
            id: id.into(),
            match_patterns: vec![],
            strategy: StrategyKind::RoundRobin,
            targets: targets.iter().map(|t| ConnectionId((*t).into())).collect(),
            model: Some("claude-sonnet-4-5".into()),
            compression: None,
            cache: None,
            budget: None,
            mode_pack: None,
        }
    }

    fn service(dir: &std::path::Path) -> (ComboService, Arc<Router>) {
        let router = Arc::new(Router::new(vec![]));
        let svc = ComboService::open(
            ComboStore::new(dir.join("combos.json")),
            Arc::new(ComboResolver::new(vec![])),
            Some(Arc::clone(&router)),
        )
        .unwrap();
        (svc, router)
    }

    // An edit must reach the router (live traffic) and the disk (restarts).
    #[test]
    fn writes_apply_to_the_router_and_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, router) = service(dir.path());
        svc.create(combo("fast", &["c1"])).unwrap();
        assert!(router.routes().iter().any(|r| r.match_models == ["fast"]));

        let (reopened, router2) = service(dir.path());
        assert_eq!(reopened.list().len(), 1);
        assert!(router2.routes().iter().any(|r| r.match_models == ["fast"]));

        reopened.delete("fast").unwrap();
        assert!(router2.routes().is_empty());
        assert_eq!(reopened.delete("fast"), Err(ComboError::NotFound));
    }

    #[test]
    fn invalid_combos_are_refused_before_anything_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(dir.path());
        let mut no_model = combo("a", &["c1"]);
        no_model.model = None;
        let mut chain = combo("b", &["c1"]);
        chain.strategy = StrategyKind::PromptChain { steps: vec![] };
        for bad in [combo("x", &[]), combo("x*", &["c1"]), no_model, chain] {
            assert!(matches!(svc.create(bad), Err(ComboError::Invalid(_))));
        }
        assert!(svc.list().is_empty());
        assert!(!dir.path().join("combos.json").exists());
        svc.create(combo("a", &["c1"])).unwrap();
        assert_eq!(
            svc.create(combo("a", &["c2"])).err(),
            Some(ComboError::Exists)
        );
    }

    // Two admins saving at once must both land; a lost update is silent.
    #[test]
    fn concurrent_creates_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(dir.path());
        let svc = Arc::new(svc);
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let svc = Arc::clone(&svc);
                std::thread::spawn(move || svc.create(combo(&format!("c{i}"), &["t"])).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(svc.list().len(), 16);
        assert_eq!(
            ComboStore::new(dir.path().join("combos.json"))
                .load()
                .unwrap()
                .len(),
            16
        );
    }

    // A console edit sends routing fields only; it must not wipe policies.
    #[test]
    fn update_keeps_policies_it_does_not_carry() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = service(dir.path());
        let mut seeded = combo("c", &["t1"]);
        seeded.compression = Some(crate::CompressionPolicy {
            plugin_id: "caveman".into(),
            auto_trigger_tokens: Some(100),
        });
        seeded.budget = Some(crate::BudgetPolicy {
            max_cost_microdollars: Some(5),
            overflow: "strict".into(),
        });
        svc.create(seeded).unwrap();
        let got = svc.update("c", combo("ignored", &["t2"])).unwrap();
        assert_eq!(got.id, "c");
        assert_eq!(got.targets[0].0, "t2");
        assert!(got.compression.is_some());
        assert_eq!(got.budget.and_then(|b| b.max_cost_microdollars), Some(5));
    }
}
