//! Request deduplication layer.
//!
//! When N concurrent identical requests arrive (same model + messages hash),
//! only the first goes upstream. The others wait for the in-flight result
//! and receive the same response.
//!
//! This prevents thundering-herd on cache misses and reduces upstream load
//! during bursts of identical requests (common in eval pipelines and CI).
//!
//! Implementation: tokio::sync::Notify per in-flight key.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// A deduplication key for a request (e.g. SHA-256 of model + messages).
pub type DedupKey = String;

/// Tracks in-flight requests by key.
pub struct DedupTable {
    in_flight: Mutex<HashMap<DedupKey, Arc<Notify>>>,
}

impl DedupTable {
    pub fn new() -> Self {
        Self {
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// Try to register as the first caller for this key.
    ///
    /// Returns `(true, notify)` if this caller should proceed upstream.
    /// Returns `(false, notify)` if a duplicate is already in-flight; the
    /// caller should `notify.notified().await` then retry from cache.
    ///
    /// The first caller **must** call [`complete`](Self::complete) when done
    /// (success or failure) to wake waiting duplicates.
    pub fn register(&self, key: &DedupKey) -> (bool, Arc<Notify>) {
        let mut map = self.in_flight.lock();
        if let Some(n) = map.get(key) {
            (false, Arc::clone(n))
        } else {
            let n = Arc::new(Notify::new());
            map.insert(key.clone(), Arc::clone(&n));
            (true, Arc::clone(&n))
        }
    }

    /// Mark a key complete and wake all waiters.
    pub fn complete(&self, key: &DedupKey) {
        let mut map = self.in_flight.lock();
        if let Some(n) = map.remove(key) {
            n.notify_waiters();
        }
    }

    pub fn in_flight_count(&self) -> usize {
        self.in_flight.lock().len()
    }
}

impl Default for DedupTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: second register returns (true, _) instead of (false, _)
    #[test]
    fn second_registration_returns_false() {
        let t = DedupTable::new();
        let (first, _) = t.register(&"key1".to_string());
        let (second, _) = t.register(&"key1".to_string());
        assert!(first, "first caller must be flagged as first");
        assert!(!second, "second caller must be flagged as duplicate");
    }

    // Plausible wrong impl: complete() doesn't remove the key, count stays at 1
    #[test]
    fn complete_removes_key() {
        let t = DedupTable::new();
        t.register(&"key1".to_string());
        assert_eq!(t.in_flight_count(), 1);
        t.complete(&"key1".to_string());
        assert_eq!(t.in_flight_count(), 0);
    }

    // Plausible wrong impl: different keys treated as the same
    #[test]
    fn different_keys_are_independent() {
        let t = DedupTable::new();
        let (f1, _) = t.register(&"key1".to_string());
        let (f2, _) = t.register(&"key2".to_string());
        assert!(f1 && f2, "different keys must both be first");
    }

    // Plausible wrong impl: notify_waiters not called on complete, waiters hang forever
    #[tokio::test]
    async fn complete_wakes_waiters() {
        let t = Arc::new(DedupTable::new());
        let (_, notify) = t.register(&"key".to_string());
        let t2 = Arc::clone(&t);
        // Clone so the future owns the Arc and lives 'static in the spawn.
        let notify2 = Arc::clone(&notify);
        let h = tokio::spawn(async move {
            // Create the Notified future inside the task — it borrows from notify2
            // which is owned by the closure, satisfying 'static.
            tokio::time::timeout(std::time::Duration::from_millis(200), notify2.notified())
                .await
                .expect("waiter must be notified within 200ms")
        });
        // Yield to let the spawned task poll notified() at least once before we
        // fire complete(); tokio::sync::Notify is edge-triggered.
        tokio::task::yield_now().await;
        t2.complete(&"key".to_string());
        h.await.unwrap();
    }

    // After complete, new register for same key should be first again
    #[test]
    fn re_register_after_complete_is_first() {
        let t = DedupTable::new();
        let (first, _) = t.register(&"key1".to_string());
        assert!(first);
        t.complete(&"key1".to_string());
        let (first2, _) = t.register(&"key1".to_string());
        assert!(first2, "after complete, next registration must be first");
    }
}
