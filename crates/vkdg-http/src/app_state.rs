use std::sync::Arc;

use vkdg_admin::handlers::requests::RequestLog;
use vkdg_connections::{
    ConnectionCatalog, CredentialManager, LatencyTracker, QuotaTracker, SessionRegistry,
};
use vkdg_observe::DecisionRecordExporter;
use vkdg_routing::Router as VkdgRouter;

use crate::admission::{AdmissionGuard, IpPolicy};
use crate::dedup::DedupTable;
use crate::frontdoor::{FrontDoor, ServerConfig};
use crate::upstream::HttpClient;
use vkdg_cache::CacheBackend;
use vkdg_combos::ComboResolver;
use vkdg_memory::MemoryStore;
use vkdg_policy_compress::Compressor;
use vkdg_provider_sdk::ProviderRegistry;

/// Shared state for the full request pipeline.  Constructed by the binary and
/// injected into AppState; optional so unit tests that only exercise admission
/// or routing can omit it.
pub struct PipelineState {
    pub admission: Arc<AdmissionGuard>,
    pub router: Arc<VkdgRouter>,
    pub catalog: Arc<ConnectionCatalog>,
    pub credentials: Arc<CredentialManager>,
    pub http_client: Arc<HttpClient>,
    pub exporter: Arc<DecisionRecordExporter>,
    pub provider_registry: Arc<ProviderRegistry>,
    /// Optional cache backend.  None = cache disabled for this pipeline.
    pub cache: Option<Arc<dyn CacheBackend>>,
    /// Optional combo resolver.  None = no combo expansion for this pipeline.
    pub combo_resolver: Option<Arc<ComboResolver>>,
    /// Optional compressor.  None = compression disabled for this pipeline.
    pub compressor: Option<Arc<dyn Compressor>>,
    /// Optional request deduplication table.  None = dedup disabled.
    pub dedup_table: Option<Arc<DedupTable>>,
    /// Optional session affinity registry.  None = no session stickiness.
    pub session_registry: Option<Arc<SessionRegistry>>,
    /// Optional quota tracker.  None = no quota tracking.
    pub quota_tracker: Option<Arc<QuotaTracker>>,
    /// Optional latency tracker.  None = no latency recording.
    pub latency_tracker: Option<Arc<LatencyTracker>>,
    /// Global system prompt prepended to every conversation request.
    /// None = no injection.
    pub global_system_prompt: Option<String>,
    /// Optional memory store for conversational memory injection/extraction.
    /// None = memory disabled for this pipeline.
    pub memory_store: Option<Arc<MemoryStore>>,
    /// Enable quality scoring via vkdg-eval after each complete response.
    pub eval_enabled: bool,
    /// Enable context-relay: when a session pin rotates to a different connection,
    /// inject the session's recent conversation history as a system context block.
    pub relay_enabled: bool,
    pub ip_policy: Option<Arc<IpPolicy>>,
    /// Optional admin request log.  None = request logging disabled.
    pub request_log: Option<Arc<RequestLog>>,
}

impl PipelineState {
    /// Construct a PipelineState with all optional fields set to None/false.
    /// Use in tests and partial pipelines; override optional fields by name afterwards.
    pub fn minimal(
        admission: Arc<AdmissionGuard>,
        router: Arc<VkdgRouter>,
        catalog: Arc<ConnectionCatalog>,
        credentials: Arc<CredentialManager>,
        http_client: Arc<HttpClient>,
        exporter: Arc<DecisionRecordExporter>,
        provider_registry: Arc<ProviderRegistry>,
    ) -> Self {
        Self {
            admission,
            router,
            catalog,
            credentials,
            http_client,
            exporter,
            provider_registry,
            cache: None,
            combo_resolver: None,
            compressor: None,
            dedup_table: None,
            session_registry: None,
            quota_tracker: None,
            global_system_prompt: None,
            ip_policy: None,
            latency_tracker: None,
            memory_store: None,
            eval_enabled: false,
            relay_enabled: false,
            request_log: None,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub front_door: Arc<FrontDoor>,
    pub pipeline: Option<Arc<PipelineState>>,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            front_door: Arc::new(FrontDoor::new(config)),
            pipeline: None,
        }
    }

    pub fn with_pipeline(mut self, pipeline: Arc<PipelineState>) -> Self {
        self.pipeline = Some(pipeline);
        self
    }
}
