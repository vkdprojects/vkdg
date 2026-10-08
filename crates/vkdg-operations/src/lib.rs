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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageData {
    Base64 { data: String },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Image {
        media_type: String,
        data: ImageData,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        /// The text of the result (text blocks joined with a newline).
        content: String,
        /// Images the tool returned. Anthropic takes them inside the result; `OpenAI`
        /// Chat has no place for them in a `tool` message.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ToolResultImage>,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
}

/// An Anthropic prompt-cache breakpoint (`"cache_control":{"type":"ephemeral"}`).
///
/// `ephemeral` is the only type Anthropic defines, so it is implied; only the
/// optional `ttl` (`"5m"` or `"1h"`) is carried, verbatim. Providers that have no
/// such breakpoint ignore it. It is never part of a cache or affinity key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

/// One block of a structured system prompt, as an Anthropic client sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBlock {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// An image a tool returned inside its result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultImage {
    pub media_type: String,
    pub data: ImageData,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// A tool the provider runs itself (Anthropic `web_search_20250305`, `code_execution`,
/// ...), declared by an Anthropic client. It has no input schema and its calls never
/// reach the client as tool calls, so it is kept as the client wrote it and forwarded
/// only to Anthropic upstreams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTool {
    pub name: String,
    /// The complete declaration object, forwarded verbatim.
    pub declaration: serde_json::Value,
}

// ── Conversation request ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationRequest {
    /// Model the client asked for, as written on the wire.
    ///
    /// A connection's `models` list holds route patterns (`claude-*`), not model
    /// ids, so an adapter that read it would send a glob upstream. This carries the
    /// actual request through to `prepare`.
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    /// Provider-run tools the client declared (Anthropic clients only); see [`ServerTool`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<ServerTool>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub system: Option<String>,
    /// The system prompt as the client's blocks, with their cache breakpoints.
    /// `system` stays the source of truth for everything that reads or edits the
    /// prompt; these blocks are only used when they still match it (see
    /// [`ConversationRequest::system_for_wire`]), so an edited prompt never ships
    /// stale blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_blocks: Vec<SystemBlock>,
    pub required_capabilities: CapabilitySet,
    /// Extended reasoning the client asked for (Anthropic `thinking`, `OpenAI`
    /// `reasoning_effort`). `None` = the client did not ask.
    #[serde(default)]
    pub thinking: Option<ThinkingRequest>,
    /// Stable session id propagated from `x-claude-code-session-id` (or
    /// `x-session-id`). Used by providers that support conversation continuity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Stable per-conversation opaque key: the SHA-256 hex the gateway derives for
    /// connection affinity (never the raw client session id or any prompt text).
    /// Providers with a server-side prompt cache may send it upstream as a cache
    /// routing hint. `None` = the request has nothing to anchor a key on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    /// How the client wants tools used. `None` = the client did not say, which
    /// every provider treats as automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Client-supplied stop strings (Anthropic `stop_sequences`, `OpenAI` `stop`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// The client forbids more than one tool call per assistant turn (Anthropic
    /// `tool_choice.disable_parallel_tool_use: true`, `OpenAI`
    /// `parallel_tool_calls: false`). `false` = the provider's default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable_parallel_tool_use: bool,
}

impl ConversationRequest {
    /// The client's system blocks, when they still describe `system`: joined by a
    /// blank line they must equal it. Anything that edited `system` without
    /// keeping the blocks in step makes this `None`, so the caller falls back to
    /// the string and a stale block is never sent upstream.
    #[must_use]
    pub fn system_for_wire(&self) -> Option<&[SystemBlock]> {
        if self.system_blocks.is_empty() {
            return None;
        }
        let mut rest = self.system.as_deref()?;
        for (i, block) in self.system_blocks.iter().enumerate() {
            if i > 0 {
                rest = rest.strip_prefix("\n\n")?;
            }
            rest = rest.strip_prefix(block.text.as_str())?;
        }
        rest.is_empty().then_some(self.system_blocks.as_slice())
    }

    /// Puts `text` in front of the system prompt, as its own block when the
    /// prompt is kept as blocks, so the client's breakpoints stay where they were.
    pub fn prepend_system(&mut self, text: &str) {
        let in_sync = self.system_for_wire().is_some();
        self.system = Some(match self.system.take() {
            None => text.to_owned(),
            Some(existing) => format!("{text}\n\n{existing}"),
        });
        if in_sync {
            self.system_blocks.insert(
                0,
                SystemBlock {
                    text: text.to_owned(),
                    cache_control: None,
                },
            );
        } else {
            self.system_blocks.clear();
        }
    }

    /// Replaces the system prompt; the client's blocks no longer apply.
    pub fn set_system(&mut self, system: Option<String>) {
        self.system = system;
        self.system_blocks.clear();
    }
}

/// `messages` with every prompt-cache breakpoint removed, for hashing: a marker
/// is a hint to the provider, not part of what the conversation says. Borrowed
/// unchanged when there is none.
#[must_use]
pub fn without_cache_control(messages: &[Message]) -> std::borrow::Cow<'_, [Message]> {
    fn marker(b: &mut ContentBlock) -> Option<&mut Option<CacheControl>> {
        match b {
            ContentBlock::Text { cache_control, .. }
            | ContentBlock::Image { cache_control, .. }
            | ContentBlock::ToolUse { cache_control, .. }
            | ContentBlock::ToolResult { cache_control, .. } => Some(cache_control),
            ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. } => None,
        }
    }
    let marked = |m: &Message| match &m.content {
        MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
            matches!(
                b,
                ContentBlock::Text {
                    cache_control: Some(_),
                    ..
                } | ContentBlock::Image {
                    cache_control: Some(_),
                    ..
                } | ContentBlock::ToolUse {
                    cache_control: Some(_),
                    ..
                } | ContentBlock::ToolResult {
                    cache_control: Some(_),
                    ..
                }
            )
        }),
        MessageContent::Text(_) => false,
    };
    if !messages.iter().any(marked) {
        return std::borrow::Cow::Borrowed(messages);
    }
    let mut owned = messages.to_vec();
    for m in &mut owned {
        if let MessageContent::Blocks(blocks) = &mut m.content {
            for b in blocks {
                if let Some(cc) = marker(b) {
                    *cc = None;
                }
            }
        }
    }
    std::borrow::Cow::Owned(owned)
}

/// Tool use policy requested by the client, in provider-neutral terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolChoice {
    /// The model decides (Anthropic `auto`, `OpenAI` `auto`).
    Auto,
    /// The model must call some tool (Anthropic `any`, `OpenAI` `required`).
    Required,
    /// No tool calls (Anthropic `none`, `OpenAI` `none`).
    Disabled,
    /// The model must call this tool.
    Named(String),
}

impl ToolChoice {
    /// Settles a client's `tool_choice` against the tools it sent.
    ///
    /// `Auto` and `Disabled` with no tools change nothing, so they normalise to
    /// `None` (providers reject a `tool_choice` without tools). `Required` and
    /// `Named` promise a tool call that cannot happen without tools, and `Named`
    /// must name a tool that was sent; both are request errors, not defaults to
    /// paper over.
    ///
    /// # Errors
    /// `ConfigInvalid { field: "tool_choice" }` for the two cases above.
    pub fn settle(
        choice: Option<Self>,
        tools: &[Tool],
        server_tools: &[ServerTool],
    ) -> Result<Option<Self>, VkdgError> {
        let invalid = |message: String| VkdgError::ConfigInvalid {
            field: "tool_choice".into(),
            message,
        };
        match choice {
            None => Ok(None),
            Some(Self::Auto | Self::Disabled) if tools.is_empty() && server_tools.is_empty() => {
                Ok(None)
            }
            Some(Self::Required) if tools.is_empty() && server_tools.is_empty() => {
                Err(invalid("`required` needs at least one tool".into()))
            }
            Some(Self::Named(name))
                if !tools.iter().any(|t| t.name == name)
                    && !server_tools.iter().any(|t| t.name == name) =>
            {
                Err(invalid(format!(
                    "names tool `{name}`, which is not in `tools`"
                )))
            }
            other => Ok(other),
        }
    }
}

/// The JSON dialect a provider speaks upstream, for the dialects the gateway can
/// translate to and from. A provider that speaks something else (AWS event
/// stream, the Responses API) declares `None` and keeps its own decoder or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    /// Anthropic Messages (`/v1/messages`).
    AnthropicMessages,
    /// `OpenAI` Chat Completions (`/v1/chat/completions`).
    OpenAiChat,
}

impl WireFormat {
    /// The dialect a client speaks on `api_type`, when the gateway can translate it.
    pub fn of_client(api_type: &vkdg_core::ApiType) -> Option<Self> {
        match api_type {
            vkdg_core::ApiType::AnthropicMessages => Some(Self::AnthropicMessages),
            vkdg_core::ApiType::OpenAiChatCompletions => Some(Self::OpenAiChat),
            _ => None,
        }
    }
}

/// A client's reasoning request, kept in the client's own terms: a token
/// budget (Anthropic) or an effort level (`OpenAI`). Adapters map it to their
/// provider's knob.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingRequest {
    pub budget_tokens: Option<u32>,
    /// `low` | `medium` | `high` (and `minimal`/`xhigh`/`max` where sent).
    pub effort: Option<String>,
}

impl ThinkingRequest {
    /// Effort level, from the explicit effort or a budget (`OmniRoute`'s mapping:
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
/// `OpenAI` chunks). Stateful: one instance per response stream; the pipeline
/// only drives it. Every chunk of output ends exactly on a `\n\n` event
/// boundary.
pub trait StreamEncoder: Send {
    /// Appends the wire bytes for one event to `out` (may append nothing; may
    /// open/close blocks first).
    fn encode_into(&mut self, event: &ConversationEvent, out: &mut Vec<u8>);

    /// Called once when the event stream ends. Appends whatever closes
    /// anything still open so the client always receives a well-formed
    /// terminal sequence.
    fn finish_into(&mut self, out: &mut Vec<u8>);

    /// [`encode_into`](Self::encode_into) into a fresh buffer.
    fn encode(&mut self, event: &ConversationEvent) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(event, &mut out);
        out
    }

    /// [`finish_into`](Self::finish_into) into a fresh buffer.
    fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        self.finish_into(&mut out);
        out
    }
}

/// Builds a [`StreamEncoder`] for one response. Passed by the ingress to the pipeline.
pub type StreamEncoderFactory = fn(&StreamContext<'_>) -> Box<dyn StreamEncoder>;

mod sampling;
pub use sampling::{
    validated_stop_sequences, validated_top_p, MAX_STOP_SEQUENCES, MAX_STOP_SEQUENCE_BYTES,
};

mod stop_wire;
pub use stop_wire::{
    anthropic_stop_reason, openai_finish_reason, parse_anthropic_stop_reason,
    parse_openai_finish_reason,
};

mod tool_id;
pub use tool_id::sanitize_tool_id;

mod usage;

pub mod stream_encode;
pub use stream_encode::{AnthropicStreamEncoder, OpenAiStreamEncoder};

mod error_encode;
pub use error_encode::{client_error, stream_error_frame, ClientError};

mod json_encode;
pub use json_encode::{json_encoder_for, JsonEncoder, MAX_JSON_RESPONSE_BYTES};

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

    fn tool(name: &str) -> Tool {
        Tool {
            name: name.into(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
            cache_control: None,
        }
    }

    // Defeat: forwarding `tool_choice: auto` without tools (both providers answer
    // 400 "tool_choice requires tools") or dropping a forced choice so the model
    // silently answers in prose instead of calling a tool.
    #[test]
    fn settle_drops_no_op_choices_and_rejects_impossible_ones_when_there_are_no_tools() {
        assert_eq!(
            ToolChoice::settle(Some(ToolChoice::Auto), &[], &[]).unwrap(),
            None
        );
        assert_eq!(
            ToolChoice::settle(Some(ToolChoice::Disabled), &[], &[]).unwrap(),
            None
        );
        for choice in [ToolChoice::Required, ToolChoice::Named("a".into())] {
            let err = ToolChoice::settle(Some(choice), &[], &[]).unwrap_err();
            assert!(
                matches!(&err, VkdgError::ConfigInvalid { field, .. } if field == "tool_choice"),
                "{err:?}"
            );
        }
    }

    // Defeat: accepting a forced tool the client never declared, which the
    // upstream rejects after the gateway has already spent a routing attempt.
    #[test]
    fn settle_requires_a_named_tool_to_exist() {
        let tools = [tool("read"), tool("write")];
        assert_eq!(
            ToolChoice::settle(Some(ToolChoice::Named("write".into())), &tools, &[]).unwrap(),
            Some(ToolChoice::Named("write".into()))
        );
        assert!(ToolChoice::settle(Some(ToolChoice::Named("delete".into())), &tools, &[]).is_err());
        assert_eq!(
            ToolChoice::settle(Some(ToolChoice::Required), &tools, &[]).unwrap(),
            Some(ToolChoice::Required)
        );
        assert_eq!(ToolChoice::settle(None, &tools, &[]).unwrap(), None);
    }

    fn with_blocks(texts: &[(&str, bool)]) -> ConversationRequest {
        let blocks: Vec<SystemBlock> = texts
            .iter()
            .map(|(t, marked)| SystemBlock {
                text: (*t).into(),
                cache_control: marked.then(CacheControl::default),
            })
            .collect();
        ConversationRequest {
            system: Some(
                blocks
                    .iter()
                    .map(|b| b.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            system_blocks: blocks,
            ..Default::default()
        }
    }

    // Defeat: shipping blocks that no longer spell the system text, so an
    // edited prompt reaches the upstream in its old form.
    #[test]
    fn system_blocks_are_used_only_while_they_spell_the_system_text() {
        let mut req = with_blocks(&[("a", false), ("b", true)]);
        assert_eq!(req.system_for_wire().map(<[_]>::len), Some(2));
        req.system = Some("a\n\nB".into());
        assert!(req.system_for_wire().is_none(), "edited text");
        req.system = Some("a\n\nb\n\nextra".into());
        assert!(req.system_for_wire().is_none(), "appended text");
        req.system = None;
        assert!(req.system_for_wire().is_none(), "removed text");
        assert!(ConversationRequest::default().system_for_wire().is_none());
    }

    // Defeat: prepending to the string only, which leaves the blocks stale, or
    // folding the prefix into the marked block and moving the breakpoint.
    #[test]
    fn prepend_system_adds_an_unmarked_block_in_front_when_in_sync() {
        let mut req = with_blocks(&[("rules", true)]);
        req.prepend_system("memo");
        assert_eq!(req.system.as_deref(), Some("memo\n\nrules"));
        let blocks = req.system_for_wire().expect("still in sync");
        assert_eq!(blocks[0].text, "memo");
        assert_eq!(blocks[0].cache_control, None);
        assert_eq!(blocks[1].cache_control, Some(CacheControl::default()));
    }

    #[test]
    fn prepend_system_clears_blocks_that_were_already_stale() {
        let mut req = with_blocks(&[("rules", true)]);
        req.system = Some("edited".into());
        req.prepend_system("memo");
        assert_eq!(req.system.as_deref(), Some("memo\n\nedited"));
        assert_eq!(req.system_blocks, []);

        let mut none = ConversationRequest::default();
        none.prepend_system("memo");
        assert_eq!(none.system.as_deref(), Some("memo"));
        assert_eq!(none.system_blocks, []);
    }

    #[test]
    fn set_system_drops_the_clients_blocks() {
        let mut req = with_blocks(&[("rules", true)]);
        req.set_system(Some("rules".into()));
        assert_eq!(req.system_blocks, []);
        assert_eq!(req.system.as_deref(), Some("rules"));
    }

    // Defeat: hashing the marker, so the same conversation gets a new cache key
    // or session after the client moves a breakpoint.
    #[test]
    fn without_cache_control_strips_markers_and_borrows_when_there_are_none() {
        let plain = vec![Message {
            role: Role::User,
            content: MessageContent::Text("hi".into()),
        }];
        assert!(matches!(
            without_cache_control(&plain),
            std::borrow::Cow::Borrowed(_)
        ));

        let marked = vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: "hi".into(),
                cache_control: Some(CacheControl {
                    ttl: Some("1h".into()),
                }),
            }]),
        }];
        let stripped = without_cache_control(&marked);
        let MessageContent::Blocks(blocks) = &stripped[0].content else {
            panic!("blocks expected")
        };
        assert!(matches!(
            &blocks[0],
            ContentBlock::Text { text, cache_control: None } if text == "hi"
        ));
        // The input keeps its marker.
        assert!(matches!(
            &marked[0].content,
            MessageContent::Blocks(b) if matches!(&b[0], ContentBlock::Text { cache_control: Some(_), .. })
        ));
    }
}
