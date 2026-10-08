//! Lightweight fact extraction from conversation turns.
//!
//! Extracts facts by pattern matching on assistant responses.
//! Phase E: LLM-powered extraction with structured output.

use vkdg_operations::{Message, MessageContent, Role};

/// Extract facts from a list of messages.
/// Returns fact strings suitable for storage.
pub fn extract_facts(messages: &[Message]) -> Vec<String> {
    let mut facts = Vec::new();
    for msg in messages {
        if !matches!(msg.role, Role::Assistant) {
            continue;
        }
        if let MessageContent::Text(text) = &msg.content {
            // Look for explicit preference/fact patterns
            // Phase E: use an LLM for structured extraction
            let text_lower = text.to_lowercase();
            if text_lower.contains("i prefer") || text_lower.contains("the user prefers") {
                facts.push(format!("preference: {}", truncate(text, 200)));
            }
            if text_lower.contains("remember that") || text_lower.contains("note:") {
                facts.push(format!("note: {}", truncate(text, 200)));
            }
        }
    }
    facts
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_operations::{Message, MessageContent, Role};

    fn msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: MessageContent::Text(text.into()),
        }
    }

    // Plausible wrong impl: user messages also extracted (only assistant should be)
    #[test]
    fn only_assistant_messages_extracted() {
        let msgs = vec![
            msg(Role::User, "I prefer Python."),
            msg(
                Role::Assistant,
                "The user prefers Python for scripting tasks.",
            ),
        ];
        let facts = extract_facts(&msgs);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("preference"));
    }

    // Plausible wrong impl: extract_facts returns empty when assistant has facts
    #[test]
    fn extracts_preference_from_assistant() {
        let msgs = vec![msg(
            Role::Assistant,
            "The user prefers dark mode and TypeScript.",
        )];
        let facts = extract_facts(&msgs);
        assert!(!facts.is_empty());
    }
}
