//! Virtual key management and budget enforcement for VKDG.
//!
//! # Overview
//!
//! Virtual keys decouple client authentication from upstream provider credentials.
//! Each virtual key carries:
//! - An identity (tenant + optional user)
//! - A monthly budget cap in microdollars
//! - A spent counter (atomically incremented per request)
//! - Optional scopes (data:inference, data:image, etc.)
//!
//! # Hot path
//!
//! Budget check is a single atomic load + compare. No lock, no DB call on the
//! hot path. The key store is an in-memory HashMap loaded at startup.
//! Phase E: SQLite persistence + background sync.

pub mod budget;
pub mod key;
pub mod store;

pub use budget::BudgetChecker;
pub use key::{KeyScope, VirtualKey, VirtualKeyId};
pub use store::VirtualKeyStore;
