//! Session affinity registry.
//!
//! Maps authenticated session IDs to the last-good connection used.
//! Avoids switching connections mid-conversation: each turn in a
//! multi-turn session prefers the same account it used before.
//!
//! Pins have TTL and are invalidated when the connection becomes unhealthy.
//! Phase E: persist across restarts (SQLite).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use vkdg_core::ConnectionId;

#[derive(Debug, Clone)]
pub struct SessionPin {
    pub connection_id: ConnectionId,
    pub pinned_at: DateTime<Utc>,
    pub ttl_secs: u64,
}

impl SessionPin {
    pub fn is_expired(&self) -> bool {
        let age = Utc::now().signed_duration_since(self.pinned_at);
        age.num_seconds() as u64 > self.ttl_secs
    }
}

pub struct SessionRegistry {
    pins: RwLock<HashMap<String, SessionPin>>,
    default_ttl_secs: u64,
}

impl SessionRegistry {
    pub fn new(default_ttl_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            pins: RwLock::new(HashMap::new()),
            default_ttl_secs,
        })
    }

    /// Get the pinned connection for a session, if any and not expired.
    pub async fn get(&self, session_id: &str) -> Option<ConnectionId> {
        let pins = self.pins.read().await;
        let pin = pins.get(session_id)?;
        if pin.is_expired() { None } else { Some(pin.connection_id.clone()) }
    }

    /// Pin a session to a connection after a successful response.
    pub async fn pin(&self, session_id: String, connection_id: ConnectionId) {
        let mut pins = self.pins.write().await;
        pins.insert(session_id, SessionPin {
            connection_id,
            pinned_at: Utc::now(),
            ttl_secs: self.default_ttl_secs,
        });
    }

    /// Invalidate a session pin (e.g., when its connection goes unhealthy).
    pub async fn invalidate(&self, session_id: &str) {
        self.pins.write().await.remove(session_id);
    }

    /// Purge expired pins. Call periodically.
    pub async fn purge_expired(&self) -> usize {
        let mut pins = self.pins.write().await;
        let before = pins.len();
        pins.retain(|_, pin| !pin.is_expired());
        before - pins.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: get returns pin even after TTL expires
    #[tokio::test]
    async fn expired_pin_not_returned() {
        let r = SessionRegistry::new(1);
        {
            let mut pins = r.pins.write().await;
            pins.insert("sess-x".into(), SessionPin {
                connection_id: ConnectionId("conn-a".into()),
                pinned_at: Utc::now() - chrono::Duration::seconds(2),
                ttl_secs: 1,
            });
        }
        assert!(r.get("sess-x").await.is_none(), "expired pin must not be returned");
    }

    // Plausible wrong impl: valid pin not returned
    #[tokio::test]
    async fn valid_pin_returned() {
        let r = SessionRegistry::new(3600);
        r.pin("sess-1".into(), ConnectionId("conn-a".into())).await;
        assert_eq!(r.get("sess-1").await, Some(ConnectionId("conn-a".into())));
    }

    // Plausible wrong impl: invalidate doesn't remove pin
    #[tokio::test]
    async fn invalidate_removes_pin() {
        let r = SessionRegistry::new(3600);
        r.pin("sess-1".into(), ConnectionId("conn-a".into())).await;
        r.invalidate("sess-1").await;
        assert!(r.get("sess-1").await.is_none());
    }

    // Plausible wrong impl: purge removes non-expired pins
    #[tokio::test]
    async fn purge_only_removes_expired() {
        let r = SessionRegistry::new(3600);
        r.pin("valid".into(), ConnectionId("c".into())).await;
        {
            let mut pins = r.pins.write().await;
            pins.insert("expired".into(), SessionPin {
                connection_id: ConnectionId("c".into()),
                pinned_at: Utc::now() - chrono::Duration::seconds(7200),
                ttl_secs: 3600,
            });
        }
        let removed = r.purge_expired().await;
        assert_eq!(removed, 1);
        assert!(r.get("valid").await.is_some());
    }
}
