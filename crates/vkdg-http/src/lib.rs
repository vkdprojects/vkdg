pub mod admission;
pub mod app_state;
pub mod dedup;
pub mod external_service;
pub mod frontdoor;
pub mod pipeline;
pub mod provider;
pub mod server;
pub mod sse;
pub mod upstream;

// Re-exports for stable public API
pub use admission::{AdmissionGuard, IpPolicy};
pub use app_state::{AppState, PipelineState};
pub use dedup::DedupTable;
pub use frontdoor::{extract_client_ip, extract_vkdg_overrides, FrontDoor, ServerConfig};
pub use server::{mcp_discovery, serve};
pub use upstream::HttpClient;
