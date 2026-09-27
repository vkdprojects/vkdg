pub mod admission;
pub mod app_state;
pub mod auth;
pub mod dedup;
pub mod external_service;
pub mod frontdoor;
pub mod hooks;
pub mod metering;
pub mod pipeline;
pub mod provider;
pub mod server;
pub mod sse;
pub mod upstream;

// Re-exports for stable public API
pub use admission::AdmissionGuard;
pub use app_state::{AppState, PipelineState};
pub use auth::{require_api_key, ClientIdentity, DataAuth};
pub use dedup::DedupTable;
pub use frontdoor::{extract_vkdg_overrides, resolve_client_ip, FrontDoor, ServerConfig};
pub use server::{mcp_discovery, serve};
pub use sse::with_termination_guard;
pub use upstream::HttpClient;
