//! Memory injection into conversation requests.

use crate::store::MemoryRecord;
use vkdg_operations::ConversationRequest;

/// Prepend retrieved memory facts to the system prompt of a request.
/// Returns a modified request; the original is unchanged.
pub fn inject_memories(
    mut req: ConversationRequest,
    memories: &[MemoryRecord],
) -> ConversationRequest {
    if memories.is_empty() {
        return req;
    }
    let memory_block = memories
        .iter()
        .map(|m| format!("- {}", m.fact))
        .collect::<Vec<_>>()
        .join("\n");
    let prefix = format!("Relevant context from previous interactions:\n{memory_block}\n");
    match req.system {
        None => req.system = Some(prefix),
        // The prefix ends with a newline, so the blank-line join of the blocks
        // adds exactly one more: the same text as before.
        Some(_) => req.prepend_system(prefix.trim_end_matches('\n')),
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;
    use vkdg_operations::{CapabilitySet, ConversationRequest};

    fn empty_req() -> ConversationRequest {
        ConversationRequest {
            model: "test-model".into(),
            messages: vec![],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
            thinking: None,
            ..Default::default()
        }
    }

    fn mem(fact: &str) -> MemoryRecord {
        MemoryRecord {
            id: Uuid::new_v4(),
            tenant_id: "t".into(),
            session_id: None,
            fact: fact.into(),
            source: "conversation".into(),
            created_at: Utc::now(),
            expires_at: None,
            tags: vec![],
        }
    }

    // Plausible wrong impl: memories overwrite existing system prompt instead of prepending
    #[test]
    fn memories_prepended_to_existing_system() {
        let req = ConversationRequest {
            system: Some("You are helpful.".into()),
            ..empty_req()
        };
        let out = inject_memories(req, &[mem("User prefers Python")]);
        let sys = out.system.unwrap();
        assert!(
            sys.contains("User prefers Python"),
            "memory must be in system"
        );
        assert!(
            sys.contains("You are helpful."),
            "original system must be preserved"
        );
        assert!(
            sys.find("User prefers Python") < sys.find("You are helpful."),
            "memory must come before original system"
        );
    }

    // The client's cache breakpoint sits on its own system block. Memories
    // change the system text, so the blocks must follow or be dropped: a stale
    // block set would send the prompt without the memories.
    #[test]
    fn memories_keep_system_blocks_in_step_with_the_system_text() {
        use vkdg_operations::{CacheControl, SystemBlock};
        let marked = Some(CacheControl::default());
        let req = ConversationRequest {
            system: Some("You are helpful.".into()),
            system_blocks: vec![SystemBlock {
                text: "You are helpful.".into(),
                cache_control: marked.clone(),
            }],
            ..empty_req()
        };
        let out = inject_memories(req, &[mem("User prefers Python")]);
        let blocks = out
            .system_for_wire()
            .expect("blocks still describe the system");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].text.contains("User prefers Python"));
        assert_eq!(
            blocks[0].cache_control, None,
            "memories are not a breakpoint"
        );
        assert_eq!(blocks[1].text, "You are helpful.");
        assert_eq!(blocks[1].cache_control, marked);
    }

    // Plausible wrong impl: inject with empty memories modifies request
    #[test]
    fn empty_memories_no_change() {
        let req = empty_req();
        let out = inject_memories(req, &[]);
        assert!(out.system.is_none());
    }
}
