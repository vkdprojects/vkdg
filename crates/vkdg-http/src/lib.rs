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
pub use admission::AdmissionGuard;
pub use app_state::{AppState, PipelineState};
pub use frontdoor::{FrontDoor, ServerConfig};
pub use server::{build_router, serve};
pub use dedup::DedupTable;
pub use upstream::HttpClient;
