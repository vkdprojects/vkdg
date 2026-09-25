use std::collections::HashSet;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Newtypes ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub String);

// ── State machine ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptState {
    Received,
    Authenticated,
    Admitted,
    AccountReserved,
    CredentialReady,
    Prepared,
    UpstreamOpen,
    Committed,
    Completed,
    Partial,
    Cancelled,
    Failed,
}

// ── API type ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiType {
    VkdgNative,
    AnthropicMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
    OpenAiImages,
}

// ── Request envelope ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub request_id: RequestId,
    pub client_id: ClientId,
    pub tenant_id: TenantId,
    pub session_key: Option<SessionKey>,
    pub api_type: ApiType,
    pub model_requested: String,
    pub deadline: Option<DateTime<Utc>>,
}

// ── Decision record ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcludedCandidate {
    pub connection_id: ConnectionId,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttemptResult {
    Completed,
    Partial,
    Cancelled,
    Failed {
        code: String,
        phase: String,
        retryable: bool,
        committed: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub request_id: RequestId,
    pub config_version: u64,
    pub client_id: ClientId,
    pub route_id: String,
    pub connection_chosen: Option<ConnectionId>,
    pub candidates_excluded: Vec<ExcludedCandidate>,
    pub attempt_count: u32,
    pub state_transitions: Vec<(AttemptState, DateTime<Utc>)>,
    pub result: AttemptResult,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum VkdgError {
    #[error("unauthenticated")]
    Unauthenticated,

    #[error("unauthorized")]
    Unauthorized,

    #[error("admission rejected: {reason}")]
    AdmissionRejected { reason: String },

    #[error("capability unsupported: {capability}")]
    CapabilityUnsupported { capability: String },

    #[error("no eligible connection")]
    NoEligibleConnection,

    #[error("upstream error {code}: {message}")]
    UpstreamError { code: u16, message: String },

    #[error("plugin error ({plugin_id}): {message}")]
    PluginError { plugin_id: String, message: String },

    #[error("config invalid: {field}: {message}")]
    ConfigInvalid { field: String, message: String },

    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T, E = VkdgError> = std::result::Result<T, E>;

// ── Capabilities ──────────────────────────────────────────────────────────────

/// A declared capability of a provider connection or required by an operation.
/// Lives in core so both connections (who advertise capabilities) and operations
/// (who require them) can reference the same type without a circular dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    Tools,
    Reasoning,
    Vision,
    JsonSchema,
    Streaming,
    Embedding,
    ImageInput,
    AudioInput,
    VideoInput,
}

/// Set of capabilities. Default is empty (no capabilities declared).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitySet(pub HashSet<Capability>);

impl CapabilitySet {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    pub fn contains(&self, cap: &Capability) -> bool {
        self.0.contains(cap)
    }

    pub fn insert(&mut self, cap: Capability) {
        self.0.insert(cap);
    }

    pub fn is_superset_of(&self, required: &CapabilitySet) -> bool {
        required.0.iter().all(|cap| self.0.contains(cap))
    }
}

pub mod pipeline;
