use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::watch;
use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
use vkdg_core::net::{parse_ip_list, IpRules};
use vkdg_core::{CapabilitySet, ConnectionId};
use vkdg_routing::{RouteConfig, RouteId, StrategyKind};

use crate::error::ConfigError;
use crate::schema::{GatewayConfig, LimitsDef, ObserveDef};

/// Immutable validated configuration snapshot.
/// Version monotonically increases with each reload.
/// Cloning is cheap — all heap data is Arc-wrapped.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub version: u64,
    pub gateway: Arc<GatewayConfig>,
    pub connections: Arc<Vec<ConnectionConfig>>,
    pub routes: Arc<Vec<RouteConfig>>,
    pub limits: Arc<LimitsDef>,
    /// `limits.ip_allowlist`/`ip_blocklist`, parsed once at load.
    pub ip_rules: Arc<IpRules>,
    pub observe: Arc<ObserveDef>,
}

impl ConfigSnapshot {
    /// Build from a validated [`GatewayConfig`] at the given version.
    ///
    /// Converts the YAML schema types into the runtime connection/route types
    /// and validates all cross-references.
    pub fn build(version: u64, cfg: GatewayConfig) -> Result<Self, ConfigError> {
        let connections = build_connections(&cfg)?;
        let routes = build_routes(&cfg, &connections)?;

        let limits = Arc::new(cfg.limits.clone().unwrap_or(LimitsDef {
            max_concurrent_requests: None,
            max_body_bytes: None,
            request_timeout_secs: None,
            ip_allowlist: vec![],
            ip_blocklist: vec![],
        }));
        let observe = Arc::new(cfg.observe.clone().unwrap_or(ObserveDef {
            otlp_endpoint: None,
            log_level: None,
            log_format: None,
        }));
        let ip_rules = Arc::new(IpRules {
            allow: parse_ip_list("limits.ip_allowlist", &limits.ip_allowlist)
                .map_err(ConfigError::Validation)?,
            block: parse_ip_list("limits.ip_blocklist", &limits.ip_blocklist)
                .map_err(ConfigError::Validation)?,
        });

        Ok(Self {
            version,
            gateway: Arc::new(cfg),
            connections: Arc::new(connections),
            routes: Arc::new(routes),
            limits,
            ip_rules,
            observe,
        })
    }

    /// Returns an empty snapshot with version 0, suitable for testing and
    /// admin API bootstrap before a config file is loaded.
    pub fn default_empty() -> Self {
        let gateway = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![],
            routes: vec![],
            limits: None,
            observe: None,
            global_system_prompt: None,
        };
        Self {
            version: 0,
            gateway: Arc::new(gateway),
            connections: Arc::new(vec![]),
            routes: Arc::new(vec![]),
            limits: Arc::new(LimitsDef {
                max_concurrent_requests: None,
                max_body_bytes: None,
                request_timeout_secs: None,
                ip_allowlist: vec![],
                ip_blocklist: vec![],
            }),
            ip_rules: Arc::new(IpRules::default()),
            observe: Arc::new(ObserveDef {
                otlp_endpoint: None,
                log_level: None,
                log_format: None,
            }),
        }
    }
}
/// A sender/receiver pair for atomic config swaps.
/// Receivers hold a strong Arc reference so the old snapshot stays alive
/// until every in-flight request using it completes.
pub type ConfigTx = watch::Sender<Arc<ConfigSnapshot>>;
pub type ConfigRx = watch::Receiver<Arc<ConfigSnapshot>>;

pub fn config_channel(initial: ConfigSnapshot) -> (ConfigTx, ConfigRx) {
    watch::channel(Arc::new(initial))
}

// ── Conversion helpers ────────────────────────────────────────────────────────

fn build_connections(cfg: &GatewayConfig) -> Result<Vec<ConnectionConfig>, ConfigError> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(cfg.connections.len());

    for def in &cfg.connections {
        if !seen.insert(def.id.clone()) {
            return Err(ConfigError::DuplicateConnection { id: def.id.clone() });
        }

        let provider = parse_provider(&def.id, &def.provider, def.base_url.as_deref())?;
        let auth = match &def.auth {
            crate::schema::AuthDef::ApiKey { env_var } => AuthKind::ApiKey {
                env_var: env_var.clone(),
            },
            crate::schema::AuthDef::OAuth2 {
                token_url,
                client_id,
                client_secret_env,
                scopes,
            } => AuthKind::OAuth2 {
                token_url: token_url.clone(),
                client_id: client_id.clone(),
                client_secret_env: client_secret_env.clone(),
                scopes: scopes.clone(),
            },
            crate::schema::AuthDef::Account { account } => {
                if account.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "connection '{}': auth.account must not be empty",
                        def.id
                    )));
                }
                if matches!(
                    provider,
                    ProviderKind::Custom { .. } | ProviderKind::AnthropicCompat { .. }
                ) {
                    return Err(ConfigError::Validation(format!(
                        "connection '{}': account auth needs a provider plugin id (kiro, codex, ...), \
                         not a compatible endpoint: an account's token belongs to its own provider",
                        def.id
                    )));
                }
                AuthKind::Account {
                    account_id: account.clone(),
                }
            }
        };

        let endpoint = parse_endpoint(&def.id, &def.provider, def.endpoint.as_deref())?;

        out.push(ConnectionConfig {
            id: ConnectionId(def.id.clone()),
            provider,
            auth,
            models: def.models.clone(),
            max_concurrent: def.max_concurrent.unwrap_or(100),
            weight: def.weight.unwrap_or(1),
            tags: def.tags.clone(),
            endpoint,
            capabilities: CapabilitySet::default(),
        });
    }

    Ok(out)
}

/// Endpoints a provider exposes, when it has more than one. A name outside the
/// list is refused rather than ignored: silently falling back to the default
/// plane would hide a typo behind traffic that still works, but on one bucket.
fn provider_endpoints(provider: &str) -> &'static [&'static str] {
    match provider {
        "kiro" => &["runtime", "codewhisperer"],
        _ => &[],
    }
}

fn parse_endpoint(
    conn: &str,
    provider: &str,
    endpoint: Option<&str>,
) -> Result<Option<String>, ConfigError> {
    let Some(name) = endpoint.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let known = provider_endpoints(provider);
    if known.is_empty() {
        return Err(ConfigError::Validation(format!(
            "connection '{conn}': provider {provider} has a single endpoint, so `endpoint` does not apply"
        )));
    }
    if !known.contains(&name) {
        return Err(ConfigError::Validation(format!(
            "connection '{conn}': unknown endpoint {name:?} for {provider}; known: {}",
            known.join(", ")
        )));
    }
    Ok(Some(name.to_owned()))
}

/// Resolve `provider` (and `base_url`) to a provider kind. `base_url` is only
/// meaningful for the two compatible kinds; anywhere else it would be silently
/// ignored and the connection would call the provider's default host.
fn parse_provider(
    conn: &str,
    provider: &str,
    base_url: Option<&str>,
) -> Result<ProviderKind, ConfigError> {
    let invalid = |m: String| ConfigError::Validation(format!("connection '{conn}': {m}"));
    let endpoint = || match base_url.map(str::trim) {
        Some(u) if u.starts_with("http://") || u.starts_with("https://") => {
            Ok(u.trim_end_matches('/').to_owned())
        }
        Some(u) => Err(invalid(format!(
            "base_url must start with http:// or https://, got {u:?}"
        ))),
        None => Err(invalid(format!(
            "provider {provider} needs base_url, e.g. http://localhost:11434"
        ))),
    };
    let kind = match provider {
        "openai-compat" => {
            return Ok(ProviderKind::Custom {
                base_url: endpoint()?,
            })
        }
        "anthropic-compat" => {
            return Ok(ProviderKind::AnthropicCompat {
                base_url: endpoint()?,
            })
        }
        "anthropic" => ProviderKind::Anthropic,
        "openai" => ProviderKind::OpenAI,
        "google" => ProviderKind::Google,
        // "custom:<url>" or a bare URL → OpenAI-compatible endpoint at that URL.
        other if other.starts_with("custom:") || other.starts_with("http") => {
            let url = other.strip_prefix("custom:").unwrap_or(other);
            ProviderKind::Custom {
                base_url: url.to_owned(),
            }
        }
        // Anything else names a provider plugin in the adapter registry.
        other => ProviderKind::Plugin {
            id: other.to_owned(),
        },
    };
    if base_url.is_some() {
        return Err(invalid(format!(
            "base_url is only used with openai-compat or anthropic-compat, not {provider}"
        )));
    }
    Ok(kind)
}

fn build_routes(
    cfg: &GatewayConfig,
    connections: &[ConnectionConfig],
) -> Result<Vec<RouteConfig>, ConfigError> {
    // Build a set of valid connection ids for fast lookup.
    let conn_ids: HashSet<&str> = connections.iter().map(|c| c.id.0.as_str()).collect();
    let mut out = Vec::with_capacity(cfg.routes.len());

    for def in &cfg.routes {
        // Validate every target references a known connection.
        for target in &def.targets {
            if !conn_ids.contains(target.as_str()) {
                return Err(ConfigError::UnknownConnection {
                    route: def.id.clone(),
                    connection: target.clone(),
                });
            }
        }

        let strategy =
            parse_strategy(&def.strategy).ok_or_else(|| ConfigError::UnknownStrategy {
                strategy: def.strategy.clone(),
                route: def.id.clone(),
            })?;

        out.push(RouteConfig {
            id: RouteId(def.id.clone()),
            match_models: def.match_models.clone(),
            strategy,
            targets: def
                .targets
                .iter()
                .map(|t| ConnectionId(t.clone()))
                .collect(),
            plugin_hooks: def.hooks.clone(),
        });
    }

    Ok(out)
}

fn parse_strategy(s: &str) -> Option<StrategyKind> {
    match s {
        "round_robin" => Some(StrategyKind::RoundRobin),
        "lowest_latency" => Some(StrategyKind::LowestLatency),
        "power_of_two_choices" => Some(StrategyKind::PowerOfTwoChoices),
        "fallback_chain" => Some(StrategyKind::FallbackChain),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_provider_id_resolves_to_its_own_adapter() {
        // A provider id that is not a core kind must address the plugin adapter
        // registered under that exact id — not fall back to "openai".
        for id in ["kiro", "groq", "deepseek", "github-copilot"] {
            assert_eq!(parse_provider("c", id, None).unwrap().adapter_id(), id);
        }
    }

    #[test]
    fn core_and_custom_provider_kinds_are_preserved() {
        assert_eq!(
            parse_provider("c", "anthropic", None).unwrap().adapter_id(),
            "anthropic"
        );
        assert_eq!(
            parse_provider("c", "openai", None).unwrap().adapter_id(),
            "openai"
        );
        assert_eq!(
            parse_provider("c", "google", None).unwrap().adapter_id(),
            "google"
        );

        // An explicit URL still means "OpenAI-compatible endpoint at this URL".
        let custom = parse_provider("c", "custom:https://api.example.com/v1", None).unwrap();
        assert_eq!(custom.adapter_id(), "openai");
        assert_eq!(custom.as_str(), "https://api.example.com/v1");
        assert_eq!(
            parse_provider("c", "https://api.example.com/v1", None)
                .unwrap()
                .adapter_id(),
            "openai"
        );
    }

    fn yaml_cfg(provider: &str, account: &str) -> GatewayConfig {
        serde_yaml::from_str(&format!(
            "listen: 0.0.0.0:8080\nconnections:\n  - id: c1\n    provider: \"{provider}\"\n    auth: {{ type: account, account: \"{account}\" }}\n    models: [\"m\"]\nroutes: []\nlimits: null\nobserve: null\n"
        ))
        .expect("yaml parses")
    }

    // Plausible wrong impl: `type: account` not accepted by serde, mapped to the wrong
    // AuthKind, or blank/custom-URL accounts accepted (fail later at request time).
    #[test]
    fn account_auth_maps_to_account_kind_and_is_validated() {
        let snap = ConfigSnapshot::build(1, yaml_cfg("kiro", "kiro-ab12cd34")).unwrap();
        match &snap.connections[0].auth {
            AuthKind::Account { account_id } => assert_eq!(account_id, "kiro-ab12cd34"),
            other => panic!("expected Account, got {other:?}"),
        }
        assert!(matches!(
            ConfigSnapshot::build(1, yaml_cfg("kiro", " ")),
            Err(ConfigError::Validation(_))
        ));
        assert!(matches!(
            ConfigSnapshot::build(1, yaml_cfg("custom:https://x", "a1")),
            Err(ConfigError::Validation(_))
        ));
    }

    fn load(yaml: &str) -> Result<ConfigSnapshot, ConfigError> {
        let cfg: GatewayConfig =
            serde_yaml::from_str(yaml).map_err(|e| ConfigError::Validation(e.to_string()))?;
        ConfigSnapshot::build(1, cfg)
    }

    const CONN: &str =
        "listen: 0.0.0.0:8080\nroutes: []\nlimits: null\nobserve: null\nconnections:\n";

    // The Tier-1 snippet from docs/sdk/adding-a-provider.md: it passed
    // `config check` while its base_url was silently dropped.
    #[test]
    fn compat_providers_use_their_base_url() {
        let snap = load(&format!(
            "{CONN}  - id: llama\n    provider: openai-compat\n    base_url: http://localhost:11434/\n    auth: {{ type: api_key, env_var: K }}\n    models: [\"llama3.3:70b\"]\n  - id: proxy\n    provider: anthropic-compat\n    base_url: https://proxy.example.com\n    auth: {{ type: api_key, env_var: K }}\n    models: [\"claude-*\"]\n"
        ))
        .unwrap();
        let (a, b) = (&snap.connections[0].provider, &snap.connections[1].provider);
        assert_eq!(
            (a.adapter_id(), a.as_str()),
            ("openai", "http://localhost:11434")
        );
        assert_eq!(
            (b.adapter_id(), b.as_str()),
            ("anthropic", "https://proxy.example.com")
        );
    }

    #[test]
    fn misplaced_or_missing_base_url_and_typos_are_errors() {
        let bad = [
            // compat without an endpoint
            "  - id: c\n    provider: openai-compat\n    auth: { type: api_key, env_var: K }\n    models: [m]\n",
            // base_url on a provider that would ignore it
            "  - id: c\n    provider: kiro\n    base_url: http://x\n    auth: { type: api_key, env_var: K }\n    models: [m]\n",
            // not a URL
            "  - id: c\n    provider: anthropic-compat\n    base_url: localhost:1\n    auth: { type: api_key, env_var: K }\n    models: [m]\n",
            // an account token belongs to its own provider, not an arbitrary endpoint
            "  - id: c\n    provider: anthropic-compat\n    base_url: http://x\n    auth: { type: account, account: a1 }\n    models: [m]\n",
            // typos in connection and auth fields
            "  - id: c\n    provider: openai-compat\n    base_ur: http://x\n    auth: { type: api_key, env_var: K }\n    models: [m]\n",
            "  - id: c\n    provider: openai\n    auth: { type: api_key, env_vr: K }\n    models: [m]\n",
        ];
        for conn in bad {
            assert!(load(&format!("{CONN}{conn}")).is_err(), "accepted:\n{conn}");
        }
    }

    // Refutes: pinning every Kiro connection to one host. The two planes keep
    // SEPARATE rate-limit buckets — measured: saturating runtime.kiro.dev to 75%
    // HTTP 429 left codewhisperer answering 80/80 at the same moment — so two
    // connections on the same account, one per plane, add real capacity. That is
    // only expressible if the plane is part of the connection config.
    #[test]
    fn a_connection_can_name_its_provider_endpoint() {
        let snap = load(&format!(
            "{CONN}  - id: k-rt\n    provider: kiro\n    endpoint: runtime\n    auth: {{ type: account, account: a1 }}\n    models: [\"claude-*\"]\n  - id: k-cw\n    provider: kiro\n    endpoint: codewhisperer\n    auth: {{ type: account, account: a1 }}\n    models: [\"claude-*\"]\n  - id: k-def\n    provider: kiro\n    auth: {{ type: account, account: a1 }}\n    models: [\"claude-*\"]\n"
        ))
        .unwrap();
        assert_eq!(snap.connections[0].endpoint.as_deref(), Some("runtime"));
        assert_eq!(
            snap.connections[1].endpoint.as_deref(),
            Some("codewhisperer")
        );
        assert_eq!(
            snap.connections[2].endpoint, None,
            "omitting it keeps the plugin's own default"
        );
    }

    // Refutes: accepting any string, so a typo silently sends traffic to the
    // default plane, or accepting the field on a provider that ignores it.
    #[test]
    fn an_unknown_or_misplaced_endpoint_is_an_error() {
        let bad = [
            "  - id: c\n    provider: kiro\n    endpoint: runtim\n    auth: { type: account, account: a1 }\n    models: [m]\n",
            "  - id: c\n    provider: anthropic\n    endpoint: runtime\n    auth: { type: api_key, env_var: K }\n    models: [m]\n",
        ];
        for conn in bad {
            assert!(load(&format!("{CONN}{conn}")).is_err(), "accepted:\n{conn}");
        }
    }

    // Docs and schema drifted once (`tags` was silently dropped); keep the
    // shipped example loadable.
    #[test]
    fn shipped_example_config_loads() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.example.yaml");
        let snap = load(&std::fs::read_to_string(path).unwrap()).expect("config.example.yaml");
        assert!(snap.connections.iter().any(|c| c.tags == ["primary"]));
    }

    // The Tier-1 snippet is read from the guide itself: if the doc changes to
    // something the schema refuses, this fails instead of a first-time user.
    #[test]
    fn tier1_snippet_in_the_provider_guide_loads() {
        let doc = include_str!("../../../docs/sdk/adding-a-provider.md");
        let tier1 = &doc[doc.find("## Tier 1").expect("Tier 1 section")..];
        let start = tier1.find("```yaml\n").expect("yaml block") + "```yaml\n".len();
        let block = &tier1[start..start + tier1[start..].find("```").expect("block end")];
        let snap =
            load(&format!("listen: 0.0.0.0:8080\nroutes: []\n{block}")).expect("doc snippet");
        let p = &snap.connections[0].provider;
        assert_eq!(
            (p.adapter_id(), p.as_str()),
            ("openai", "http://localhost:11434")
        );
    }
}
