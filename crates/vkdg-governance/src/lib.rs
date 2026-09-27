//! Data-plane API keys for VKDG.
//!
//! A [`VirtualKey`] authenticates a client on `/v1/*`, independent of the
//! upstream provider credential. Keys are persisted by [`VirtualKeyStore`]
//! (SQLite, `0600`); only a SHA-256 hash of each token is stored.

pub mod key;
pub mod store;

pub use key::{hash_token, KeyScope, VirtualKey, VirtualKeyId, TOKEN_PREFIX};
pub use store::{KeyStoreError, VirtualKeyStore, DEFAULT_CACHE_TTL};
