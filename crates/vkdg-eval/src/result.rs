use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EvalStatus {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub eval_id: Uuid,
    pub request_id: String,
    pub connection_id: String,
    pub status: EvalStatus,
    pub score: f32,          // 0.0 = worst, 1.0 = perfect
    pub latency_ms: u32,
    pub ttft_ms: Option<u32>,
    pub token_count: u32,
    pub notes: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
