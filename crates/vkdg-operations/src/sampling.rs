//! Request-side sampling parameters every ingress validates the same way.
//!
//! A client value is either carried to the provider as sent or rejected with the
//! field named; nothing is clamped or defaulted on the way.

use vkdg_core::VkdgError;

/// Most stop strings one request may carry. `OpenAI` documents 4; Anthropic
/// documents no number. 16 leaves room for client quirks while keeping a hostile
/// array from being copied into every attempt's upstream body.
pub const MAX_STOP_SEQUENCES: usize = 16;

/// Longest stop string in bytes. Real stop strings are a token or a delimiter.
pub const MAX_STOP_SEQUENCE_BYTES: usize = 256;

/// Checks client stop strings and drops empty ones (an empty string can never
/// match, and Anthropic rejects it).
///
/// `field` names the wire field (`stop`, `stop_sequences`) in the error.
///
/// # Errors
/// `ConfigInvalid { field }` when more than [`MAX_STOP_SEQUENCES`] non-empty
/// strings are given or one exceeds [`MAX_STOP_SEQUENCE_BYTES`].
pub fn validated_stop_sequences(field: &str, raw: Vec<String>) -> Result<Vec<String>, VkdgError> {
    let invalid = |message: String| VkdgError::ConfigInvalid {
        field: field.to_owned(),
        message,
    };
    let kept: Vec<String> = raw.into_iter().filter(|s| !s.is_empty()).collect();
    if kept.len() > MAX_STOP_SEQUENCES {
        return Err(invalid(format!(
            "at most {MAX_STOP_SEQUENCES} stop sequences are accepted, got {}",
            kept.len()
        )));
    }
    if let Some(long) = kept.iter().find(|s| s.len() > MAX_STOP_SEQUENCE_BYTES) {
        return Err(invalid(format!(
            "a stop sequence is {} bytes; the limit is {MAX_STOP_SEQUENCE_BYTES}",
            long.len()
        )));
    }
    Ok(kept)
}

/// Checks a client `top_p`: finite and within `0.0..=1.0`.
///
/// # Errors
/// `ConfigInvalid { field: "top_p" }` outside that range or for NaN/infinity.
pub fn validated_top_p(top_p: Option<f32>) -> Result<Option<f32>, VkdgError> {
    match top_p {
        Some(p) if !(0.0..=1.0).contains(&p) => Err(VkdgError::ConfigInvalid {
            field: "top_p".into(),
            message: format!("must be between 0 and 1, got {p}"),
        }),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field_of(err: &VkdgError) -> &str {
        match err {
            VkdgError::ConfigInvalid { field, .. } => field,
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    // Defeat: copying an unbounded stop array into every upstream attempt, or
    // truncating it silently so the model runs past a stop the client set.
    #[test]
    fn stop_sequences_over_the_limits_are_rejected_not_truncated() {
        let many: Vec<String> = (0..=MAX_STOP_SEQUENCES).map(|i| format!("s{i}")).collect();
        assert_eq!(
            field_of(&validated_stop_sequences("stop", many).unwrap_err()),
            "stop"
        );
        let long = vec!["x".repeat(MAX_STOP_SEQUENCE_BYTES + 1)];
        assert_eq!(
            field_of(&validated_stop_sequences("stop_sequences", long).unwrap_err()),
            "stop_sequences"
        );
        let exactly: Vec<String> = (0..MAX_STOP_SEQUENCES).map(|i| format!("s{i}")).collect();
        assert_eq!(
            validated_stop_sequences("stop", exactly.clone()).unwrap(),
            exactly
        );
    }

    // Defeat: forwarding `""` (Anthropic answers 400) or counting empties
    // against the cap so a client sending padding is rejected.
    #[test]
    fn empty_stop_strings_are_dropped_and_do_not_count() {
        let mut raw = vec![String::new(); MAX_STOP_SEQUENCES + 5];
        raw.push("END".into());
        assert_eq!(validated_stop_sequences("stop", raw).unwrap(), vec!["END"]);
    }

    // Defeat: passing NaN or out-of-range nucleus mass through to a provider
    // that answers 400 only after routing.
    #[test]
    fn top_p_outside_zero_to_one_is_rejected() {
        for bad in [-0.1, 1.01, f32::NAN, f32::INFINITY] {
            assert_eq!(field_of(&validated_top_p(Some(bad)).unwrap_err()), "top_p");
        }
        for ok in [0.0, 0.95, 1.0] {
            assert_eq!(validated_top_p(Some(ok)).unwrap(), Some(ok));
        }
        assert_eq!(validated_top_p(None).unwrap(), None);
    }
}
