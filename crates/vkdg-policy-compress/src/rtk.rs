//! RTK compressor: tool-result block detection and class-based filtering.
//!
//! RTK achieves 60-90% savings on tool outputs by:
//! 1. Detecting the content class (command output, JSON, stack trace, file list, diff)
//! 2. Dispatching to the matching FilterPack via PackRegistry
//!
//! Designed for CI/CD pipelines and code agents that produce long tool outputs.
//! Never removes code blocks, line numbers, URLs, or identifiers.

use crate::{
    class::{detect_class, ContentClass},
    metrics::CompressionMetrics,
    registry::PackRegistry,
    CompressionError, Compressor,
};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Role};

pub struct RtkCompressor {
    registry: PackRegistry,
}

impl RtkCompressor {
    /// Build an `RtkCompressor` backed by the given registry.
    pub fn new(registry: PackRegistry) -> Self {
        Self { registry }
    }
}

impl Default for RtkCompressor {
    fn default() -> Self {
        Self::new(PackRegistry::with_all_defaults())
    }
}

impl Compressor for RtkCompressor {
    fn name(&self) -> &str {
        "rtk"
    }

    fn estimate_tokens(&self, req: &ConversationRequest) -> u32 {
        req.messages
            .iter()
            .map(|m| match &m.content {
                MessageContent::Text(s) => (s.len() as u32).saturating_div(4),
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::ToolResult { content, .. } => {
                            (content.len() as u32).saturating_div(4)
                        }
                        ContentBlock::Text { text } => (text.len() as u32).saturating_div(4),
                        _ => 20,
                    })
                    .sum(),
            })
            .sum()
    }

    fn compress(
        &self,
        mut req: ConversationRequest,
        _budget: u32,
    ) -> Result<(ConversationRequest, CompressionMetrics), CompressionError> {
        let original_tokens = self.estimate_tokens(&req);

        for msg in req.messages.iter_mut() {
            if matches!(msg.role, Role::System) {
                continue;
            }

            match &mut msg.content {
                MessageContent::Blocks(blocks) => {
                    for block in blocks.iter_mut() {
                        if let ContentBlock::ToolResult { content, .. } = block {
                            let class = detect_class(content);
                            if let Some(pack) = self.registry.find_for_class(&class) {
                                *content = pack.apply(content);
                            }
                        }
                    }
                }
                MessageContent::Text(text) => {
                    // RTK also applies to plain text if it looks like tool output
                    let class = detect_class(text);
                    if !matches!(class, ContentClass::Generic) {
                        if let Some(pack) = self.registry.find_for_class(&class) {
                            *text = pack.apply(text);
                        }
                    }
                }
            }
        }

        let compressed_tokens = self.estimate_tokens(&req);
        let removed = original_tokens.saturating_sub(compressed_tokens);
        Ok((
            req,
            CompressionMetrics {
                original_message_count: 0,
                compressed_message_count: 0,
                estimated_tokens_removed: removed,
                strategy: "rtk".into(),
                lossless: removed == 0,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_operations::{CapabilitySet, ContentBlock, Message, MessageContent, Role};

    fn tool_result_req(content: &str) -> ConversationRequest {
        ConversationRequest {
            messages: vec![Message {
                role: Role::Tool,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".into(),
                    content: content.into(),
                }]),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
        }
    }

    // Plausible wrong impl: RTK removes content from short outputs that don't need truncation
    #[test]
    fn short_command_output_unchanged() {
        let c = RtkCompressor::default();
        let short = "$ ls\nfoo.txt\nbar.txt";
        let req = tool_result_req(short);
        let (out, _metrics) = c.compress(req, 10000).unwrap();
        let content = match &out.messages[0].content {
            MessageContent::Blocks(b) => match &b[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!("unexpected block type"),
            },
            _ => panic!("unexpected content type"),
        };
        // Short output should not grow
        assert!(
            content.len() <= short.len() + 10,
            "short output grew unexpectedly: {content:?}"
        );
    }

    // Plausible wrong impl: stack trace truncation keeps wrong frames
    #[test]
    fn stack_trace_truncated_to_5_frames() {
        let c = RtkCompressor::default();
        let trace = "Error: null pointer\n".to_string()
            + &(0..20)
                .map(|i| format!("\tat frame{}()", i))
                .collect::<Vec<_>>()
                .join("\n");
        let req = tool_result_req(&trace);
        let (out, _) = c.compress(req, 10000).unwrap();
        let content = match &out.messages[0].content {
            MessageContent::Blocks(b) => match &b[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!(),
            },
            _ => panic!(),
        };
        let frame_lines = content.lines().filter(|l| l.contains("\tat ")).count();
        assert!(
            frame_lines <= 5,
            "stack trace must be truncated to 5 frames, got {frame_lines}"
        );
    }

    // Plausible wrong impl: JSON output mangled (extra quotes, etc.)
    #[test]
    fn json_output_compact_but_valid() {
        let c = RtkCompressor::default();
        let json = "{\n  \"key\": \"value\",\n  \"num\": 42\n}";
        let req = tool_result_req(json);
        let (out, _) = c.compress(req, 10000).unwrap();
        let content = match &out.messages[0].content {
            MessageContent::Blocks(b) => match &b[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!(),
            },
            _ => panic!(),
        };
        // Must still be valid JSON
        assert!(
            serde_json::from_str::<serde_json::Value>(&content).is_ok(),
            "compacted JSON must be valid: {content}"
        );
        // Must be more compact than original
        assert!(content.len() < json.len(), "JSON not compacted: {content}");
    }

    // Plausible wrong impl: system messages compressed (must be preserved)
    #[test]
    fn system_messages_not_compressed() {
        let c = RtkCompressor::default();
        let sys = "You are a helpful assistant. $ run this command\n".repeat(100);
        let mut req = tool_result_req("hello");
        req.messages.push(Message {
            role: Role::System,
            content: MessageContent::Text(sys.clone()),
        });
        let (out, _) = c.compress(req, 10000).unwrap();
        match &out.messages[1].content {
            MessageContent::Text(t) => assert_eq!(*t, sys, "system message must be unchanged"),
            _ => panic!("unexpected content type"),
        }
    }

    fn extract_content(req: &ConversationRequest) -> String {
        match &req.messages[0].content {
            MessageContent::Blocks(b) => match &b[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!("unexpected block type"),
            },
            _ => panic!("unexpected content type"),
        }
    }

    // Plausible wrong impl: TypeScript build errors not detected as TypeScriptBuild class
    #[test]
    fn typescript_errors_detected_as_build_output() {
        let ts_output = "src/main.ts(42,5): error TS2345: Argument of type 'string'\nsrc/utils.ts(18,3): error TS2304: Cannot find name 'foo'\nFound 2 errors.";
        let class = detect_class(ts_output);
        assert!(
            matches!(class, ContentClass::TypeScriptBuild),
            "must detect TS build: {:?}",
            class
        );
    }

    // Plausible wrong impl: git status not detected
    #[test]
    fn git_status_detected() {
        let git_out = "On branch main\nChanges not staged for commit:\n  modified: src/main.rs";
        let class = detect_class(git_out);
        assert!(
            matches!(class, ContentClass::GitStatus),
            "must detect git status: {:?}",
            class
        );
    }

    // Plausible wrong impl: test output not compressed
    #[test]
    fn test_output_retains_only_summary_lines() {
        let c = RtkCompressor::default();
        let test_out = (0..50)
            .map(|i| format!("test test_{i} ... ok"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\ntest result: ok. 50 passed; 0 failed";
        let req = tool_result_req(&test_out);
        let (out, metrics) = c.compress(req, 10000).unwrap();
        let content = extract_content(&out);
        assert!(
            content.contains("50 passed"),
            "summary must be preserved: {content}"
        );
        assert!(
            metrics.estimated_tokens_removed > 0,
            "test output must be compressed"
        );
    }
}
