pub mod admission;
pub mod app_state;
pub mod frontdoor;
pub mod pipeline;
pub mod provider;
pub mod server;
pub mod sse;
pub mod upstream;
pub mod dedup;
pub mod external_service;

// Re-exports for stable public API
pub use admission::{AdmissionGuard, IpPolicy};
pub use app_state::{AppState, PipelineState};
pub use frontdoor::{FrontDoor, ServerConfig, extract_client_ip, extract_vkdg_overrides};
pub use server::{serve, mcp_discovery};
pub use dedup::DedupTable;
pub use upstream::HttpClient;
