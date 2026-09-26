//! Lightweight eval framework for VKDG.
//!
//! Runs quality evaluations on gateway responses:
//! - Latency scoring: TTFT, completion time, tokens/sec
//! - Content quality: simple heuristics (not empty, not truncated, no error patterns)
//! - Consistency: same prompt produces consistent outputs across connections
//!
//! Phase E: LLM-judge evaluation, A/B testing, combo target health scores.

pub mod result;
pub mod scorer;

pub use result::{EvalResult, EvalStatus};
pub use scorer::{EvalScorer, LatencyMetrics};
