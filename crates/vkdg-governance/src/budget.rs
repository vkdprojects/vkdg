//! Budget enforcement helpers.

use crate::key::VirtualKey;

/// Rough token cost estimate in microdollars.
///
/// Claude 3.5 Haiku: $0.80/M input → ~0.8 µ$ per token.
/// We use $1/M input and $3/M output as conservative estimates
/// until the provider returns exact usage.
pub const MICRODOLLARS_PER_TOKEN_INPUT: u64 = 1; // $1/M
pub const MICRODOLLARS_PER_TOKEN_OUTPUT: u64 = 3; // $3/M

/// Pre-flight budget check: can this key afford an estimated number of input tokens?
///
/// Returns `Err` with a human-readable message if the estimated cost exceeds
/// the key's remaining budget.
pub fn preflight_check(key: &VirtualKey, estimated_input_tokens: u32) -> Result<(), String> {
    let estimated_cost = u64::from(estimated_input_tokens) * MICRODOLLARS_PER_TOKEN_INPUT;
    if let Some(remaining) = key.remaining_budget() {
        if estimated_cost > remaining {
            return Err(format!(
                "budget exceeded: estimated cost {estimated_cost} microdollars, \
                 remaining {remaining} microdollars",
            ));
        }
    }
    Ok(())
}

/// Stateless helper that wraps [`preflight_check`].
pub struct BudgetChecker;

impl BudgetChecker {
    pub fn check(key: &VirtualKey, estimated_tokens: u32) -> Result<(), String> {
        preflight_check(key, estimated_tokens)
    }
}
