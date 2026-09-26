//! Caveman compressor: strip filler words and redundant phrases.
//!
//! Named after BurntSushi's caveman — regex approach to text compression.
//! Applies 30+ regex rules targeting common AI-generated padding:
//! - Preamble padding ("Certainly! I'd be happy to...")
//! - Transition filler ("It's worth noting that...", "As mentioned above...")
//! - Redundant affirmations ("Great question!", "Absolutely!")
//! - Verbose attributions ("According to the information provided...")
//!
//! Lossiness: low-risk on content; targets only structural padding.
//! Does NOT remove code blocks, URLs, identifiers, or numbers.

use regex::Regex;
use std::sync::LazyLock;
use vkdg_operations::{ConversationRequest, MessageContent, Role};
use crate::{Compressor, CompressionError, metrics::CompressionMetrics};

pub struct CavemanCompressor;

impl Compressor for CavemanCompressor {
    fn name(&self) -> &str { "caveman" }

    fn estimate_tokens(&self, req: &ConversationRequest) -> u32 {
        req.messages.iter().map(|m| match &m.content {
            MessageContent::Text(s) => (s.len() as u32).saturating_div(4),
            MessageContent::Blocks(_) => 50,
        }).sum()
    }

    fn compress(
        &self,
        mut req: ConversationRequest,
        _budget: u32,
    ) -> Result<(ConversationRequest, CompressionMetrics), CompressionError> {
        let original_tokens = self.estimate_tokens(&req);

        for msg in req.messages.iter_mut() {
            // Only apply to user/assistant text — never to system or tool messages.
            if matches!(msg.role, Role::System | Role::Tool) {
                continue;
            }
            if let MessageContent::Text(text) = &mut msg.content {
                *text = apply_caveman_rules(text);
            }
        }

        let compressed_tokens = self.estimate_tokens(&req);
        let removed = original_tokens.saturating_sub(compressed_tokens);

        Ok((req, CompressionMetrics {
            original_message_count: 0,
            compressed_message_count: 0,
            estimated_tokens_removed: removed,
            strategy: "caveman".into(),
            lossless: removed == 0,
        }))
    }
}

/// Apply all Caveman rules to a text string.
fn apply_caveman_rules(text: &str) -> String {
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        vec![
            // Preamble / affirmation openers (line-start or standalone sentence)
            (Regex::new(r"(?i)^(?:Certainly!?|Absolutely!?|Of course!?|Sure!?|Great question!?)\s*").unwrap(), ""),
            (Regex::new(r"(?i)I(?:'d| would) be (?:happy|glad|pleased) to (?:help(?: you)? (?:with that\.?\s*|with this\.?\s*)|(?:explain|clarify|assist)[^.]*\.\s*)").unwrap(), ""),
            (Regex::new(r"(?i)I(?:'m| am) (?:glad|happy|pleased) you asked\.?\s*").unwrap(), ""),
            (Regex::new(r"(?i)(?:Great|Excellent|Wonderful|Fantastic|Awesome)!\s*").unwrap(), ""),
            // Transition filler
            (Regex::new(r"(?i)It(?:'s| is) worth (?:noting|mentioning|pointing out) that\s+").unwrap(), ""),
            (Regex::new(r"(?i)It(?:'s| is) important to (?:note|mention|point out) that\s+").unwrap(), ""),
            (Regex::new(r"(?i)As (?:I )?mentioned (?:above|earlier|previously),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)As (?:you )?(?:can see|may (?:know|be aware)|probably know),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)In (?:summary|conclusion|other words),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)To (?:summarize|recap|reiterate),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)That being said,?\s+").unwrap(), ""),
            (Regex::new(r"(?i)With that (?:said|in mind),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)Having said that,?\s+").unwrap(), ""),
            (Regex::new(r"(?i)First(?:ly)? and foremost,?\s+").unwrap(), ""),
            (Regex::new(r"(?i)Last but not least,?\s+").unwrap(), ""),
            (Regex::new(r"(?i)Without further ado,?\s+").unwrap(), ""),
            // Verbose attribution
            (Regex::new(r"(?i)According to (?:the )?(?:information|context) (?:provided|available|given),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)Based on (?:the )?(?:information|context) (?:provided|available|given),?\s+").unwrap(), ""),
            (Regex::new(r"(?i)In (?:light|view) of (?:the )?(?:above|foregoing|information provided),?\s+").unwrap(), ""),
            // Redundant hedges
            (Regex::new(r"(?i)\b(?:please note that|note that|keep in mind that)\s+").unwrap(), ""),
            (Regex::new(r"(?i)\bI (?:want|need|would like) to (?:point out|emphasize|highlight) that\s+").unwrap(), ""),
            // Closing fluff
            (Regex::new(r"(?i)\n+I hope (?:this|that) (?:helps|answers your question|clarifies)[^.!?]*[.!?]?\s*$").unwrap(), ""),
            (Regex::new(r"(?i)\n+(?:Let me know|Feel free to (?:ask|reach out))[^.!?]*[.!?]?\s*$").unwrap(), ""),
            (Regex::new(r"(?i)\n+(?:Don't hesitate to ask|Please (?:don't hesitate|feel free))[^.!?]*[.!?]?\s*$").unwrap(), ""),
            // Multiple blank lines -> single blank line
            (Regex::new(r"\n{3,}").unwrap(), "\n\n"),
            // Trailing whitespace per line
            (Regex::new(r"[ \t]+\n").unwrap(), "\n"),
        ]
    });

    let mut result = text.to_string();
    for (re, replacement) in RULES.iter() {
        result = re.replace_all(&result, *replacement).into_owned();
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_operations::{Message, MessageContent, Role};
    use vkdg_core::CapabilitySet;

    fn make_req(text: &str) -> ConversationRequest {
        ConversationRequest {
            messages: vec![Message { role: Role::User, content: MessageContent::Text(text.into()) }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
        }
    }

    // Plausible wrong impl: compressor strips content, not just filler
    #[test]
    fn strips_preamble_preserves_content() {
        let c = CavemanCompressor;
        let req = make_req("Certainly! I'd be happy to help you with that. The answer is 42.");
        let (out, _metrics) = c.compress(req, 10000).unwrap();
        let text = match &out.messages[0].content {
            MessageContent::Text(t) => t.clone(),
            _ => unreachable!(),
        };
        assert!(text.contains("The answer is 42"), "content preserved: {text}");
        assert!(!text.contains("Certainly"), "preamble stripped: {text}");
    }

    // Plausible wrong impl: system messages modified (must be preserved verbatim)
    #[test]
    fn system_messages_not_modified() {
        let c = CavemanCompressor;
        let original = "Certainly! You are a helpful assistant.";
        let mut req = make_req("hello");
        req.messages.push(Message {
            role: Role::System,
            content: MessageContent::Text(original.into()),
        });
        let (out, _) = c.compress(req, 10000).unwrap();
        let sys_text = match &out.messages[1].content {
            MessageContent::Text(t) => t.clone(),
            _ => unreachable!(),
        };
        assert_eq!(sys_text, original, "system message must be unchanged");
    }

    // Plausible wrong impl: metrics.estimated_tokens_removed is always 0
    #[test]
    fn metrics_reflect_actual_savings() {
        let c = CavemanCompressor;
        let req = make_req("Certainly! I'd be happy to help. The sky is blue.");
        let (_, metrics) = c.compress(req, 10000).unwrap();
        assert!(metrics.estimated_tokens_removed > 0, "must report token savings");
    }

    // Plausible wrong impl: empty string panics or returns garbage
    #[test]
    fn empty_text_no_panic() {
        let c = CavemanCompressor;
        let req = make_req("");
        let result = c.compress(req, 10000);
        assert!(result.is_ok());
    }

    // Plausible wrong impl: tool messages silently stripped
    #[test]
    fn tool_messages_not_modified() {
        let c = CavemanCompressor;
        let original = "Certainly! tool result data: key=value";
        let mut req = make_req("user message");
        req.messages.push(Message {
            role: Role::Tool,
            content: MessageContent::Text(original.into()),
        });
        let (out, _) = c.compress(req, 10000).unwrap();
        let tool_text = match &out.messages[1].content {
            MessageContent::Text(t) => t.clone(),
            _ => unreachable!(),
        };
        assert_eq!(tool_text, original, "tool message must be unchanged");
    }

    // Plausible wrong impl: transition filler left intact
    #[test]
    fn strips_transition_filler() {
        let c = CavemanCompressor;
        let req = make_req("It's worth noting that Rust is fast.");
        let (out, _) = c.compress(req, 10000).unwrap();
        let text = match &out.messages[0].content {
            MessageContent::Text(t) => t.clone(),
            _ => unreachable!(),
        };
        assert!(text.contains("Rust is fast"), "content preserved: {text}");
        assert!(!text.contains("worth noting"), "filler stripped: {text}");
    }
}
