//! Provider account credit / quota usage.
//!
//! A provider that sells credits (Kiro's Q Developer plans, for one) can report
//! how many its account has spent in the current period and what its plan
//! allowance is. The gateway only ever surfaces provider-reported figures: a
//! missing value becomes "unavailable", never a fabricated `0`.

/// One account's credit usage for the current billing period.
///
/// Every field is exactly what the provider reported. `credits_limit` is the
/// *plan* allowance, never the paid overage ceiling — they can be equal for one
/// plan and wildly different for another, so a caller must not substitute one
/// for the other.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    /// Credits consumed this period (provider-reported).
    pub credits_used: f64,
    /// Plan credit allowance. `None` when the provider does not report one; the
    /// caller surfaces "unavailable" rather than inventing a number.
    pub credits_limit: Option<f64>,
    /// Unix seconds when the credit period resets.
    pub credits_period_end: Option<i64>,
    /// Human-readable plan name; varies per account (e.g. "KIRO POWER").
    pub plan: Option<String>,
    /// Opaque, non-reversible fingerprint of the upstream user identity, when the
    /// provider reports one. Lets an operator spot two gateway accounts that
    /// resolve to the same upstream user without ever exposing the raw id.
    pub upstream_user_ref: Option<String>,
}

/// A provider that can report an account's credit usage.
///
/// Returned from [`ProviderAdapter::usage`](crate::ProviderAdapter::usage). The
/// call is made with a single account's credential and MUST NOT reuse a shared
/// key: each account's usage belongs to that account alone.
pub trait UsageProvider: Send + Sync {
    /// Query the provider for this account's current-period usage.
    ///
    /// Errors (HTTP failure, unparseable body, no credit line) surface as an
    /// error the caller renders as "unavailable"; they never affect routing.
    fn fetch_usage<'a>(
        &'a self,
        credential: &'a vkdg_connections::Credential,
    ) -> futures::future::BoxFuture<'a, Result<UsageSnapshot, crate::ProviderError>>;
}
