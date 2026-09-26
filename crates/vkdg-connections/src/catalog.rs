use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use vkdg_core::{CapabilitySet, ConnectionId};

use super::connection::{Connection, ConnectionConfig};

pub struct ConnectionCatalog {
    connections: HashMap<ConnectionId, Arc<RwLock<Connection>>>,
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
        Self { connections }
    }

    pub fn get(&self, id: &ConnectionId) -> Option<Arc<RwLock<Connection>>> {
        self.connections.get(id).cloned()
    }

    /// Connections that are Healthy, have capacity, and serve `model`.
    /// Runs synchronous reads; callers on an async runtime should use
    /// `blocking_read` only when the lock is uncontended (catalog mutations
    /// are rare configuration events, not hot-path writes).
    pub fn eligible(&self, model: &str, exclude: &[ConnectionId]) -> Vec<ConnectionId> {
        self.connections
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
        self.connections.keys().cloned().collect()
    }
}
