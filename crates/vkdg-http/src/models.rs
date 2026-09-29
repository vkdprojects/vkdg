//! `GET /v1/models`: the models this gateway can serve to the calling key.
//!
//! Lists concrete model ids named in config (connection `models`, route
//! `match_models`), skipping glob patterns, which name families rather than
//! models. Filtered by the key's `allowed_models`, so a client never sees a
//! model it would be refused. The body follows the caller's dialect: Anthropic
//! when the request carries `anthropic-version`, `OpenAI` otherwise.

use std::collections::BTreeSet;

use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::{AppState, ClientIdentity};

/// Concrete model ids the pipeline can route, sorted and deduplicated.
pub async fn available_models(state: &AppState) -> Vec<String> {
    let Some(pipeline) = &state.pipeline else {
        return vec![];
    };
    let mut models: BTreeSet<String> = pipeline
        .router
        .routes()
        .iter()
        .flat_map(|r| r.match_models.iter().cloned())
        .collect();
    for id in pipeline.catalog.connection_ids() {
        if let Some(conn) = pipeline.catalog.get(&id) {
            models.extend(conn.read().await.config.models.iter().cloned());
        }
    }
    models.into_iter().filter(|m| is_concrete(m)).collect()
}

pub async fn list_models(State(state): State<AppState>, req: Request) -> Response {
    let identity = req.extensions().get::<ClientIdentity>().cloned();
    let anthropic = req.headers().contains_key("anthropic-version");
    let models: Vec<String> = available_models(&state)
        .await
        .into_iter()
        .filter(|m| {
            identity
                .as_ref()
                .map_or(true, |id| id.check_model(m).is_ok())
        })
        .collect();
    let body = if anthropic {
        json!({
            "data": models.iter().map(|m| json!({
                "type": "model", "id": m, "display_name": m, "created_at": "1970-01-01T00:00:00Z",
            })).collect::<Vec<_>>(),
            "has_more": false,
            "first_id": models.first(),
            "last_id": models.last(),
        })
    } else {
        json!({
            "object": "list",
            "data": models.iter().map(|m| json!({
                "id": m, "object": "model", "created": 0, "owned_by": "vkdg",
            })).collect::<Vec<_>>(),
        })
    };
    Json(body).into_response()
}

fn is_concrete(model: &str) -> bool {
    !model.is_empty() && !model.contains(['*', '?'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vkdg_connections::{AuthKind, ConnectionCatalog, ConnectionConfig, ProviderKind};
    use vkdg_core::{CapabilitySet, ConnectionId};
    use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

    fn state() -> AppState {
        let conn = ConnectionConfig {
            id: ConnectionId("c1".into()),
            provider: ProviderKind::Custom {
                base_url: "http://127.0.0.1:1".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "UNUSED".into(),
            },
            models: vec!["claude-*".into(), "gpt-4o-mini".into(), "auto".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec![
                "claude-sonnet-4.5".into(),
                "gpt-4o-mini".into(),
                "o?-*".into(),
            ],
            strategy: StrategyKind::RoundRobin,
            targets: vec![ConnectionId("c1".into())],
            plugin_hooks: PluginHooks::default(),
        };
        let pipeline = crate::PipelineState::minimal(
            Arc::new(crate::AdmissionGuard::new(1)),
            Arc::new(Router::new(vec![route])),
            Arc::new(ConnectionCatalog::new(vec![conn])),
            Arc::new(vkdg_connections::CredentialManager::new()),
            Arc::new(crate::HttpClient::new()),
            Arc::new(vkdg_observe::DecisionRecordExporter::new()),
            Arc::new(vkdg_provider_sdk::ProviderRegistry::empty()),
        );
        AppState::new(crate::ServerConfig::default()).with_pipeline(Arc::new(pipeline))
    }

    async fn call(
        state: AppState,
        headers: &[(&str, &str)],
        allowed: &[&str],
    ) -> serde_json::Value {
        let mut req = http::Request::get("/v1/models");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut req = req.body(axum::body::Body::empty()).unwrap();
        req.extensions_mut().insert(ClientIdentity {
            key_id: "k".into(),
            tenant_id: "t".into(),
            client_ip: None,
            allowed_models: allowed
                .iter()
                .map(|m| (*m).to_owned())
                .collect::<Vec<_>>()
                .into(),
        });
        let resp = list_models(State(state), req).await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn ids(v: &serde_json::Value) -> Vec<String> {
        v["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn lists_concrete_models_from_config_without_globs() {
        let v = call(state(), &[], &[]).await;
        assert_eq!(v["object"], "list");
        assert_eq!(ids(&v), ["auto", "claude-sonnet-4.5", "gpt-4o-mini"]);
    }

    // A client must not be offered a model its key would be refused.
    #[tokio::test]
    async fn key_model_list_filters_the_catalog() {
        let v = call(state(), &[], &["claude-*"]).await;
        assert_eq!(ids(&v), ["claude-sonnet-4.5"]);
    }

    #[tokio::test]
    async fn anthropic_clients_get_the_anthropic_shape() {
        let v = call(state(), &[("anthropic-version", "2023-06-01")], &[]).await;
        assert_eq!(v["data"][0]["type"], "model");
        assert_eq!(v["has_more"], false);
        assert_eq!(v["first_id"], "auto");
    }

    #[test]
    fn globs_are_not_models() {
        assert!(is_concrete("gpt-4o"));
        assert!(!is_concrete("claude-*") && !is_concrete("o?-mini") && !is_concrete(""));
    }
}
