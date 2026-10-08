//! Bridges a WASM component to the [`Compressor`] trait.
//!
//! Same shape as [`crate::wasm_provider`]: the pipeline calls one trait, and a
//! compression strategy can arrive as a `.wasm` without a gateway release. This
//! matters more for compressors than for providers, because a compression heuristic
//! is exactly the kind of thing users want to tune for their own codebase.
//!
//! A compressor that fails is skipped rather than failing the request: compression
//! is an optimisation, so a broken community plugin must not cost a user their call.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vkdg_operations::{ConversationRequest, Message, MessageContent, Role};
use vkdg_policy_compress::{CompressionError, CompressionMetrics, Compressor};

use crate::{PluginManifest, WasmPluginInstance};

mod export {
    pub const NAME: &str = "name";
    pub const COMPRESS: &str = "compress";
    pub const ESTIMATE_TOKENS: &str = "estimate-tokens";
}

/// What the gateway hands a compressor.
#[derive(Serialize)]
struct CompressInput<'a> {
    request: &'a ConversationRequest,
    /// Token budget the result must fit within.
    budget: u32,
}

/// What a compressor returns.
#[derive(Deserialize)]
struct CompressedJson {
    /// Replacement messages, in order.
    messages: Vec<MessageJson>,
    #[serde(default)]
    system: Option<String>,
    report: ReportJson,
}

#[derive(Deserialize)]
struct MessageJson {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ReportJson {
    tokens_before: u32,
    tokens_after: u32,
    #[serde(default)]
    algorithm: String,
    /// True when the compressor changed message structure.
    #[serde(default)]
    lossy: bool,
}

/// A compression strategy backed by a WASM component.
pub struct WasmCompressor {
    name: String,
    instance: Arc<WasmPluginInstance>,
}

impl WasmCompressor {
    pub fn new(instance: Arc<WasmPluginInstance>, manifest: &PluginManifest) -> Self {
        let name = instance
            .call_string_fn(export::NAME, "")
            .unwrap_or_else(|_| manifest.id.0.clone());
        Self { name, instance }
    }
}

fn parse_role(role: &str) -> Role {
    match role {
        "assistant" => Role::Assistant,
        "system" => Role::System,
        "tool" => Role::Tool,
        // Anything unrecognised is treated as user content: it keeps the text in
        // the conversation rather than dropping it on a typo.
        _ => Role::User,
    }
}

impl Compressor for WasmCompressor {
    fn name(&self) -> &str {
        &self.name
    }

    fn estimate_tokens(&self, req: &ConversationRequest) -> u32 {
        // The estimate gates whether compression runs at all, so a plugin that
        // cannot answer falls back to a chars/4 approximation rather than
        // reporting zero and disabling itself.
        let fallback = || {
            let chars: usize = req
                .messages
                .iter()
                .map(|m| match &m.content {
                    MessageContent::Text(t) => t.len(),
                    MessageContent::Blocks(_) => 0,
                })
                .sum();
            u32::try_from(chars / 4).unwrap_or(u32::MAX)
        };

        serde_json::to_string(req)
            .ok()
            .and_then(|payload| {
                self.instance
                    .call_string_fn(export::ESTIMATE_TOKENS, &payload)
                    .ok()
            })
            .and_then(|out| out.trim().parse::<u32>().ok())
            .unwrap_or_else(fallback)
    }

    fn compress(
        &self,
        req: ConversationRequest,
        budget: u32,
    ) -> Result<(ConversationRequest, CompressionMetrics), CompressionError> {
        let payload = serde_json::to_string(&CompressInput {
            request: &req,
            budget,
        })
        .map_err(|_| CompressionError::NotApplicable)?;

        let out = self
            .instance
            .call_string_fn(export::COMPRESS, &payload)
            // A plugin that cannot run is "not applicable", so the pipeline moves
            // on with the uncompressed request instead of failing the call.
            .map_err(|_| CompressionError::NotApplicable)?;
        let parsed: CompressedJson =
            serde_json::from_str(&out).map_err(|_| CompressionError::NotApplicable)?;

        let original_message_count = req.messages.len();
        let mut compressed = req;
        compressed.messages = parsed
            .messages
            .into_iter()
            .map(|m| Message {
                role: parse_role(&m.role),
                content: MessageContent::Text(m.content),
            })
            .collect();
        if let Some(system) = parsed.system {
            compressed.set_system(Some(system));
        }

        let metrics = CompressionMetrics {
            original_message_count,
            compressed_message_count: compressed.messages.len(),
            estimated_tokens_removed: parsed
                .report
                .tokens_before
                .saturating_sub(parsed.report.tokens_after),
            strategy: if parsed.report.algorithm.is_empty() {
                self.name.clone()
            } else {
                parsed.report.algorithm
            },
            lossless: !parsed.report.lossy,
        };
        Ok((compressed, metrics))
    }
}
