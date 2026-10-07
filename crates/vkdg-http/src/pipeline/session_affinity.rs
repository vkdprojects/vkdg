//! Conversation affinity key.
//!
//! No client sends a session key, so the gateway derives one. A conversation is
//! identified by who is talking and how it opened: the tenant, the client, the
//! system prompt and the first user message. Those never change as the history
//! grows, so every turn of a conversation maps to the same key while another
//! conversation (different opening) maps to another.
//!
//! The key is a SHA-256 digest: it carries no prompt text, so it is safe to hold
//! in the session registry and to appear in a debug trace.

use sha2::{Digest, Sha256};
use vkdg_core::{RequestEnvelope, SessionKey};
use vkdg_operations::{ConversationRequest, Role};

/// Key for `conv`, or `None` when the request has nothing to anchor on (no
/// client session id and no user message).
///
/// A client-supplied `session_id` wins: it names the conversation exactly, where
/// the opening message is only a proxy for it.
pub fn derive(envelope: &RequestEnvelope, conv: &ConversationRequest) -> Option<SessionKey> {
    let mut hasher = Sha256::new();
    absorb(&mut hasher, envelope.tenant_id.0.as_bytes());
    absorb(&mut hasher, envelope.client_id.0.as_bytes());
    if let Some(id) = &conv.session_id {
        absorb(&mut hasher, b"session-id");
        absorb(&mut hasher, id.as_bytes());
    } else {
        let opening = conv.messages.iter().find(|m| m.role == Role::User)?;
        absorb(&mut hasher, b"opening");
        absorb(&mut hasher, conv.system.as_deref().unwrap_or("").as_bytes());
        // `Message` serialises infallibly; an empty fallback would only widen
        // the key's collisions, never break routing.
        absorb(
            &mut hasher,
            &serde_json::to_vec(&opening.content).unwrap_or_default(),
        );
    }
    Some(SessionKey(hex(&hasher.finalize())))
}

/// Length-prefixed, so `("ab", "c")` and `("a", "bc")` hash differently.
fn absorb(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn hex(digest: &[u8]) -> String {
    use std::fmt::Write;
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::{ApiType, ClientId, RequestId, TenantId};
    use vkdg_operations::{Message, MessageContent};

    fn envelope(client: &str) -> RequestEnvelope {
        RequestEnvelope {
            request_id: RequestId::new(),
            client_id: ClientId(client.into()),
            tenant_id: TenantId("default".into()),
            session_key: None,
            api_type: ApiType::AnthropicMessages,
            model_requested: "claude-x".into(),
            deadline: None,
            mode_pack_override: None,
            compression_override: None,
            cache_bypass: false,
            include_think_tags: false,
            client_ip: None,
        }
    }

    fn msg(role: Role, s: &str) -> Message {
        Message {
            role,
            content: MessageContent::Text(s.into()),
        }
    }

    fn conv(system: &str, messages: Vec<Message>) -> ConversationRequest {
        ConversationRequest {
            messages,
            system: Some(system.into()),
            ..Default::default()
        }
    }

    // Plausible wrong impl: hashing the whole history, so the key changes on
    // every turn and the pin is never found again.
    #[test]
    fn key_is_stable_as_the_history_grows() {
        let e = envelope("omp");
        let first = conv("sys", vec![msg(Role::User, "open")]);
        let later = conv(
            "sys",
            vec![
                msg(Role::User, "open"),
                msg(Role::Assistant, "a"),
                msg(Role::User, "more"),
            ],
        );
        assert_eq!(derive(&e, &first), derive(&e, &later));
    }

    // Plausible wrong impl: a constant or client-only key, which would pin every
    // conversation of a client to one account.
    #[test]
    fn key_differs_between_conversations() {
        let e = envelope("omp");
        let a = derive(&e, &conv("sys", vec![msg(Role::User, "open a")]));
        let b = derive(&e, &conv("sys", vec![msg(Role::User, "open b")]));
        let c = derive(&e, &conv("other sys", vec![msg(Role::User, "open a")]));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    // Plausible wrong impl: two clients sending the same opening share a pin.
    #[test]
    fn key_differs_between_clients() {
        let c = conv("sys", vec![msg(Role::User, "open")]);
        assert_ne!(derive(&envelope("a"), &c), derive(&envelope("b"), &c));
    }

    // Plausible wrong impl: concatenating fields without a separator makes
    // ("ab","c") and ("a","bc") collide.
    #[test]
    fn fields_do_not_run_together() {
        let e = envelope("omp");
        let a = derive(&e, &conv("ab", vec![msg(Role::User, "c")]));
        let b = derive(&e, &conv("a", vec![msg(Role::User, "bc")]));
        assert_ne!(a, b);
    }

    // Plausible wrong impl: the explicit session id is ignored in favour of the
    // opening message, so two sessions that open alike share a pin.
    #[test]
    fn explicit_session_id_takes_precedence() {
        let e = envelope("omp");
        let mut one = conv("sys", vec![msg(Role::User, "same opening")]);
        let mut two = one.clone();
        one.session_id = Some("s-1".into());
        two.session_id = Some("s-2".into());
        assert_ne!(derive(&e, &one), derive(&e, &two));

        // The id alone anchors the key, whatever the history says.
        let mut regrown = one.clone();
        regrown.messages.push(msg(Role::Assistant, "a"));
        regrown.system = Some("changed".into());
        assert_eq!(derive(&e, &one), derive(&e, &regrown));
    }

    // Plausible wrong impl: the key embeds conversation text.
    #[test]
    fn key_carries_no_conversation_text() {
        let e = envelope("omp");
        let key = derive(
            &e,
            &conv("secret system", vec![msg(Role::User, "secret opening")]),
        )
        .unwrap()
        .0;
        assert_eq!(key.len(), 64);
        assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn nothing_to_anchor_on_gives_no_key() {
        let e = envelope("omp");
        assert_eq!(derive(&e, &conv("sys", vec![])), None);
        assert_eq!(
            derive(&e, &conv("sys", vec![msg(Role::Assistant, "a")])),
            None
        );
    }
}
