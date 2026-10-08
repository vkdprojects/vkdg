//! Codex rate-limit windows: `GET https://chatgpt.com/backend-api/wham/usage`.
//!
//! ```text
//! authorization: Bearer <oauth access token>
//! chatgpt-account-id: <account id from the token's JWT claims>
//! => 200 { "plan_type": "plus",
//!          "rate_limit": {
//!            "primary_window":   { "used_percent": 42, "limit_window_seconds": 18000,
//!                                  "reset_after_seconds": 1234, "reset_at": 1776185999 },
//!            "secondary_window": { … "limit_window_seconds": 604800 … } | null } }
//! ```
//!
//! The endpoint is undocumented. Which window is "primary" depends on the plan:
//! some plans report a lone weekly window as `primary_window`. A window is
//! therefore classified by its own `limit_window_seconds`, never by its slot, and
//! a window without that field is skipped rather than guessed.
use futures::future::BoxFuture;
use serde_json::Value;
use vkdg_connections::Credential;
use vkdg_provider_sdk::{ProviderError, UsageProvider, UsageSnapshot, UsageWindow, WindowKind};

use crate::extract_chatgpt_account_id;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// A window at least this long is the weekly one; anything shorter is the
/// 5-hour one. Splits 18 000 s from 604 800 s with a wide margin either way.
const WEEKLY_MIN_SECONDS: u64 = 2 * 24 * 60 * 60;

/// Codex's [`UsageProvider`], reachable from [`crate::CodexAdapter::usage`].
pub struct CodexUsage;

impl UsageProvider for CodexUsage {
    fn fetch_usage<'a>(
        &'a self,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<UsageSnapshot, ProviderError>> {
        Box::pin(async move {
            let body = fetch_usage_body(&reqwest::Client::new(), credential).await?;
            parse_usage(&body)
        })
    }
}

/// GET the rate-limit windows for one account; its own token and `ChatGPT`
/// account id authorise the call.
async fn fetch_usage_body(
    client: &reqwest::Client,
    credential: &Credential,
) -> Result<Value, ProviderError> {
    let account_id = credential
        .extra
        .get("chatgpt_account_id")
        .cloned()
        .or_else(|| extract_chatgpt_account_id(&credential.token));
    let mut req = client
        .get(USAGE_URL)
        .bearer_auth(&credential.token)
        .header("accept", "application/json")
        .header("origin", "https://chatgpt.com")
        .header("referer", "https://chatgpt.com/")
        .header("user-agent", "Mozilla/5.0");
    if let Some(id) = account_id {
        req = req.header("chatgpt-account-id", id);
    }
    let resp = req
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
            "wham usage returned {}: {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        )));
    }
    serde_json::from_str(&text).map_err(|e| ProviderError::Http(format!("usage body: {e}")))
}

/// Map a usage body to a [`UsageSnapshot`].
///
/// Fails with [`ProviderError::Http`] when no window is usable, so the caller
/// renders "unavailable" instead of a fabricated 0 %.
pub fn parse_usage(body: &Value) -> Result<UsageSnapshot, ProviderError> {
    let rate_limit = body.get("rate_limit");
    let windows: Vec<UsageWindow> = ["primary_window", "secondary_window"]
        .iter()
        .filter_map(|slot| parse_window(rate_limit?.get(*slot)?))
        .collect();
    if windows.is_empty() {
        return Err(ProviderError::Http(
            "usage body has no rate-limit window".into(),
        ));
    }
    Ok(UsageSnapshot {
        windows,
        plan: body
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..UsageSnapshot::default()
    })
}

/// One window, kind decided by its duration. `None` when the percentage or the
/// duration is missing — neither is defaulted.
fn parse_window(window: &Value) -> Option<UsageWindow> {
    let used_percent = window.get("used_percent")?.as_f64()?;
    let seconds = window.get("limit_window_seconds")?.as_u64()?;
    let kind = if seconds >= WEEKLY_MIN_SECONDS {
        WindowKind::Weekly
    } else {
        WindowKind::FiveHour
    };
    Some(UsageWindow {
        kind,
        used_percent,
        resets_at: window.get("reset_at").and_then(Value::as_i64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kinds(s: &UsageSnapshot) -> Vec<WindowKind> {
        s.windows.iter().map(|w| w.kind).collect()
    }

    // Refutes position-based classification and a "both windows always present"
    // assumption: 5 h primary + weekly secondary map to FiveHour + Weekly.
    #[test]
    fn five_hour_primary_and_weekly_secondary() {
        let body = json!({
            "plan_type": "plus",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 42, "limit_window_seconds": 18000,
                    "reset_after_seconds": 1234, "reset_at": 1_776_185_999
                },
                "secondary_window": {
                    "used_percent": 7.5, "limit_window_seconds": 604_800,
                    "reset_after_seconds": 99999, "reset_at": 1_776_700_000
                }
            }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(kinds(&s), vec![WindowKind::FiveHour, WindowKind::Weekly]);
        assert_eq!(s.windows[0].used_percent, 42.0);
        assert_eq!(s.windows[0].resets_at, Some(1_776_185_999));
        assert_eq!(s.windows[1].used_percent, 7.5);
        assert_eq!(s.plan.as_deref(), Some("plus"));
    }

    // Refutes "primary is always the 5 h window": a plan that dropped the 5 h
    // window returns a lone weekly `primary_window` (604800 s) and a null
    // secondary. It must render as ONE weekly window, not a mislabelled 5 h one.
    #[test]
    fn lone_weekly_primary_is_classified_by_duration() {
        let body = json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 60, "limit_window_seconds": 604_800, "reset_at": 1_776_700_000
                },
                "secondary_window": null
            }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(kinds(&s), vec![WindowKind::Weekly]);
    }

    // Refutes guessing a window's kind from its slot when the duration is absent.
    #[test]
    fn window_without_duration_is_skipped_not_guessed() {
        let body = json!({
            "rate_limit": {
                "primary_window": { "used_percent": 10 },
                "secondary_window": {
                    "used_percent": 20, "limit_window_seconds": 604_800
                }
            }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(kinds(&s), vec![WindowKind::Weekly]);
    }

    // Refutes defaulting a missing percentage to 0 % and a missing reset to
    // epoch 0.
    #[test]
    fn missing_percent_is_skipped_and_missing_reset_stays_unknown() {
        let body = json!({
            "rate_limit": {
                "primary_window": { "limit_window_seconds": 18000, "reset_at": 5 },
                "secondary_window": { "used_percent": 20, "limit_window_seconds": 604_800 }
            }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(kinds(&s), vec![WindowKind::Weekly]);
        assert_eq!(s.windows[0].resets_at, None);
    }

    // Refutes "no data → 0 %": nothing usable is an error ("unavailable").
    #[test]
    fn body_without_any_window_is_an_error_not_zero() {
        assert!(parse_usage(&json!({})).is_err());
        assert!(parse_usage(&json!({ "rate_limit": {} })).is_err());
        assert!(parse_usage(&json!({
            "rate_limit": { "primary_window": null, "secondary_window": null }
        }))
        .is_err());
    }

    // Refutes populating credit figures for a window-metered provider.
    #[test]
    fn windows_only_provider_carries_no_credit_figures() {
        let s = parse_usage(&json!({
            "rate_limit": { "primary_window": {
                "used_percent": 1, "limit_window_seconds": 18000
            } }
        }))
        .unwrap();
        assert_eq!(s.credits_used, None);
        assert_eq!(s.credits_limit, None);
    }
}
