use chrono::Utc;
use uuid::Uuid;
use crate::result::{EvalResult, EvalStatus};

#[derive(Debug, Clone)]
pub struct LatencyMetrics {
    pub latency_ms: u32,
    pub ttft_ms: Option<u32>,
    pub token_count: u32,
}

pub struct EvalScorer;

impl EvalScorer {
    /// Score a response based on latency and content quality.
    /// Returns an EvalResult with a score in [0.0, 1.0].
    pub fn score(
        request_id: &str,
        connection_id: &str,
        response_body: &str,
        metrics: LatencyMetrics,
    ) -> EvalResult {
        let mut notes = Vec::new();
        let mut score = 1.0f32;

        // Check 1: non-empty response
        if response_body.trim().is_empty() {
            notes.push("empty response".into());
            score -= 0.5;
        }

        // Check 2: no error patterns in body
        let body_lower = response_body.to_lowercase();
        if body_lower.contains("\"type\":\"error\"") {
            notes.push("error in response body".into());
            score -= 0.4;
        }

        // Check 3: latency score (penalize > 5000ms)
        if metrics.latency_ms > 5000 {
            let penalty = ((metrics.latency_ms as f32 - 5000.0) / 10000.0).min(0.3);
            notes.push(format!("high latency: {}ms", metrics.latency_ms));
            score -= penalty;
        }

        // Check 4: TTFT score (penalize > 2000ms)
        if let Some(ttft) = metrics.ttft_ms {
            if ttft > 2000 {
                notes.push(format!("slow TTFT: {}ms", ttft));
                score -= 0.1;
            }
        }

        let score = score.clamp(0.0, 1.0);
        let status = if score >= 0.7 { EvalStatus::Pass } else { EvalStatus::Fail };

        EvalResult {
            eval_id: Uuid::new_v4(),
            request_id: request_id.into(),
            connection_id: connection_id.into(),
            status,
            score,
            latency_ms: metrics.latency_ms,
            ttft_ms: metrics.ttft_ms,
            token_count: metrics.token_count,
            notes,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(latency_ms: u32) -> LatencyMetrics {
        LatencyMetrics { latency_ms, ttft_ms: None, token_count: 50 }
    }

    // Plausible wrong impl: empty response returns Pass
    #[test]
    fn empty_response_fails() {
        let r = EvalScorer::score("req-1", "conn-1", "", metrics(100));
        assert_eq!(r.status, EvalStatus::Fail);
        assert!(r.notes.iter().any(|n| n.contains("empty")));
    }

    // Plausible wrong impl: error body returns Pass
    #[test]
    fn error_body_fails() {
        let body = r#"{"type":"error","error":{"message":"rate limited"}}"#;
        let r = EvalScorer::score("req-1", "conn-1", body, metrics(100));
        assert!(r.score < 0.7, "error body must reduce score: {}", r.score);
    }

    // Plausible wrong impl: high latency response passes with perfect score
    #[test]
    fn high_latency_penalizes_score() {
        let r_fast = EvalScorer::score("r", "c", "hello", metrics(100));
        let r_slow = EvalScorer::score("r", "c", "hello", metrics(8000));
        assert!(r_slow.score < r_fast.score, "slow response must score lower");
    }

    // Plausible wrong impl: good response fails
    #[test]
    fn good_response_passes() {
        let r = EvalScorer::score("r", "c", "The answer is 42.", metrics(150));
        assert_eq!(r.status, EvalStatus::Pass);
        assert!(r.score >= 0.7);
    }
}
