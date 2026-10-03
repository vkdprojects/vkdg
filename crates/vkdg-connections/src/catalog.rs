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

    /// Connections whose declared `models` do not cover `model`.
    ///
    /// A route names its targets by id, so its `match_models` glob says nothing
    /// about what each target actually serves. Without this the pipeline lets a
    /// strategy pick a connection outside its catalogue and the upstream answers
    /// an opaque 400. Health and capacity are deliberately not considered here:
    /// those are transient and owned by the rate-limit filter, while a missing
    /// model is a static property of the configuration.
    pub fn not_serving_model(&self, model: &str) -> Vec<ConnectionId> {
        self.connections
            .read()
            .iter()
            .filter_map(|(id, arc)| {
                let conn = arc.try_read().ok()?;
                (!conn.serves_model(model)).then(|| id.clone())
            })
            .collect()
    }

    /// Connections that cannot take this request right now, each with the reason.
    ///
    /// Three independent causes, and the caller needs them separated in the
    /// decision record: the model is outside the connection's catalogue (static),
    /// the connection is unhealthy or in cooldown, or it is already at
    /// `max_concurrent` (both transient). Routing by route alone ignores all
    /// three, so a strategy can pick a target that then fails to reserve — the
    /// request dies as "no eligible connection" instead of trying a sibling.
    pub fn unroutable(&self, model: &str) -> Vec<(ConnectionId, &'static str)> {
        self.connections
            .read()
            .iter()
            .filter_map(|(id, arc)| {
                let conn = arc.try_read().ok()?;
                if !conn.serves_model(model) {
                    return Some((id.clone(), "model_not_served"));
                }
                if !conn.state.is_healthy() {
                    return Some((id.clone(), "unhealthy"));
                }
                if !conn.has_capacity() {
                    return Some((id.clone(), "at_capacity"));
                }
                None
            })
            .collect()
    }

    /// Seconds until the first connection serving `model` leaves cooldown or an
    /// open circuit, when at least one is in that state. `None` when none is.
    /// Lets the pipeline answer "everything is cooling down" with a `429` and
    /// a `retry-after` instead of "no eligible connection".
    pub fn secs_until_cooldown_ends(&self, model: &str) -> Option<u32> {
        use super::connection::ConnectionState as S;
        let now = chrono::Utc::now();
        self.connections
            .read()
            .values()
            .filter_map(|arc| {
                let conn = arc.try_read().ok()?;
                if !conn.serves_model(model) {
                    return None;
                }
                match &conn.state {
                    S::Cooldown { until, .. } | S::CircuitOpen { until } if *until > now => {
                        Some(*until)
                    }
                    _ => None,
                }
            })
            .min()
            .map(|until| {
                // Round up so a client that waits exactly this long is past it.
                u32::try_from((until - now).num_seconds().max(0) + 1).unwrap_or(u32::MAX)
            })
    }

    /// Returns all connection IDs in this catalog.
    /// Used by the pipeline to populate `RoutingHints` for all known connections.
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
            endpoint: None,
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

    // Plausible wrong impl: routing trusts only the route's `match_models` and
    // never checks the connection's own catalogue, so a round-robin route whose
    // targets have different catalogues sends every other request to a
    // connection that cannot serve the model. Observed in production as
    // alternating 200 / 400 INVALID_MODEL_ID on a two-target Kiro route.
    #[tokio::test]
    async fn connections_that_do_not_serve_the_model_are_reported() {
        let catalog = ConnectionCatalog::new(vec![
            cfg("full", &["claude-*", "glm-*"], 4),
            cfg("limited", &["claude-sonnet-4.5"], 4),
        ]);

        assert_eq!(
            catalog.not_serving_model("glm-5"),
            vec![ConnectionId("limited".into())],
            "only the connection without the model is reported"
        );
        assert!(
            catalog.not_serving_model("claude-sonnet-4.5").is_empty(),
            "a model both serve excludes nobody"
        );
        // A connection at capacity or in cooldown is still *able* to serve the
        // model: that is the rate-limit filter's job, not this one. Reporting it
        // here would permanently exclude a healthy target.
        let guard = catalog
            .get(&ConnectionId("full".into()))
            .unwrap()
            .read()
            .await
            .acquire();
        assert!(guard.is_some());
        assert!(
            !catalog
                .not_serving_model("glm-5")
                .contains(&ConnectionId("full".into())),
            "capacity is not a catalogue decision"
        );
    }

    // Plausible wrong impl: filtering candidates by model only. A route target
    // already at `max_concurrent` is then still picked, fails to reserve, and
    // the request dies as "no eligible connection" without trying the sibling
    // that was free — observed in production as 502s under 12 concurrent
    // requests across three Kiro connections.
    #[tokio::test]
    async fn a_target_without_a_free_slot_is_reported_with_its_reason() {
        let catalog = ConnectionCatalog::new(vec![
            cfg("busy", &["claude-*"], 1),
            cfg("free", &["claude-*"], 1),
            cfg("other-model", &["glm-*"], 1),
        ]);
        let guard = catalog
            .get(&ConnectionId("busy".into()))
            .unwrap()
            .read()
            .await
            .acquire();
        assert!(guard.is_some(), "the only slot is taken");

        let out: std::collections::HashMap<_, _> = catalog
            .unroutable("claude-sonnet-5")
            .into_iter()
            .map(|(id, reason)| (id.0, reason))
            .collect();

        assert_eq!(out.get("busy").copied(), Some("at_capacity"));
        assert_eq!(out.get("other-model").copied(), Some("model_not_served"));
        assert!(
            !out.contains_key("free"),
            "the free target must stay routable: {out:?}"
        );

        drop(guard);
        assert!(
            !catalog
                .unroutable("claude-sonnet-5")
                .iter()
                .any(|(id, _)| id.0 == "busy"),
            "releasing the slot makes it routable again"
        );
    }
}
