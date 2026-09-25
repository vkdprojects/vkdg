use serde::{Deserialize, Serialize};

/// Top-level gateway configuration (parsed from YAML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Listening address, e.g. `"0.0.0.0:8080"`.
    pub listen: String,
    pub connections: Vec<ConnectionDef>,
    pub routes: Vec<RouteDef>,
    pub limits: Option<LimitsDef>,
    pub observe: Option<ObserveDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDef {
    pub id: String,
    /// `"anthropic"` | `"openai"` | `"google"` | `"custom:<url>"`
    pub provider: String,
    pub auth: AuthDef,
    pub models: Vec<String>,
    pub max_concurrent: Option<u32>,
    pub weight: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthDef {
    ApiKey { env_var: String },
    OAuth2 {
        token_url: String,
        client_id: String,
        client_secret_env: String,
        scopes: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDef {
    pub id: String,
    pub match_models: Vec<String>,
    /// `"round_robin"` | `"weighted"` | `"fallback_chain"` | `"lowest_latency"`
    /// | `"power_of_two_choices"` | `"last_known_good"`
    pub strategy: String,
    /// Connection ids.
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsDef {
    pub max_concurrent_requests: Option<usize>,
    pub max_body_bytes: Option<u64>,
    pub request_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveDef {
    pub otlp_endpoint: Option<String>,
    pub log_level: Option<String>,
    /// `"pretty"` | `"json"`
    pub log_format: Option<String>,
}
