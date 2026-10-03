//! Claude Code model discovery: `GET /v1/models?limit=1000`.
//!
//! The subscription token authorises the same list endpoint an API key does,
//! with the headers `prepare` sends for messages. Ids come back in Anthropic's
//! spelling (`claude-sonnet-4-6`) and are stored as is.
//!
//! ```text
//! GET https://api.anthropic.com/v1/models?limit=1000[&after_id=<last_id>]
//! => 200 { data:[{ id:"claude-sonnet-4-6", display_name:"Claude Sonnet 4.6", … }],
//!          has_more:false, first_id:"…", last_id:"…" }
//! ```

use futures::future::BoxFuture;
use serde_json::Value;
use vkdg_connections::{ConnectionConfig, Credential};
use vkdg_provider_sdk::{ModelCatalog, ProviderError};

use crate::{base_url, ANTHROPIC_VERSION, OAUTH_BETA};

/// Largest page the API serves.
const PAGE_LIMIT: &str = "1000";

/// Upper bound on pages followed, so a misbehaving upstream cannot loop us.
const MAX_PAGES: usize = 10;

/// Claude Code's [`ModelCatalog`], reachable from [`crate::ClaudeCodeAdapter::model_catalog`].
pub struct ClaudeCodeCatalog;

impl ModelCatalog for ClaudeCodeCatalog {
    fn list_models<'a>(
        &'a self,
        config: &'a ConnectionConfig,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Vec<String>, ProviderError>> {
        Box::pin(async move {
            let client = reqwest::Client::new();
            let url = format!("{}/v1/models", base_url(config));
            let mut ids: Vec<String> = Vec::new();
            let mut after: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let mut query = vec![("limit", PAGE_LIMIT)];
                if let Some(cursor) = after.as_deref() {
                    query.push(("after_id", cursor));
                }
                let resp = client
                    .get(&url)
                    .query(&query)
                    .bearer_auth(&credential.token)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .header("anthropic-beta", OAUTH_BETA)
                    .send()
                    .await
                    .map_err(|e| ProviderError::Http(e.to_string()))?;
                let status = resp.status();
                let text = resp
                    .text()
                    .await
                    .map_err(|e| ProviderError::Http(e.to_string()))?;
                if !status.is_success() {
                    return Err(ProviderError::Http(format!(
                        "models returned {}: {}",
                        status.as_u16(),
                        text.chars().take(200).collect::<String>()
                    )));
                }
                let body: Value = serde_json::from_str(&text)
                    .map_err(|e| ProviderError::Http(format!("models body: {e}")))?;
                for id in parse_models(&body)? {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                after = next_cursor(&body);
                if after.is_none() {
                    break;
                }
            }
            Ok(ids)
        })
    }
}

/// Ids in one page of `GET /v1/models`. `display_name` ("Claude Sonnet 4.6") is
/// not a model to request.
fn parse_models(body: &Value) -> Result<Vec<String>, ProviderError> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Http("models carried no data list".into()))?;
    Ok(data
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect())
}

/// `after_id` for the next page: the page's `last_id`, only while `has_more`.
fn next_cursor(body: &Value) -> Option<String> {
    if body.get("has_more").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    body.get("last_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like the live response: display names differ from ids.
    fn page(has_more: bool) -> Value {
        serde_json::json!({
            "data": [
                {
                    "type": "model",
                    "id": "claude-sonnet-4-6",
                    "display_name": "Claude Sonnet 4.6",
                    "created_at": "2026-02-17T00:00:00Z"
                },
                {
                    "type": "model",
                    "id": "claude-haiku-4-5-20251001",
                    "display_name": "Claude Haiku 4.5",
                    "created_at": "2025-10-01T00:00:00Z"
                }
            ],
            "has_more": has_more,
            "first_id": "claude-sonnet-4-6",
            "last_id": "claude-haiku-4-5-20251001"
        })
    }

    // Refutes reading `display_name` instead of `id`, and rewriting the ids:
    // dated ids stay exactly as Anthropic spells them.
    #[test]
    fn ids_come_from_id_untouched() {
        assert_eq!(
            parse_models(&page(false)).unwrap(),
            ["claude-sonnet-4-6", "claude-haiku-4-5-20251001"]
        );
    }

    // Refutes treating an error body (`{"type":"error",…}`) as an empty list,
    // which would wipe the connection's models instead of failing the sync.
    #[test]
    fn a_body_without_data_is_an_error() {
        let err =
            serde_json::json!({ "type": "error", "error": { "type": "authentication_error" } });
        assert!(parse_models(&err).is_err());
        assert!(parse_models(&serde_json::json!({ "data": null })).is_err());
    }

    // Refutes paginating on `last_id` alone (the final page carries one too,
    // so that never stops) and ignoring `has_more`.
    #[test]
    fn the_cursor_exists_only_while_has_more() {
        assert_eq!(
            next_cursor(&page(true)).as_deref(),
            Some("claude-haiku-4-5-20251001")
        );
        assert_eq!(next_cursor(&page(false)), None);
        let no_cursor = serde_json::json!({ "data": [], "has_more": true });
        assert_eq!(next_cursor(&no_cursor), None);
    }

    // Refutes blank ids surviving as model names.
    #[test]
    fn blank_ids_are_skipped() {
        let body = serde_json::json!({ "data": [{ "id": " " }, { "name": "x" }, { "id": "m" }] });
        assert_eq!(parse_models(&body).unwrap(), ["m"]);
    }
}
