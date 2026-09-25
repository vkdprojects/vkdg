/// Metrics produced by a compression operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionMetrics {
    pub original_message_count: usize,
    pub compressed_message_count: usize,
    pub estimated_tokens_removed: u32,
    pub strategy: String,
    /// True if no messages were removed (budget already satisfied).
    pub lossless: bool,
}
