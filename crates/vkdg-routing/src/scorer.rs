//! Multi-factor connection scorer for automatic routing.
//!
//! Each candidate receives a score in [0.0, 1.0] based on weighted factors.
//! The default mode pack is "balanced" (equal weights).
//! Mode packs are selected per-request via combo config or X-VKDG-Mode header.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub health:      f32,
    pub quota:       f32,
    pub cost_inv:    f32,
    pub latency_inv: f32,
    pub stability:   f32,
}

impl ScoringWeights {
    pub fn balanced() -> Self {
        Self { health: 0.2, quota: 0.2, cost_inv: 0.2, latency_inv: 0.2, stability: 0.2 }
    }
    pub fn ship_fast() -> Self {
        Self { health: 0.3, quota: 0.1, cost_inv: 0.05, latency_inv: 0.5, stability: 0.05 }
    }
    pub fn cost_saver() -> Self {
        Self { health: 0.1, quota: 0.2, cost_inv: 0.6, latency_inv: 0.05, stability: 0.05 }
    }
    pub fn quality_first() -> Self {
        Self { health: 0.4, quota: 0.2, cost_inv: 0.05, latency_inv: 0.05, stability: 0.3 }
    }
    pub fn offline_friendly() -> Self {
        Self { health: 0.5, quota: 0.1, cost_inv: 0.0, latency_inv: 0.0, stability: 0.4 }
    }
    pub fn from_mode_pack(mode: &str) -> Self {
        match mode {
            "ship-fast"        => Self::ship_fast(),
            "cost-saver"       => Self::cost_saver(),
            "quality-first"    => Self::quality_first(),
            "offline-friendly" => Self::offline_friendly(),
            _                  => Self::balanced(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CandidateSignals {
    pub connection_id: String,
    /// 0.0 = degraded, 1.0 = perfectly healthy
    pub health: f32,
    /// tokens remaining as fraction of limit; None = unknown (treated as 0.5)
    pub quota_headroom: Option<f32>,
    /// cost per 1k tokens in microdollars; None = unknown (treated as 500)
    pub cost_per_ktoken: Option<u32>,
    /// EWMA p50 latency in ms; None = unknown (treated as 200ms)
    pub latency_p50_ms: Option<u32>,
    /// number of recent circuit-open or cooldown events; 0 = perfectly stable
    pub instability_events: u32,
}

impl CandidateSignals {
    pub fn score(&self, w: &ScoringWeights) -> f32 {
        let health = self.health.clamp(0.0, 1.0);
        let quota  = self.quota_headroom.unwrap_or(0.5).clamp(0.0, 1.0);
        // cost: lower is better; invert and normalize
        let cost_inv = {
            let c = self.cost_per_ktoken.unwrap_or(500) as f32;
            // normalize against 100 µ$/ktoken reference so cheap vs expensive spans [0,1] usefully
            (1.0 / (1.0 + c / 100.0)).clamp(0.0, 1.0)
        };
        // latency: lower is better; sigmoid-style inversion over 5000ms range
        let lat_inv = {
            let l = self.latency_p50_ms.unwrap_or(200) as f32;
            (5000.0 / (l + 5000.0)).clamp(0.0, 1.0)
        };
        // stability: decay instability events
        let stability = 1.0 / (1.0 + self.instability_events as f32);

        health    * w.health
            + quota     * w.quota
            + cost_inv  * w.cost_inv
            + lat_inv   * w.latency_inv
            + stability * w.stability
    }
}

/// Sort candidates by score descending. Returns sorted Vec of connection IDs.
pub fn rank_candidates(candidates: &[CandidateSignals], weights: &ScoringWeights) -> Vec<String> {
    let mut scored: Vec<(&CandidateSignals, f32)> =
        candidates.iter().map(|c| (c, c.score(weights))).collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(c, _)| c.connection_id.clone()).collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: degraded candidate ranked above healthy one
    #[test]
    fn healthy_ranks_above_degraded() {
        let w = ScoringWeights::balanced();
        let good = CandidateSignals {
            connection_id: "good".into(), health: 1.0,
            quota_headroom: Some(0.8), cost_per_ktoken: Some(100),
            latency_p50_ms: Some(100), instability_events: 0,
        };
        let bad = CandidateSignals {
            connection_id: "bad".into(), health: 0.1,
            quota_headroom: Some(0.1), cost_per_ktoken: Some(9000),
            latency_p50_ms: Some(4000), instability_events: 10,
        };
        let ranked = rank_candidates(&[bad, good], &w);
        assert_eq!(ranked[0], "good", "healthy candidate must rank first");
    }

    // Plausible wrong impl: cost-saver mode still picks expensive connection
    #[test]
    fn cost_saver_mode_prefers_cheap() {
        let w = ScoringWeights::cost_saver();
        let cheap = CandidateSignals {
            connection_id: "cheap".into(), health: 0.9,
            quota_headroom: Some(0.5), cost_per_ktoken: Some(10),
            latency_p50_ms: Some(300), instability_events: 0,
        };
        let expensive = CandidateSignals {
            connection_id: "expensive".into(), health: 1.0,
            quota_headroom: Some(0.9), cost_per_ktoken: Some(5000),
            latency_p50_ms: Some(50), instability_events: 0,
        };
        let ranked = rank_candidates(&[expensive, cheap], &w);
        assert_eq!(ranked[0], "cheap", "cost-saver must prefer cheap connection");
    }

    // Plausible wrong impl: ship-fast mode ignores latency
    #[test]
    fn ship_fast_mode_prefers_low_latency() {
        let w = ScoringWeights::ship_fast();
        let fast = CandidateSignals {
            connection_id: "fast".into(), health: 0.9,
            quota_headroom: Some(0.5), cost_per_ktoken: Some(1000),
            latency_p50_ms: Some(20), instability_events: 0,
        };
        let slow = CandidateSignals {
            connection_id: "slow".into(), health: 1.0,
            quota_headroom: Some(0.9), cost_per_ktoken: Some(10),
            latency_p50_ms: Some(3000), instability_events: 0,
        };
        let ranked = rank_candidates(&[slow, fast], &w);
        assert_eq!(ranked[0], "fast", "ship-fast must prefer low-latency connection");
    }

    // Plausible wrong impl: unknown signals treated as 0 instead of neutral
    #[test]
    fn unknown_signals_treated_neutrally() {
        let w = ScoringWeights::balanced();
        let unknown = CandidateSignals {
            connection_id: "x".into(), health: 0.5,
            quota_headroom: None, cost_per_ktoken: None,
            latency_p50_ms: None, instability_events: 0,
        };
        let score = unknown.score(&w);
        assert!(score > 0.0 && score < 1.0, "unknown signals must yield mid-range score, got {score}");
    }

    // Plausible wrong impl: from_mode_pack returns balanced for all inputs
    #[test]
    fn mode_pack_names_map_correctly() {
        let modes = ["ship-fast", "cost-saver", "quality-first", "offline-friendly", "balanced"];
        let weights: Vec<ScoringWeights> =
            modes.iter().map(|m| ScoringWeights::from_mode_pack(m)).collect();
        assert!(
            weights[0].latency_inv > weights[1].latency_inv,
            "ship-fast must weight latency more than cost-saver"
        );
        assert!(
            weights[1].cost_inv > weights[0].cost_inv,
            "cost-saver must weight cost more than ship-fast"
        );
    }
}
