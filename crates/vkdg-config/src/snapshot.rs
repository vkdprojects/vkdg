use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::watch;
use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
use vkdg_core::{CapabilitySet, ConnectionId};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, StrategyKind};

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
        }));
        let observe = Arc::new(cfg.observe.clone().unwrap_or(ObserveDef {
            otlp_endpoint: None,
            log_level: None,
            log_format: None,
        }));

        Ok(Self {
            version,
            gateway: Arc::new(cfg),
            connections: Arc::new(connections),
            routes: Arc::new(routes),
            limits,
            observe,
        })
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

        let provider = parse_provider(&def.provider);
        let auth = match &def.auth {
            crate::schema::AuthDef::ApiKey { env_var } => {
                AuthKind::ApiKey { env_var: env_var.clone() }
            }
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
        };

        out.push(ConnectionConfig {
            id: ConnectionId(def.id.clone()),
            provider,
            auth,
            models: def.models.clone(),
            max_concurrent: def.max_concurrent.unwrap_or(100),
            weight: def.weight.unwrap_or(1),
            tags: vec![],
            capabilities: CapabilitySet::default(),
        });
    }

    Ok(out)
}

fn parse_provider(s: &str) -> ProviderKind {
    match s {
        "anthropic" => ProviderKind::Anthropic,
        "openai" => ProviderKind::OpenAI,
        "google" => ProviderKind::Google,
        other => {
            // "custom:<url>" or bare URL → Custom
            let url = other.strip_prefix("custom:").unwrap_or(other);
            ProviderKind::Custom { base_url: url.to_owned() }
        }
    }
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

        let strategy = parse_strategy(&def.strategy).ok_or_else(|| {
            ConfigError::UnknownStrategy {
                strategy: def.strategy.clone(),
                route: def.id.clone(),
            }
        })?;

        out.push(RouteConfig {
            id: RouteId(def.id.clone()),
            match_models: def.match_models.clone(),
            strategy,
            targets: def.targets.iter().map(|t| ConnectionId(t.clone())).collect(),
            plugin_hooks: PluginHooks::default(),
        });
    }

    Ok(out)
}

fn parse_strategy(s: &str) -> Option<StrategyKind> {
    match s {
        "round_robin" => Some(StrategyKind::RoundRobin),
        "weighted" => Some(StrategyKind::Weighted),
        "lowest_latency" => Some(StrategyKind::LowestLatency),
        "power_of_two_choices" => Some(StrategyKind::PowerOfTwoChoices),
        "fallback_chain" => Some(StrategyKind::FallbackChain),
        "last_known_good" => Some(StrategyKind::LastKnownGood),
        _ => None,
    }
}
