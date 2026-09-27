use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use vkdg_core::{CapabilitySet, ConnectionId};

use super::connection::{Connection, ConnectionConfig};

pub struct ConnectionCatalog {
    /// Replaced on config reload. Each `Connection` keeps its identity (and its
    /// in-flight counter and health) across reloads while its config changes.
    connections: parking_lot::RwLock<HashMap<ConnectionId, Arc<RwLock<Connection>>>>,
}

impl ConnectionCatalog {
    pub fn new(configs: Vec<ConnectionConfig>) -> Self {
        let connections = configs
            .into_iter()
            .map(|cfg| {
                let id = cfg.id.clone();
                (id, Arc::new(RwLock::new(Connection::new(cfg))))
            })
            .collect();
        Self {
            connections: parking_lot::RwLock::new(connections),
        }
    }

    /// Apply the connections of a new, validated config snapshot.
    ///
    /// A connection whose id survives keeps its `Connection` (in-flight counter,
    /// health state) and takes the new config. New ids start fresh; removed ids
    /// leave the catalog, and requests already holding a guard on them finish
    /// normally because the guard owns its counter.
    pub async fn apply(&self, configs: Vec<ConnectionConfig>) {
        let current = self.connections.read().clone();
        let mut next = HashMap::with_capacity(configs.len());
        for cfg in configs {
            let id = cfg.id.clone();
            let conn = match current.get(&id) {
                Some(existing) => {
                    existing.write().await.config = cfg;
                    Arc::clone(existing)
                }
                None => Arc::new(RwLock::new(Connection::new(cfg))),
            };
            next.insert(id, conn);
        }
        *self.connections.write() = next;
    }

    pub fn get(&self, id: &ConnectionId) -> Option<Arc<RwLock<Connection>>> {
        self.connections.read().get(id).cloned()
    }

    /// Connections that are Healthy, have capacity, and serve `model`.
    /// Runs synchronous reads; callers on an async runtime should use
    /// `blocking_read` only when the lock is uncontended (catalog mutations
    /// are rare configuration events, not hot-path writes).
    pub fn eligible(&self, model: &str, exclude: &[ConnectionId]) -> Vec<ConnectionId> {
        self.connections
            .read()
            .iter()
            .filter_map(|(id, arc)| {
                if exclude.contains(id) {
                    return None;
                }
                // `try_read` — if the lock is held by a writer we skip rather
                // than block the caller; the writer is a state-transition event
                // and the connection will appear in the next routing attempt.
                let conn = arc.try_read().ok()?;
                if conn.state.is_healthy() && conn.has_capacity() && conn.serves_model(model) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Like `eligible`, but also filters out connections whose declared
    /// `capabilities` do not cover every capability in `required`.
    ///
    /// A connection with an empty `capabilities` set is treated as supporting
    /// *nothing* explicitly — it will be excluded if `required` is non-empty.
    /// This ensures fail-closed behaviour: capability mismatch is never silent.
    pub fn eligible_for_operation(
        &self,
        model: &str,
        exclude: &[ConnectionId],
        required: &CapabilitySet,
    ) -> Vec<ConnectionId> {
        self.connections
            .read()
            .iter()
            .filter_map(|(id, arc)| {
                if exclude.contains(id) {
                    return None;
                }
                let conn = arc.try_read().ok()?;
                if !conn.state.is_healthy() || !conn.has_capacity() || !conn.serves_model(model) {
                    return None;
                }
                // Every required capability must be present.
                // An empty required set means no filtering — all healthy connections pass.
                for cap in &required.0 {
                    if !conn.config.capabilities.contains(cap) {
                        return None;
                    }
                }
                Some(id.clone())
            })
            .collect()
    }

    /// Returns all connection IDs in this catalog.
    /// Used by the pipeline to populate RoutingHints for all known connections.
    pub fn connection_ids(&self) -> Vec<ConnectionId> {
        self.connections.read().keys().cloned().collect()
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;
    use crate::connection::{AuthKind, ProviderKind};

    fn cfg(id: &str, models: &[&str], max: u32) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(id.into()),
            provider: ProviderKind::Custom {
                base_url: "http://localhost".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "UNUSED".into(),
            },
            models: models.iter().map(|m| (*m).to_owned()).collect(),
            max_concurrent: max,
            weight: 1,
            tags: vec![],
            capabilities: CapabilitySet::default(),
        }
    }

    // Plausible wrong impl: reload rebuilds connections, so an in-flight request's
    // guard decrements an orphaned counter and max_concurrent is exceeded.
    #[tokio::test]
    async fn reload_keeps_in_flight_count_and_applies_new_config() {
        let catalog = ConnectionCatalog::new(vec![cfg("a", &["foo-*"], 1)]);
        let guard = catalog
            .get(&ConnectionId("a".into()))
            .unwrap()
            .read()
            .await
            .acquire();
        assert!(guard.is_some());
        assert!(catalog.eligible("foo-1", &[]).is_empty(), "at capacity");

        catalog.apply(vec![cfg("a", &["bar-*"], 1)]).await;
        assert!(
            catalog.eligible("bar-1", &[]).is_empty(),
            "still at capacity after reload"
        );
        drop(guard);
        assert_eq!(
            catalog.eligible("bar-1", &[]),
            vec![ConnectionId("a".into())]
        );
        assert!(
            catalog.eligible("foo-1", &[]).is_empty(),
            "old model list gone"
        );
    }

    #[tokio::test]
    async fn reload_adds_and_removes_connections() {
        let catalog = ConnectionCatalog::new(vec![cfg("a", &["*"], 4)]);
        catalog.apply(vec![cfg("b", &["*"], 4)]).await;
        assert_eq!(catalog.connection_ids(), vec![ConnectionId("b".into())]);
        assert!(catalog.get(&ConnectionId("a".into())).is_none());
    }
}
