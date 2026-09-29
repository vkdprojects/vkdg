//! Per-model token prices, declared by provider plugins.
//!
//! Prices are integer microdollars per million tokens so cost math stays exact
//! and never touches floats. A provider billed by subscription (Kiro, Claude
//! Code) declares no prices: its requests have no cost, which is reported as
//! "unknown", never as zero.

use std::borrow::Cow;

/// Price of one model family, in microdollars per million tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPrice {
    /// Model-name pattern (`claude-sonnet-4*`), matched with [`crate::glob`]
    /// against the model name with `.` read as `-` (see [`price_for`]).
    pub pattern: Cow<'static, str>,
    pub input_per_mtok: u64,
    pub output_per_mtok: u64,
}

/// Tokens a response reported, as the usage meter reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BilledTokens {
    /// All input tokens, cached ones included.
    pub input: u64,
    /// Part of `input` read from the prompt cache.
    pub cache_read: u64,
    /// Part of `input` written to the prompt cache.
    pub cache_write: u64,
    pub output: u64,
}

impl ModelPrice {
    /// Prices in microdollars per million tokens: `$3/M` is `3_000_000`.
    pub const fn new(pattern: &'static str, input_per_mtok: u64, output_per_mtok: u64) -> Self {
        Self {
            pattern: Cow::Borrowed(pattern),
            input_per_mtok,
            output_per_mtok,
        }
    }

    /// Cost of a response in microdollars, rounded up so a tiny request is
    /// never free. Cache reads bill at 0.1x and cache writes at 1.25x the
    /// input price (Anthropic's rates). Providers that do not report cache
    /// tokens separately are billed at the full input price: an upper bound.
    pub fn cost_microdollars(&self, t: BilledTokens) -> u64 {
        let input = u128::from(self.input_per_mtok);
        let fresh = t.input.saturating_sub(t.cache_read + t.cache_write);
        // Scaled by 20 so 0.1x (2/20) and 1.25x (25/20) stay integers.
        let scaled = u128::from(fresh) * input * 20
            + u128::from(t.cache_read) * input * 2
            + u128::from(t.cache_write) * input * 25
            + u128::from(t.output) * u128::from(self.output_per_mtok) * 20;
        u64::try_from(scaled.div_ceil(20_000_000)).unwrap_or(u64::MAX)
    }
}

/// First entry whose pattern matches `model`. Tables list specific patterns
/// before general ones. Clients spell versions both ways (`claude-sonnet-4.5`,
/// `claude-sonnet-4-5`), so `.` in the model name is read as `-`; patterns
/// use the dash form.
pub fn price_for<'a>(table: &'a [ModelPrice], model: &str) -> Option<&'a ModelPrice> {
    let model = model.replace('.', "-");
    table
        .iter()
        .find(|p| crate::glob::matches(&p.pattern, &model))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64) -> BilledTokens {
        BilledTokens {
            input,
            output,
            ..BilledTokens::default()
        }
    }

    #[test]
    fn dotted_and_dashed_names_find_the_same_price() {
        const TABLE: &[ModelPrice] = &[
            ModelPrice::new("claude-opus-4-1*", 15_000_000, 75_000_000),
            ModelPrice::new("claude-opus-4-*", 5_000_000, 25_000_000),
        ];
        let p = |m| price_for(TABLE, m).map(|p| p.input_per_mtok);
        assert_eq!(p("claude-opus-4.5"), Some(5_000_000));
        assert_eq!(p("claude-opus-4-5"), Some(5_000_000));
        assert_eq!(p("claude-opus-4.1"), Some(15_000_000));
        assert_eq!(p("gpt-4o"), None);
    }

    #[test]
    fn cost_is_exact_and_rounds_up() {
        let p = ModelPrice::new("m", 3_000_000, 15_000_000);
        // 1M in + 1M out = $18.
        assert_eq!(
            p.cost_microdollars(tokens(1_000_000, 1_000_000)),
            18_000_000
        );
        // 4107 in, 1 out = 12321 + 15 µ$.
        assert_eq!(p.cost_microdollars(tokens(4107, 1)), 12_336);
        // One token at $0.15/M is 0.15 µ$: charged as 1, not 0.
        assert_eq!(
            ModelPrice::new("m", 150_000, 0).cost_microdollars(tokens(1, 0)),
            1
        );
        assert_eq!(p.cost_microdollars(tokens(0, 0)), 0);
    }

    // Billing cached input at the full rate overcharges prompt-cached traffic ~10x.
    #[test]
    fn cache_reads_and_writes_bill_at_their_own_rates() {
        let p = ModelPrice::new("m", 3_000_000, 15_000_000);
        let t = BilledTokens {
            input: 1_000_000 + 1_000_000 + 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            output: 0,
        };
        // fresh $3 + read $0.30 + write $3.75.
        assert_eq!(p.cost_microdollars(t), 7_050_000);
    }
}
