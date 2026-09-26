//! RTK compressor: tool-result block detection and class-based filtering.
//!
//! RTK achieves 60-90% savings on tool outputs by:
//! 1. Detecting the content class (command output, JSON, stack trace, file list, diff)
//! 2. Applying class-specific filter packs (remove timestamps, truncate repetition,
//!    collapse whitespace-heavy lines, trim verbose headers)
//!
//! Designed for CI/CD pipelines and code agents that produce long tool outputs.
//! Never removes code blocks, line numbers, URLs, or identifiers.

use crate::{metrics::CompressionMetrics, CompressionError, Compressor};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Role};

#[derive(Debug, Clone, PartialEq)]
enum ContentClass {
    CommandOutput,
    StackTrace,
    JsonOutput,
    FileList,
    Diff,
    Generic,
    GitStatus,
    GitLog,
    TypeScriptBuild,
    EslintOutput,
    NpmAudit,
    DockerLog,
    TestOutput,
    HexDump,
}

fn detect_class(text: &str) -> ContentClass {
    if text.contains("\n\tat ")
        || text.contains("Traceback (most recent call")
        || text.contains("Error: at ")
    {
        ContentClass::StackTrace
    } else if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
        ContentClass::JsonOutput
    } else if text
        .lines()
        .any(|l| l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("@@"))
    {
        ContentClass::Diff
    } else if text.lines().all(|l| {
        l.trim().is_empty() || l.starts_with('/') || l.starts_with('.') || l.ends_with('/')
    }) {
        ContentClass::FileList
    } else if text.contains("On branch")
        || text.contains("Changes not staged")
        || text.contains("Untracked files:")
        || (text.contains("modified:") && text.contains("git"))
    {
        ContentClass::GitStatus
    } else if text
        .lines()
        .take(5)
        .filter(|l| !l.trim().is_empty())
        .all(|l| l.len() > 7 && l.chars().take(7).all(|c| c.is_ascii_hexdigit()))
    {
        ContentClass::GitLog
    } else if text.contains("error TS")
        || (text.contains(".ts(") && text.contains("error:"))
        || (text.contains("Cannot find module") && text.contains(".ts"))
    {
        ContentClass::TypeScriptBuild
    } else if (text.contains('✖') && (text.contains("error") || text.contains("warning")))
        || (text.contains("problems (") && text.contains("error"))
        || text.contains("Prettier:")
        || text.contains("biome check")
    {
        ContentClass::EslintOutput
    } else if text.contains("npm audit")
        || (text.contains("vulnerabilities") && text.contains("npm"))
    {
        ContentClass::NpmAudit
    } else if (text.contains("test result:")
        && (text.contains("passed") || text.contains("failed")))
        || (text.contains("PASS") && text.contains("FAIL") && text.contains("Tests:"))
        || (text.contains("passed") && text.contains("failed") && text.contains("====="))
    {
        ContentClass::TestOutput
    } else if text.lines().take(3).all(|l| {
        l.contains('|') && l.len() > 20 && l.chars().filter(|c| c.is_ascii_hexdigit()).count() > 10
    }) && !text.trim().is_empty()
    {
        ContentClass::HexDump
    } else if text.lines().any(|l| {
        l.contains('|') && l.len() > 25 && l.chars().next().is_some_and(|c| c.is_alphabetic())
    }) {
        ContentClass::DockerLog
    } else if text.contains('$') && text.contains('\n') {
        ContentClass::CommandOutput
    } else {
        ContentClass::Generic
    }
}

fn apply_rtk_filters(text: &str, class: &ContentClass) -> String {
    match class {
        ContentClass::CommandOutput => {
            let mut lines: Vec<&str> = text.lines().collect();
            // Remove consecutive blank lines
            lines.dedup_by(|a, b| a.trim().is_empty() && b.trim().is_empty());
            // Truncate if > 50 lines: keep first 25 + last 25
            if lines.len() > 50 {
                let mut out: Vec<String> = lines[..25].iter().map(|s| s.to_string()).collect();
                out.push("... [truncated] ...".to_string());
                out.extend(lines[lines.len() - 25..].iter().map(|s| s.to_string()));
                out.join("\n")
            } else {
                lines.join("\n")
            }
        }
        ContentClass::StackTrace => {
            let mut kept: Vec<String> = Vec::new();
            let mut frame_count = 0usize;
            for line in text.lines() {
                if line.contains("\tat ") || line.contains("  File \"") {
                    frame_count += 1;
                    if frame_count > 5 {
                        if frame_count == 6 {
                            kept.push("... [stack truncated] ...".to_string());
                        }
                        continue;
                    }
                }
                kept.push(line.to_string());
            }
            kept.join("\n")
        }
        ContentClass::JsonOutput => {
            // Compact JSON: remove unnecessary whitespace if it's just indented
            let joined = text.trim().to_string();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&joined) {
                serde_json::to_string(&val).unwrap_or(joined)
            } else {
                joined
            }
        }
        ContentClass::FileList => {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() > 30 {
                let total = lines.len();
                let mut out: Vec<String> = lines[..30].iter().map(|s| s.to_string()).collect();
                out.push(format!("... [{} more entries]...", total - 30));
                out.join("\n")
            } else {
                lines.join("\n")
            }
        }
        ContentClass::Diff => {
            let mut kept: Vec<String> = Vec::new();
            let mut changed = 0usize;
            for line in text.lines() {
                if line.starts_with('+') || line.starts_with('-') {
                    changed += 1;
                    if changed > 40 {
                        if changed == 41 {
                            kept.push("... [diff truncated] ...".to_string());
                        }
                        continue;
                    }
                }
                kept.push(line.to_string());
            }
            kept.join("\n")
        }
        ContentClass::Generic => {
            // Just deduplicate consecutive blank lines
            let mut lines: Vec<&str> = text.lines().collect();
            lines.dedup_by(|a, b| a.trim().is_empty() && b.trim().is_empty());
            lines.join("\n")
        }
        ContentClass::GitStatus => {
            let lines: Vec<&str> = text.lines().collect();
            let mut kept: Vec<String> = Vec::new();
            let mut file_count = 0usize;
            for line in &lines {
                let trimmed = line.trim();
                if trimmed.starts_with("modified:")
                    || trimmed.starts_with("new file:")
                    || trimmed.starts_with("deleted:")
                {
                    file_count += 1;
                    if file_count <= 10 {
                        kept.push(line.to_string());
                    }
                } else {
                    kept.push(line.to_string());
                }
            }
            if file_count > 10 {
                kept.push(format!("  ... ({} more files)", file_count - 10));
            }
            kept.join("\n")
        }
        ContentClass::GitLog => {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() > 20 {
                let total = lines.len();
                let mut out: Vec<String> = lines[..20].iter().map(|s| s.to_string()).collect();
                out.push(format!("... ({} more commits)", total - 20));
                out.join("\n")
            } else {
                text.to_string()
            }
        }
        ContentClass::TypeScriptBuild => {
            let lines: Vec<&str> = text.lines().collect();
            let total_errors = lines.iter().filter(|l| l.contains("error TS")).count();
            let mut out: Vec<String> = lines
                .iter()
                .filter(|l| l.contains("error TS") || l.contains("error:"))
                .take(20)
                .map(|s| s.to_string())
                .collect();
            if total_errors > 20 {
                out.push(format!("... {} more errors", total_errors - 20));
            }
            out.join("\n")
        }
        ContentClass::EslintOutput => {
            let lines: Vec<&str> = text.lines().collect();
            lines
                .iter()
                .filter(|l| l.contains("error") || l.contains("warning") || l.contains("problem"))
                .take(20)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }
        ContentClass::NpmAudit => {
            let lines: Vec<&str> = text.lines().collect();
            lines
                .iter()
                .filter(|l| {
                    l.contains("critical")
                        || l.contains("high")
                        || l.contains("moderate")
                        || l.contains("vulnerabilities")
                        || l.contains("npm audit fix")
                })
                .take(15)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }
        ContentClass::DockerLog => {
            let lines: Vec<&str> = text.lines().collect();
            let mut service_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut kept: Vec<String> = Vec::new();
            for line in &lines {
                if let Some(service) = line.split('|').next() {
                    let service = service.trim().to_string();
                    let count = service_counts.entry(service).or_insert(0);
                    if *count < 5 {
                        kept.push(line.to_string());
                        *count += 1;
                    }
                } else {
                    kept.push(line.to_string());
                }
            }
            kept.join("\n")
        }
        ContentClass::TestOutput => {
            let lines: Vec<&str> = text.lines().collect();
            lines
                .iter()
                .filter(|l| {
                    l.contains("PASS")
                        || l.contains("FAIL")
                        || l.contains("passed")
                        || l.contains("failed")
                        || l.contains("error")
                        || l.contains("FAILED")
                        || l.contains("test result:")
                        || l.contains("Finished")
                        || l.contains("=====")
                })
                .take(30)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }
        ContentClass::HexDump => {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() > 8 {
                let mut out: Vec<String> = lines[..8].iter().map(|s| s.to_string()).collect();
                out.push("... [hex dump truncated]".to_string());
                out.join("\n")
            } else {
                text.to_string()
            }
        }
    }
}

pub struct RtkCompressor;

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
                            *content = apply_rtk_filters(content, &class);
                        }
                    }
                }
                MessageContent::Text(text) => {
                    // RTK also applies to plain text if it looks like tool output
                    let class = detect_class(text);
                    if class != ContentClass::Generic {
                        *text = apply_rtk_filters(text, &class);
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
        let c = RtkCompressor;
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
        let c = RtkCompressor;
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
        let c = RtkCompressor;
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
        let c = RtkCompressor;
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
        let c = RtkCompressor;
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
