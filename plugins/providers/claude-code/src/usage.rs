//! Claude Code rate-limit windows: `GET https://api.anthropic.com/api/oauth/usage`.
//!
//! ```text
//! authorization: Bearer <oauth access token>
//! anthropic-beta: oauth-2025-04-20
//! => 200 { "five_hour":        { "utilization": 6.0,  "resets_at": "2026-04-08T18:59:59Z" },
//!          "seven_day":        { "utilization": 35.0, "resets_at": "…" } | null,
//!          "seven_day_sonnet": { "utilization": 21.0, "resets_at": "…" } | null,
//!          "seven_day_opus":   { "utilization": 12.0, "resets_at": "…" } | null,
//!          "extra_usage": { … } }
//! ```
//!
//! The endpoint is undocumented and plan-dependent: a bucket the plan does not
//! enforce is `null` (or absent). Only buckets that carry a `utilization` become
//! windows; an absent one is never rendered as 0 %.

use chrono::DateTime;
use futures::future::BoxFuture;
use serde_json::Value;
use vkdg_connections::Credential;
use vkdg_provider_sdk::{ProviderError, UsageProvider, UsageSnapshot, UsageWindow, WindowKind};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Response key → window, in display order.
const BUCKETS: [(&str, WindowKind); 4] = [
    ("five_hour", WindowKind::FiveHour),
    ("seven_day", WindowKind::Weekly),
    ("seven_day_sonnet", WindowKind::WeeklySonnet),
    ("seven_day_opus", WindowKind::WeeklyOpus),
];

/// Claude Code's [`UsageProvider`], reachable from [`crate::ClaudeCodeAdapter::usage`].
pub struct ClaudeCodeUsage;

impl UsageProvider for ClaudeCodeUsage {
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

/// GET the usage buckets for one account; its own token authorises the call.
async fn fetch_usage_body(
    client: &reqwest::Client,
    credential: &Credential,
) -> Result<Value, ProviderError> {
    let resp = client
        .get(USAGE_URL)
        .bearer_auth(&credential.token)
        .header("anthropic-beta", OAUTH_BETA)
        .header("accept", "application/json")
        .header("user-agent", "vkdg")
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
            "oauth usage returned {}: {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        )));
    }
    serde_json::from_str(&text).map_err(|e| ProviderError::Http(format!("usage body: {e}")))
}

/// Map a usage body to a [`UsageSnapshot`].
///
/// Only buckets carrying a numeric `utilization` become windows. Fails with
/// [`ProviderError::Http`] when none does, so the caller renders "unavailable"
/// instead of a fabricated 0 %.
pub fn parse_usage(body: &Value) -> Result<UsageSnapshot, ProviderError> {
    let windows: Vec<UsageWindow> = BUCKETS
        .iter()
        .filter_map(|(key, kind)| {
            let bucket = body.get(*key)?;
            let used_percent = bucket.get("utilization")?.as_f64()?;
            let resets_at = bucket
                .get("resets_at")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp());
            Some(UsageWindow {
                kind: *kind,
                used_percent,
                resets_at,
            })
        })
        .collect();
    if windows.is_empty() {
        return Err(ProviderError::Http(
            "usage body has no rate-limit window".into(),
        ));
    }
    Ok(UsageSnapshot {
        windows,
        ..UsageSnapshot::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kinds(s: &UsageSnapshot) -> Vec<WindowKind> {
        s.windows.iter().map(|w| w.kind).collect()
    }

    // Refutes assuming every plan has a general weekly cap: this plan reports a
    // 5h window and per-model weekly windows, `seven_day` is null. A parser that
    // always emitted a Weekly window (0 % or otherwise) would fail here.
    #[test]
    fn plan_without_general_weekly_reports_only_what_it_has() {
        let body = json!({
            "five_hour":        { "utilization": 49.0, "resets_at": "2026-04-08T18:59:59Z" },
            "seven_day":        null,
            "seven_day_sonnet": { "utilization": 21.0, "resets_at": "2026-04-14T17:59:59Z" },
            "seven_day_opus":   null
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(
            kinds(&s),
            vec![WindowKind::FiveHour, WindowKind::WeeklySonnet]
        );
        assert_eq!(s.windows[0].used_percent, 49.0);
        assert_eq!(s.windows[1].used_percent, 21.0);
    }

    // Refutes parsing the reset time as anything but the RFC3339 instant, and
    // refutes mixing up which bucket owns which timestamp.
    #[test]
    fn reset_time_is_unix_seconds_of_the_rfc3339_instant() {
        let body = json!({
            "five_hour": { "utilization": 6.0, "resets_at": "2026-04-08T18:59:59Z" },
            "seven_day": { "utilization": 35.0, "resets_at": "2026-04-14T16:59:59Z" }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(s.windows[0].resets_at, Some(1_775_674_799));
        assert_eq!(s.windows[1].resets_at, Some(1_776_185_999));
    }

    // Refutes turning a missing / unparseable reset time into epoch 0.
    #[test]
    fn missing_or_garbled_reset_stays_unknown() {
        let body = json!({
            "five_hour": { "utilization": 6.0, "resets_at": null },
            "seven_day": { "utilization": 35.0, "resets_at": "not a date" },
            "seven_day_opus": { "utilization": 1.0 }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(s.windows.len(), 3);
        assert!(s.windows.iter().all(|w| w.resets_at.is_none()));
    }

    // Refutes defaulting a bucket without `utilization` to 0 %.
    #[test]
    fn bucket_without_utilization_is_skipped_not_zeroed() {
        let body = json!({
            "five_hour": { "utilization": 12.5, "resets_at": null },
            "seven_day": { "resets_at": "2026-04-14T16:59:59Z" }
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(kinds(&s), vec![WindowKind::FiveHour]);
    }

    // Refutes a "no data → all windows at 0 %" fabrication: no usable bucket is
    // an error the caller renders "unavailable".
    #[test]
    fn body_without_any_window_is_an_error_not_zero() {
        assert!(parse_usage(&json!({})).is_err());
        assert!(parse_usage(&json!({ "five_hour": null, "seven_day": null })).is_err());
        assert!(parse_usage(&json!({ "extra_usage": { "utilization": 12.5 } })).is_err());
    }

    // Refutes the credit fields being populated for a window-metered provider.
    #[test]
    fn windows_only_provider_carries_no_credit_figures() {
        let s = parse_usage(&json!({ "five_hour": { "utilization": 1.0 } })).unwrap();
        assert_eq!(s.credits_used, None);
        assert_eq!(s.credits_limit, None);
        assert_eq!(s.credits_period_end, None);
    }
}
