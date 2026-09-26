use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque virtual key identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualKeyId(pub String);

/// Scopes a virtual key is allowed to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyScope {
    /// Full inference access.
    DataInference,
    /// Image generation only.
    DataImage,
    /// Read-only admin access.
    AdminRead,
    /// Full admin access.
    AdminWrite,
}

fn default_atomic_u64() -> AtomicU64 {
    AtomicU64::new(0)
}

fn default_atomic_bool() -> AtomicBool {
    AtomicBool::new(false)
}

/// A virtual key entry.
///
/// The raw token is never stored — only its SHA-256 hash.
/// The raw token is shown once at creation time.
///
/// `spent_microdollars` and `revoked` are runtime-only atomic fields;
/// they are skipped during serialization (Phase E will use a separate
/// persistence record).
#[derive(Debug, Serialize, Deserialize)]
pub struct VirtualKey {
    pub id: VirtualKeyId,
    pub name: String,
    pub tenant_id: String,
    /// SHA-256 hex of the raw bearer token.
    pub token_hash: String,
    pub scopes: Vec<KeyScope>,
    /// Monthly budget in microdollars. None = unlimited.
    pub budget_microdollars: Option<u64>,
    /// Microdollars spent this budget window.
    /// Atomically incremented; reset each `budget_reset_at`.
    #[serde(skip, default = "default_atomic_u64")]
    pub spent_microdollars: AtomicU64,
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Whether the key has been revoked.
    #[serde(skip, default = "default_atomic_bool")]
    pub revoked: AtomicBool,
}

impl VirtualKey {
    /// Create a new key. Returns `(VirtualKey, raw_token)`.
    ///
    /// `raw_token` is shown once; the key stores only `token_hash`.
    pub fn new(
        name: String,
        tenant_id: String,
        scopes: Vec<KeyScope>,
        budget_microdollars: Option<u64>,
    ) -> (Self, String) {
        use sha2::{Digest, Sha256};

        let raw = Uuid::new_v4().to_string();
        let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
        let key = Self {
            id: VirtualKeyId(Uuid::new_v4().to_string()),
            name,
            tenant_id,
            token_hash: hash,
            scopes,
            budget_microdollars,
            spent_microdollars: AtomicU64::new(0),
            budget_reset_at: Some(Utc::now() + chrono::Duration::days(30)),
            created_at: Utc::now(),
            revoked: AtomicBool::new(false),
        };
        (key, raw)
    }

    /// Check if this key's token matches the given raw token.
    pub fn matches_token(&self, raw: &str) -> bool {
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
        hash == self.token_hash
    }

    /// Remaining budget in microdollars. `None` if unlimited.
    pub fn remaining_budget(&self) -> Option<u64> {
        let limit = self.budget_microdollars?;
        let spent = self.spent_microdollars.load(Ordering::Relaxed);
        Some(limit.saturating_sub(spent))
    }

    /// Atomically record spend. Returns `Err(remaining)` if budget would be exceeded.
    pub fn record_spend(&self, microdollars: u64) -> Result<(), u64> {
        if let Some(limit) = self.budget_microdollars {
            // Optimistic add: undo if over-limit.
            let prev = self
                .spent_microdollars
                .fetch_add(microdollars, Ordering::Relaxed);
            if prev + microdollars > limit {
                self.spent_microdollars
                    .fetch_sub(microdollars, Ordering::Relaxed);
                return Err(limit.saturating_sub(prev));
            }
        }
        Ok(())
    }

    /// Whether this key is currently revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plausible wrong impl: `record_spend` does not prevent over-spend.
    #[test]
    fn record_spend_rejects_over_budget() {
        let (key, _) = VirtualKey::new("test".into(), "t".into(), vec![], Some(1000));
        key.record_spend(800).unwrap();
        let result = key.record_spend(300); // 800 + 300 = 1100 > 1000
        assert!(result.is_err(), "over-budget spend must be rejected");
    }

    /// Plausible wrong impl: `matches_token` returns true for wrong token.
    #[test]
    fn matches_token_rejects_wrong_token() {
        let (key, raw) = VirtualKey::new("test".into(), "t".into(), vec![], None);
        assert!(key.matches_token(&raw));
        assert!(!key.matches_token("wrong-token"));
    }

    /// Plausible wrong impl: `remaining_budget` returns 0 for unlimited key.
    #[test]
    fn unlimited_key_has_none_budget() {
        let (key, _) = VirtualKey::new("test".into(), "t".into(), vec![], None);
        assert!(
            key.remaining_budget().is_none(),
            "unlimited key must return None budget"
        );
    }
}
