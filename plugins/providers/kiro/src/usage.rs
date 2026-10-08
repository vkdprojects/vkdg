//! Kiro credit usage: `AmazonCodeWhispererService.GetUsageLimits`.
//!
//! The chat stream's `meteringEvent` carries a per-request credit delta with no
//! running total, so it cannot answer "how many credits has this account spent
//! this period". The control-plane `GetUsageLimits` operation does: it returns
//! the plan allowance and the period-to-date usage for each billed resource.
//!
//! Measured against the live service (2026-09, API-key accounts):
//!
//! ```text
//! POST https://q.us-east-1.amazonaws.com/
//!   x-amz-target: AmazonCodeWhispererService.GetUsageLimits
//!   tokentype: API_KEY            (API-key credential only)
//!   authorization: Bearer <ksk_…>
//!   body: {}
//! => 200 { subscriptionInfo{subscriptionTitle,…},
//!          usageBreakdownList:[{resourceType:"CREDIT",
//!             currentUsageWithPrecision, usageLimitWithPrecision,
//!             overageCapWithPrecision, nextDateReset, …}],
//!          userInfo{userId?} }
//! ```
//!
//! The credit line is the entry whose `resourceType` is `CREDIT`. Its plan
//! allowance is `usageLimitWithPrecision` and it VARIES per account (KIRO POWER
//! reports 10000, KIRO PRO MAX reports 5000); `overageCapWithPrecision` is the
//! paid-overage ceiling and is a different number — using it as the plan limit
//! would erase that difference. The payload carries no remaining/available
//! field, so any "remaining" is derived (`limit - used`) by the caller.

use std::fmt::Write as _;

use futures::future::BoxFuture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use vkdg_connections::Credential;
use vkdg_provider_sdk::{ProviderError, UsageProvider, UsageSnapshot};

use crate::endpoint::sends_api_key_token_type;
use crate::region::{control_plane_host, runtime_region};

/// `resourceType` of the credit line inside `usageBreakdownList`.
const CREDIT_RESOURCE: &str = "CREDIT";
const USAGE_TARGET: &str = "AmazonCodeWhispererService.GetUsageLimits";

/// Kiro's [`UsageProvider`], reachable from [`crate::KiroAdapter::usage`].
pub struct KiroUsage;

impl UsageProvider for KiroUsage {
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

/// Auth method persisted at login (`api_key` for a `ksk_` key), else Builder ID.
pub(crate) fn auth_method(extra: &std::collections::HashMap<String, String>) -> &str {
    extra
        .get("auth_method")
        .map_or(crate::auth::AUTH_BUILDER_ID, String::as_str)
}

/// `https://q.<region>.amazonaws.com` for the account's runtime region: the
/// host of every control-plane operation (`GetUsageLimits`, `ListAvailableModels`).
pub(crate) fn control_plane_base(credential: &Credential) -> String {
    let extra = credential.extra.as_ref();
    let region = runtime_region(
        extra.get("profile_arn").map(String::as_str),
        extra.get("oidc_region").map(String::as_str),
    );
    control_plane_host(&region)
}

/// The credential's own token, plus `tokentype: API_KEY` for an API key. Never a
/// `profileArn` header: AWS answers 403 when an API key sends one.
pub(crate) fn authorize(
    req: reqwest::RequestBuilder,
    credential: &Credential,
    auth_method: &str,
) -> reqwest::RequestBuilder {
    let req = req.bearer_auth(&credential.token);
    if sends_api_key_token_type(auth_method) {
        req.header("tokentype", "API_KEY")
    } else {
        req
    }
}

/// POST `GetUsageLimits` for one account and return the parsed JSON body.
///
/// The credential's own token authorises the call; nothing is shared between
/// accounts. An API key sends `tokentype: API_KEY` and never a `profileArn`
/// (AWS answers 403 otherwise); an OAuth account sends neither header here.
async fn fetch_usage_body(
    client: &reqwest::Client,
    credential: &Credential,
) -> Result<Value, ProviderError> {
    let extra = credential.extra.as_ref();
    let req = client
        .post(format!("{}/", control_plane_base(credential)))
        .header("content-type", "application/x-amz-json-1.0")
        .header("accept", "application/json")
        .header("x-amz-target", USAGE_TARGET);
    let req = authorize(req, credential, auth_method(extra));

    let resp = req
        .json(&serde_json::json!({}))
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
            "GetUsageLimits returned {}: {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        )));
    }
    serde_json::from_str(&text).map_err(|e| ProviderError::Http(format!("usage body: {e}")))
}

/// Map a `GetUsageLimits` body to a [`UsageSnapshot`].
///
/// Fails with [`ProviderError::Http`] when the body has no `CREDIT` line, so the
/// caller renders "unavailable" instead of a fabricated zero.
pub fn parse_usage(body: &Value) -> Result<UsageSnapshot, ProviderError> {
    let credit = body
        .get("usageBreakdownList")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|e| e.get("resourceType").and_then(Value::as_str) == Some(CREDIT_RESOURCE))
        })
        .ok_or_else(|| ProviderError::Http("GetUsageLimits carried no CREDIT resource".into()))?;

    // `currentUsageWithPrecision` is the fractional period-to-date total. Its
    // integer sibling `currentUsage` truncates, so prefer the precise one.
    let credits_used = credit
        .get("currentUsageWithPrecision")
        .and_then(Value::as_f64)
        .or_else(|| credit.get("currentUsage").and_then(Value::as_f64))
        .ok_or_else(|| ProviderError::Http("credit line carried no usage".into()))?;

    // The plan allowance. Deliberately NOT `overageCap*`: that is the paid
    // overage ceiling and can differ from the plan limit.
    let credits_limit = credit
        .get("usageLimitWithPrecision")
        .and_then(Value::as_f64)
        .or_else(|| credit.get("usageLimit").and_then(Value::as_f64));

    let period_end_secs = credit
        .get("nextDateReset")
        .or_else(|| body.get("nextDateReset"))
        .and_then(Value::as_f64);
    #[allow(clippy::cast_possible_truncation)] // Unix timestamp seconds always fit in i64
    let credits_period_end = period_end_secs.map(|s| s as i64);

    let plan = body
        .get("subscriptionInfo")
        .and_then(|s| s.get("subscriptionTitle"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let upstream_user_ref = body
        .get("userInfo")
        .and_then(|u| u.get("userId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(fingerprint);

    Ok(UsageSnapshot {
        credits_used: Some(credits_used),
        credits_limit,
        credits_period_end,
        windows: Vec::new(),
        plan,
        upstream_user_ref,
    })
}

/// Short, non-reversible fingerprint of an upstream user id.
///
/// The raw `userId` is an account identifier; only a hash prefix leaves this
/// process, enough to tell two accounts apart without exposing the id itself.
fn fingerprint(user_id: &str) -> String {
    let digest = Sha256::digest(user_id.as_bytes());
    format!("uid#{}", hex8(&digest))
}

fn hex8(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(4)
        .fold(String::with_capacity(8), |mut s, b| {
            write!(s, "{b:02x}").expect("infallible");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real sanitized bodies captured from the live service (2026-09).
    // Two DIFFERENT plans with DIFFERENT limits: a shared-cache or swapped-
    // credential bug would make these two collapse to one reading.
    fn kiro_power() -> Value {
        serde_json::json!({
            "daysUntilReset": 0,
            "limits": [],
            "nextDateReset": 1_790_812_800.0,
            "overageConfiguration": { "overageStatus": "DISABLED" },
            "subscriptionInfo": {
                "subscriptionTitle": "KIRO POWER",
                "type": "Q_DEVELOPER_STANDALONE_POWER"
            },
            "usageBreakdownList": [{
                "currentUsage": 142,
                "currentUsageWithPrecision": 142.84,
                "overageCap": 10000,
                "overageCapWithPrecision": 10000.0,
                "overageRate": 0.04,
                "resourceType": "CREDIT",
                "unit": "INVOCATIONS",
                "usageLimit": 10000,
                "usageLimitWithPrecision": 10000.0
            }],
            "userInfo": {}
        })
    }

    fn kiro_pro_max() -> Value {
        serde_json::json!({
            "daysUntilReset": 0,
            "limits": [],
            "nextDateReset": 1_790_812_800.0,
            "overageConfiguration": { "overageStatus": "DISABLED" },
            "subscriptionInfo": {
                "subscriptionTitle": "KIRO PRO MAX",
                "type": "Q_DEVELOPER_STANDALONE_PRO_MAX"
            },
            "usageBreakdownList": [{
                "currentUsage": 2285,
                "currentUsageWithPrecision": 2285.75,
                "overageCap": 10000,
                "overageCapWithPrecision": 10000.0,
                "overageRate": 0.04,
                "resourceType": "CREDIT",
                "unit": "INVOCATIONS",
                "usageLimit": 5000,
                "usageLimitWithPrecision": 5000.0
            }],
            "userInfo": { "userId": "d-9067c98495.b4482408-a011-703b-a535-d5cd9bb0d646" }
        })
    }

    // Refutes reading the wrong field for the plan allowance: `overageCap` is
    // 10000 for BOTH plans, so a parser that used it would report the same limit
    // for POWER and PRO MAX and hide the real 10000-vs-5000 difference.
    #[test]
    fn credit_limit_is_the_plan_allowance_not_the_overage_cap() {
        let power = parse_usage(&kiro_power()).unwrap();
        let pro_max = parse_usage(&kiro_pro_max()).unwrap();

        assert_eq!(power.credits_limit, Some(10000.0));
        assert_eq!(pro_max.credits_limit, Some(5000.0));
        assert_ne!(
            power.credits_limit, pro_max.credits_limit,
            "distinct plans must report distinct limits"
        );
    }

    // Refutes truncating usage to the integer `currentUsage` and dropping the
    // plan / period fields.
    #[test]
    fn parses_used_plan_and_period_from_real_body() {
        let s = parse_usage(&kiro_pro_max()).unwrap();
        assert_eq!(s.credits_used, Some(2285.75));
        assert!(s.windows.is_empty(), "Kiro meters credits, not windows");
        assert_eq!(s.plan.as_deref(), Some("KIRO PRO MAX"));
        assert_eq!(s.credits_period_end, Some(1_790_812_800));
    }

    // Refutes leaking the raw userId, and refutes a constant/empty fingerprint:
    // the two distinct upstream ids must map to two distinct, non-reversible
    // references, and an absent id must stay absent (never fabricated).
    #[test]
    fn user_ref_is_a_stable_nonreversible_fingerprint_or_absent() {
        let raw = "d-9067c98495.b4482408-a011-703b-a535-d5cd9bb0d646";
        let pro_max = parse_usage(&kiro_pro_max()).unwrap();
        let user_ref = pro_max
            .upstream_user_ref
            .expect("PRO MAX reported a userId");
        assert!(!user_ref.contains(raw), "raw userId must never be exposed");
        assert_eq!(user_ref, fingerprint(raw), "fingerprint must be stable");
        assert_ne!(user_ref, fingerprint("some-other-user"));

        // POWER account body carries no userId: stays None, never a fake value.
        assert_eq!(parse_usage(&kiro_power()).unwrap().upstream_user_ref, None);
    }

    // Refutes turning a missing credit line into a fabricated `0`: absence is an
    // error the caller renders "unavailable".
    #[test]
    fn missing_credit_line_is_an_error_not_zero() {
        let no_credit = serde_json::json!({
            "usageBreakdownList": [{ "resourceType": "STORAGE", "currentUsage": 5 }],
            "subscriptionInfo": { "subscriptionTitle": "KIRO POWER" }
        });
        assert!(parse_usage(&no_credit).is_err());
        assert!(parse_usage(&serde_json::json!({})).is_err());
    }

    // A plan with allowance omitted (hypothetical) must leave the limit unknown,
    // not zero, while still reporting the usage it did send.
    #[test]
    fn absent_limit_stays_unknown_but_usage_is_kept() {
        let body = serde_json::json!({
            "usageBreakdownList": [{
                "resourceType": "CREDIT",
                "currentUsageWithPrecision": 12.5
            }]
        });
        let s = parse_usage(&body).unwrap();
        assert_eq!(s.credits_used, Some(12.5));
        assert_eq!(s.credits_limit, None);
        assert_eq!(s.credits_period_end, None);
    }
}
