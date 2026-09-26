//! Provider connection management for VKDG.
//!
//! Central crate for upstream connection state, credential lifecycle,
//! and routing eligibility. The catalog is the source of truth for
//! which connections are available to the pipeline.

pub mod catalog;
pub mod connection;
pub mod credentials;
pub mod latency;
pub mod quota;
pub mod session;

pub use catalog::ConnectionCatalog;
pub use connection::{
    AuthKind, Connection, ConnectionConfig, ConnectionGuard, ConnectionState, ProviderKind,
    TokenState,
};
pub use credentials::CredentialManager;
pub use latency::{EwmaLatency, LatencyTracker};
pub use quota::{QuotaTracker, QuotaWindow};
pub use session::{SessionPin, SessionRegistry};
