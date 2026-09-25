use std::sync::Arc;

use vkdg_connections::{ConnectionCatalog, CredentialManager};
use vkdg_observe::DecisionRecordExporter;
use vkdg_routing::Router as VkdgRouter;

use crate::admission::AdmissionGuard;
use crate::frontdoor::{FrontDoor, ServerConfig};
use crate::provider::ProviderAdapter;
use crate::upstream::HttpClient;

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
    pub provider_adapter: Arc<dyn ProviderAdapter>,
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
