//! Conversational memory system for VKDG.
//!
//! Provides extraction, storage, and injection of memory facts across sessions.
//! Three memory operations:
//! - Extract: pull facts from a completed conversation turn
//! - Store: persist extracted facts with TTL and tenant scoping
//! - Inject: prepend relevant facts as system context on future turns
//!
//! Phase D: in-memory store, exact keyword retrieval.
//! Phase E: vector embeddings for semantic retrieval, SQLite persistence.

pub mod extract;
pub mod inject;
pub mod store;

pub use extract::extract_facts;
pub use inject::inject_memories;
pub use store::{MemoryRecord, MemoryStore};
