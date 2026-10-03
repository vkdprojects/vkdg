//! Kiro model discovery: `GET /ListAvailableModels?origin=AI_EDITOR`.
//!
//! AWS's own CLI does not hardcode a model list either (see [`crate::models`]):
//! it asks the control plane which models the account may call. The ids come back
//! in Kiro's spelling, dots included (`claude-sonnet-4.6`), and are stored as is;
//! the connection matches client names against them with dots and dashes treated
//! alike.
//!
//! ```text
//! GET https://q.us-east-1.amazonaws.com/ListAvailableModels?origin=AI_EDITOR
//!   tokentype: API_KEY            (API-key credential only)
//!   authorization: Bearer <token>
//! => 200 { models:[{ modelId:"claude-sonnet-4.6", modelName:"Claude Sonnet 4.6", … }],
//!          defaultModel:{…}, nextToken:null }
//! ```

use futures::future::BoxFuture;
use serde_json::Value;
use vkdg_connections::{ConnectionConfig, Credential};
use vkdg_provider_sdk::{ModelCatalog, ProviderError};

use crate::auth::{AUTH_API_KEY, AUTH_BUILDER_ID};
use crate::endpoint::sends_profile_arn;
use crate::models::AUTO_MODEL;
use crate::usage::{authorize, control_plane_base};

/// Upper bound on pages followed, so a misbehaving upstream cannot loop us.
const MAX_PAGES: usize = 10;

/// Kiro's [`ModelCatalog`], reachable from [`crate::KiroAdapter::model_catalog`].
pub struct KiroModelCatalog;

impl ModelCatalog for KiroModelCatalog {
    fn list_models<'a>(
        &'a self,
        config: &'a ConnectionConfig,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Vec<String>, ProviderError>> {
        Box::pin(async move {
            let client = reqwest::Client::new();
            let auth = auth_method(config, credential);
            let mut ids: Vec<String> = Vec::new();
            let mut next: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let url = request_url(credential, auth, next.as_deref())?;
                let resp = authorize(
                    client.get(url).header("accept", "application/json"),
                    credential,
                    auth,
                )
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
                        "ListAvailableModels returned {}: {}",
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
                next = next_token(&body);
                if next.is_none() {
                    break;
                }
            }
            Ok(ids)
        })
    }
}

/// Same rule as `prepare`: the login records the method; a connection that
/// authenticates with an env-var key (no account) is the API-key flow.
fn auth_method<'a>(config: &ConnectionConfig, credential: &'a Credential) -> &'a str {
    credential.extra.get("auth_method").map_or(
        match config.auth {
            vkdg_connections::AuthKind::ApiKey { .. } => AUTH_API_KEY,
            _ => AUTH_BUILDER_ID,
        },
        String::as_str,
    )
}

/// `ListAvailableModels` URL for one page. An OAuth account names its profile; an
/// API key must not (AWS answers 403), exactly as for chat requests.
fn request_url(
    credential: &Credential,
    auth_method: &str,
    next_token: Option<&str>,
) -> Result<reqwest::Url, ProviderError> {
    let mut params: Vec<(&str, &str)> = vec![("origin", "AI_EDITOR")];
    if sends_profile_arn(auth_method) {
        if let Some(arn) = credential.extra.get("profile_arn") {
            params.push(("profileArn", arn.as_str()));
        }
    }
    if let Some(token) = next_token {
        params.push(("nextToken", token));
    }
    reqwest::Url::parse_with_params(
        &format!("{}/ListAvailableModels", control_plane_base(credential)),
        params,
    )
    .map_err(|e| ProviderError::Config(format!("ListAvailableModels url: {e}")))
}

/// Model ids in one `ListAvailableModels` page, plus `auto` (the service picks
/// the model; it is accepted although the list does not name it).
///
/// The id is `modelId`. `modelName` is the display name ("Claude Sonnet 4.6")
/// and is not a valid model to request.
fn parse_models(body: &Value) -> Result<Vec<String>, ProviderError> {
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Http("ListAvailableModels carried no models list".into()))?;
    let mut ids: Vec<String> = Vec::with_capacity(models.len() + 1);
    for id in models
        .iter()
        .filter_map(|m| m.get("modelId").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_owned());
        }
    }
    if !ids.iter().any(|id| id == AUTO_MODEL) {
        ids.push(AUTO_MODEL.to_owned());
    }
    Ok(ids)
}

/// Continuation token of the page, absent (or null, or empty) on the last one.
fn next_token(body: &Value) -> Option<String> {
    body.get("nextToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    // Shaped like the live response (2026-09): display names differ from ids.
    fn page() -> Value {
        serde_json::json!({
            "defaultModel": { "modelId": "auto", "modelName": "Auto" },
            "models": [
                {
                    "description": "Latest Claude Sonnet",
                    "modelId": "claude-sonnet-4.6",
                    "modelName": "Claude Sonnet 4.6",
                    "rateMultiplier": 1.3,
                    "rateUnit": "credit",
                    "supportedInputTypes": ["TEXT", "IMAGE"],
                    "tokenLimits": { "maxInputTokens": 200_000, "maxOutputTokens": 64_000 }
                },
                {
                    "modelId": "claude-haiku-4.5",
                    "modelName": "Claude Haiku 4.5",
                    "tokenLimits": { "maxInputTokens": 200_000, "maxOutputTokens": 64_000 }
                },
                { "modelId": "deepseek-3.2", "modelName": "DeepSeek 3.2" }
            ],
            "nextToken": null
        })
    }

    // Refutes reading `modelName` (a display name no upstream accepts) and
    // dotted ids being rewritten: the id is kept in Kiro's spelling.
    #[test]
    fn ids_come_from_model_id_in_kiros_own_spelling() {
        let ids = parse_models(&page()).unwrap();
        assert_eq!(
            ids,
            [
                "claude-sonnet-4.6",
                "claude-haiku-4.5",
                "deepseek-3.2",
                "auto"
            ]
        );
    }

    // Refutes dropping `auto` (the list never names it) and duplicating it when
    // the service does.
    #[test]
    fn auto_is_always_listed_exactly_once() {
        let with_auto = serde_json::json!({ "models": [
            { "modelId": "auto" }, { "modelId": "claude-sonnet-4.6" }
        ]});
        let ids = parse_models(&with_auto).unwrap();
        assert_eq!(ids.iter().filter(|id| *id == "auto").count(), 1);
        assert_eq!(ids.len(), 2);
        let none = parse_models(&serde_json::json!({ "models": [] })).unwrap();
        assert_eq!(none, ["auto"]);
    }

    // Refutes treating an error-shaped body as "no models": that would wipe a
    // connection's list instead of failing the sync.
    #[test]
    fn a_body_without_a_models_list_is_an_error() {
        assert!(parse_models(&serde_json::json!({ "message": "denied" })).is_err());
        assert!(parse_models(&serde_json::json!({ "models": "x" })).is_err());
    }

    // Refutes entries without an id (or a blank one) becoming empty model names.
    #[test]
    fn entries_without_an_id_are_skipped() {
        let body = serde_json::json!({ "models": [
            { "modelName": "No id" }, { "modelId": "  " }, { "modelId": " glm-5 " }
        ]});
        assert_eq!(parse_models(&body).unwrap(), ["glm-5", "auto"]);
    }

    // Refutes following a null/empty token forever, and ignoring a real one.
    #[test]
    fn next_token_only_for_a_real_continuation() {
        assert_eq!(next_token(&page()), None);
        assert_eq!(next_token(&serde_json::json!({ "nextToken": "" })), None);
        assert_eq!(next_token(&serde_json::json!({})), None);
        assert_eq!(
            next_token(&serde_json::json!({ "nextToken": "abc" })).as_deref(),
            Some("abc")
        );
    }

    fn credential(extra: &[(&str, &str)]) -> Credential {
        Credential {
            token: "tok".into(),
            extra: Arc::new(
                extra
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect::<HashMap<_, _>>(),
            ),
        }
    }

    // Refutes sending `profileArn` with an API key (AWS: 403), and an OAuth
    // account omitting it; the page token must also reach the URL.
    #[test]
    fn profile_arn_follows_the_credential_and_origin_is_ai_editor() {
        let arn = "arn:aws:codewhisperer:eu-central-1:123456789012:profile/ABC";
        let oauth = credential(&[("profile_arn", arn), ("auth_method", "idc")]);
        let url = request_url(&oauth, "idc", None).unwrap();
        assert_eq!(url.host_str(), Some("q.eu-central-1.amazonaws.com"));
        assert_eq!(url.path(), "/ListAvailableModels");
        let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["origin"], "AI_EDITOR");
        assert_eq!(q["profileArn"], arn);

        let key = credential(&[("profile_arn", arn), ("auth_method", "api_key")]);
        let url = request_url(&key, "api_key", Some("page2")).unwrap();
        let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(!q.contains_key("profileArn"), "{q:?}");
        assert_eq!(q["nextToken"], "page2");
    }
}
