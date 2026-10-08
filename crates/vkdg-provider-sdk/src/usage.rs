//! Provider account usage: credit balances and rate-limit windows.
//!
//! A provider reports what its account has spent in the current period: Kiro's
//! Q Developer plans sell credits, Claude Code and Codex enforce rolling
//! rate-limit windows (5 hours, 7 days). The gateway only ever surfaces
//! provider-reported figures: a missing value becomes "unavailable", never a
//! fabricated `0`.

/// Which rate-limit window a [`UsageWindow`] measures.
///
/// Providers report only the windows their plan enforces, so a snapshot carries
/// exactly the kinds the upstream sent — never a placeholder for an absent one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// Rolling five-hour window.
    FiveHour,
    /// Rolling seven-day window across all models.
    Weekly,
    /// Seven-day window that applies to Sonnet models only.
    WeeklySonnet,
    /// Seven-day window that applies to Opus models only.
    WeeklyOpus,
}

impl WindowKind {
    /// Stable wire identifier used by the admin API.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FiveHour => "five_hour",
            Self::Weekly => "weekly",
            Self::WeeklySonnet => "weekly_sonnet",
            Self::WeeklyOpus => "weekly_opus",
        }
    }
}

/// One rate-limit window as the provider reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub kind: WindowKind,
    /// Share of the window already consumed, as the provider reported it
    /// (nominally `0..=100`; not clamped here).
    pub used_percent: f64,
    /// Unix seconds when the window rolls over, when reported.
    pub resets_at: Option<i64>,
}

/// One account's usage: credits for the current billing period and/or
/// rate-limit windows.
///
/// Every field is exactly what the provider reported. `credits_limit` is the
/// *plan* allowance, never the paid overage ceiling — they can be equal for one
/// plan and wildly different for another, so a caller must not substitute one
/// for the other.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageSnapshot {
    /// Credits consumed this period (provider-reported). `None` for providers
    /// that meter by window rather than by credit.
    pub credits_used: Option<f64>,
    /// Plan credit allowance. `None` when the provider does not report one; the
    /// caller surfaces "unavailable" rather than inventing a number.
    pub credits_limit: Option<f64>,
    /// Unix seconds when the credit period resets.
    pub credits_period_end: Option<i64>,
    /// Rate-limit windows the plan enforces; empty for credit-only providers.
    pub windows: Vec<UsageWindow>,
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
