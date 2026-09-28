use serde::{Deserialize, Serialize};
use vkdg_core::{RequestId, VkdgError};

// ── Operation family ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationFamily {
    ConversationGenerate,
    EmbeddingCreate,
    ImageGenerate,
    ImageEdit,
    AudioTranscribe,
    AudioSynthesize,
    VideoGenerate,
    VideoRemix,
}

// ── Capabilities ──────────────────────────────────────────────────────────────
// Defined in vkdg-core so connections (which advertise capabilities) and
// operations (which require them) share the same type without circular dep.
pub use vkdg_core::{Capability, CapabilitySet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SupportLevel {
    Supported,
    SupportedWithDeclaredLoss { losses: Vec<String> },
    Unsupported { reason: String },
}

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageData {
    Base64 { data: String },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: ImageData,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
    Thinking {
        thinking: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

// ── Tool ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

// ── Conversation request ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRequest {
    /// Model the client asked for, as written on the wire.
    ///
    /// A connection's `models` list holds route patterns (`claude-*`), not model
    /// ids, so an adapter that read it would send a glob upstream. This carries the
    /// actual request through to `prepare`.
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub system: Option<String>,
    pub required_capabilities: CapabilitySet,
    /// Extended reasoning the client asked for (Anthropic `thinking`, OpenAI
    /// `reasoning_effort`). `None` = the client did not ask.
    #[serde(default)]
    pub thinking: Option<ThinkingRequest>,
}

/// A client's reasoning request, kept in the client's own terms: a token
/// budget (Anthropic) or an effort level (OpenAI). Adapters map it to their
/// provider's knob.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingRequest {
    pub budget_tokens: Option<u32>,
    /// `low` | `medium` | `high` (and `minimal`/`xhigh`/`max` where sent).
    pub effort: Option<String>,
}

impl ThinkingRequest {
    /// Effort level, from the explicit effort or a budget (OmniRoute's mapping:
    /// ≥32k high, ≥16k medium, >0 low). `minimal` counts as `low`.
    pub fn effort_level(&self) -> &str {
        match self.effort.as_deref() {
            Some("minimal") => "low",
            Some(e) if !e.is_empty() => e,
            _ => match self.budget_tokens.unwrap_or(0) {
                b if b >= 32_000 => "high",
                b if b >= 16_000 => "medium",
                _ => "low",
            },
        }
    }
}

// ── Operation ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingCreateRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerateRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub n: Option<u32>,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub style: Option<String>,
    pub response_format: Option<String>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEditRequest {
    pub prompt: String,
    pub image: ImageData,
    pub mask: Option<ImageData>,
    pub model: Option<String>,
    pub n: Option<u32>,
    pub size: Option<String>,
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageOutput {
    pub url: Option<String>,
    pub b64_json: Option<String>,
    pub revised_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageResponse {
    pub request_id: RequestId,
    pub created: u64,
    pub data: Vec<ImageOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTranscribeRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSynthesizeRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenerateRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub duration_secs: Option<u32>,
    pub resolution: Option<String>,
    pub style: Option<String>,
    pub reference_image: Option<ImageData>,
    pub audio: Option<bool>,
    pub webhook_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoRemixRequest {
    pub source: ImageData,
    pub prompt: String,
    pub model: Option<String>,
    pub duration_secs: Option<u32>,
    pub webhook_url: Option<String>,
}

/// Status event for a video job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VideoJobEvent {
    Queued {
        job_id: String,
    },
    Running {
        job_id: String,
        progress_pct: Option<u8>,
    },
    Succeeded {
        job_id: String,
        artifact_url: String,
    },
    Failed {
        job_id: String,
        reason: String,
    },
    Cancelled {
        job_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    Conversation(ConversationRequest),
    EmbeddingCreate(EmbeddingCreateRequest),
    ImageGenerate(ImageGenerateRequest),
    ImageEdit(ImageEditRequest),
    AudioTranscribe(AudioTranscribeRequest),
    AudioSynthesize(AudioSynthesizeRequest),
    VideoGenerate(VideoGenerateRequest),
    VideoRemix(VideoRemixRequest),
}

// ── Stream events ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UsageCount {
    Reported(u32),
    Estimated(u32),
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationEvent {
    Started {
        request_id: RequestId,
    },
    OutputDelta {
        delta: String,
        index: u32,
    },
    /// Model reasoning ("thinking") text; never mixed into the visible answer.
    ReasoningDelta {
        delta: String,
        index: u32,
    },
    /// A tool-call fragment. The first event for an `index` carries `name` and
    /// `tool_use_id`; later ones carry `input_delta` (JSON text) only.
    ToolCallDelta {
        tool_use_id: String,
        name: String,
        input_delta: String,
        index: u32,
    },
    /// The tool call at `index` is complete (its input will not grow).
    ToolCallEnd {
        index: u32,
    },
    Usage {
        input_tokens: UsageCount,
        output_tokens: UsageCount,
        /// Prompt tokens served from the provider's prompt cache.
        cache_read_tokens: UsageCount,
        /// Prompt tokens written to the provider's prompt cache.
        cache_creation_tokens: UsageCount,
    },
    Completed {
        stop_reason: StopReason,
    },
    Failed {
        error: VkdgError,
    },
}

impl UsageCount {
    /// The count if known (reported or estimated), else 0.
    pub fn value(&self) -> u32 {
        match self {
            UsageCount::Reported(n) | UsageCount::Estimated(n) => *n,
            UsageCount::Unknown => 0,
        }
    }
}

// ── Stream contracts ──────────────────────────────────────────────────────────

/// Request data a provider stream decoder may need (e.g. per-model limits).
#[derive(Debug, Clone, Copy)]
pub struct StreamContext<'a> {
    /// Model id requested by the client.
    pub model: &'a str,
    pub request_id: &'a RequestId,
}

/// Encodes [`ConversationEvent`]s into one client wire dialect (Anthropic SSE,
/// OpenAI chunks). Stateful: one instance per response stream. Implemented by
/// the ingress crates; the pipeline only drives it.
pub trait StreamEncoder: Send {
    /// Wire bytes for one event (may be empty; may open/close blocks first).
    fn encode(&mut self, event: &ConversationEvent) -> Vec<u8>;

    /// Called once when the event stream ends. Closes anything still open so the
    /// client always receives a well-formed terminal sequence.
    fn finish(&mut self) -> Vec<u8>;
}

/// Builds a [`StreamEncoder`] for one response. Passed by the ingress to the pipeline.
pub type StreamEncoderFactory = fn(&StreamContext<'_>) -> Box<dyn StreamEncoder>;

pub mod stream_encode;
pub use stream_encode::{AnthropicStreamEncoder, OpenAiStreamEncoder};

/// Builds the encoder for the dialect the client spoke.
///
/// The pipeline uses this whenever a provider needs protocol translation, so a
/// dialect is written in exactly one place.
pub fn stream_encoder_for(
    api_type: &vkdg_core::ApiType,
    ctx: &StreamContext<'_>,
) -> Box<dyn StreamEncoder> {
    match api_type {
        vkdg_core::ApiType::OpenAiChatCompletions => Box::new(OpenAiStreamEncoder::new(ctx)),
        // Anthropic Messages is the internal default dialect.
        _ => Box::new(AnthropicStreamEncoder::new(ctx)),
    }
}

// ── Non-streaming response ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationUsage {
    pub input_tokens: UsageCount,
    pub output_tokens: UsageCount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationResponse {
    pub request_id: RequestId,
    pub content: Vec<ContentBlock>,
    pub usage: ConversationUsage,
    pub stop_reason: StopReason,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Defeat: VideoGenerateRequest is defined without a `prompt` field (empty struct),
    // so callers cannot set the required prompt and the upstream always gets an empty body.
    #[test]
    fn video_generate_request_has_prompt() {
        let req = VideoGenerateRequest {
            prompt: "a sunset over the ocean".into(),
            model: None,
            duration_secs: None,
            resolution: None,
            style: None,
            reference_image: None,
            audio: None,
            webhook_url: None,
        };
        assert!(!req.prompt.is_empty(), "prompt must not be empty");
        assert_eq!(req.prompt, "a sunset over the ocean");
    }
}
