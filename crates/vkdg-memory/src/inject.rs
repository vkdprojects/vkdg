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
    let prefix = format!(
        "Relevant context from previous interactions:\n{}\n",
        memory_block
    );
    req.system = match req.system.take() {
        None => Some(prefix),
        Some(existing) => Some(format!("{prefix}\n{existing}")),
    };
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
            messages: vec![],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
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

    // Plausible wrong impl: inject with empty memories modifies request
    #[test]
    fn empty_memories_no_change() {
        let req = empty_req();
        let out = inject_memories(req, &[]);
        assert!(out.system.is_none());
    }
}
