//! Conversation turns arranged the way tool-calling APIs validate them.
//!
//! Provider-neutral history is looser than Anthropic's and `OpenAI`'s rules:
//! clients send one `tool` message per result, interleave user text, and replay
//! calls whose results never arrived. Both providers answer each of those with a
//! 400, so the history is repaired here, once, without cloning message content.
//! The two wire builders only write the arranged turns out in their own JSON.

use vkdg_operations::{ContentBlock, Message, MessageContent, Role};

/// Result text for a tool call whose result never reached the gateway.
pub const MISSING_RESULT_TEXT: &str = "[No response received]";

/// One content block on its way to the wire.
pub enum Piece<'a> {
    /// Text of a plain-string message.
    Text(&'a str),
    /// Text the gateway wrote (an orphaned tool result rendered as prose).
    Rendered(String),
    Block(&'a ContentBlock),
    /// Error result standing in for a tool call that has none.
    MissingResult(&'a str),
}

pub struct Turn<'a> {
    pub assistant: bool,
    pub pieces: Vec<Piece<'a>>,
}

pub struct Conversation<'a> {
    pub turns: Vec<Turn<'a>>,
    /// Text of `Role::System` messages, hoisted so a provider sees system text
    /// only at the top (Anthropic accepts nothing else; some `OpenAI`-compatible
    /// servers reject a system turn after the first message).
    pub system_text: Vec<&'a str>,
}

/// Arranges `messages` into alternating turns whose tool calls and results pair
/// up: consecutive same-role messages merge, tool results lead the user turn that
/// follows their calls (in call order, one per call), a call without a result
/// gets an error result, and a result no call owns survives as text.
pub fn arrange(messages: &[Message]) -> Conversation<'_> {
    let mut turns: Vec<Turn<'_>> = Vec::with_capacity(messages.len());
    let mut system_text = Vec::new();
    for message in messages {
        if message.role == Role::System {
            system_text.extend(message_pieces(message).filter_map(|p| piece_text(&p)));
            continue;
        }
        let assistant = message.role == Role::Assistant;
        let kept: Vec<Piece<'_>> = message_pieces(message).filter(is_replayable).collect();
        if kept.is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some(prev) if prev.assistant == assistant => prev.pieces.extend(kept),
            _ => turns.push(Turn {
                assistant,
                pieces: kept,
            }),
        }
    }
    pair_tool_results(&mut turns);
    Conversation { turns, system_text }
}

fn message_pieces(message: &Message) -> impl Iterator<Item = Piece<'_>> {
    let (text, blocks) = match &message.content {
        MessageContent::Text(t) => (Some(Piece::Text(t)), &[][..]),
        MessageContent::Blocks(b) => (None, b.as_slice()),
    };
    text.into_iter().chain(blocks.iter().map(Piece::Block))
}

fn piece_text<'a>(piece: &Piece<'a>) -> Option<&'a str> {
    match piece {
        Piece::Text(t) => Some(t),
        Piece::Block(ContentBlock::Text { text, .. }) => Some(text),
        _ => None,
    }
}

/// What Anthropic accepts back: text with something in it, and thinking that
/// carries a signature it can verify (a block from another provider has none).
fn is_replayable(piece: &Piece<'_>) -> bool {
    match piece {
        Piece::Text(t) => !t.trim().is_empty(),
        Piece::Block(ContentBlock::Text { text, .. }) => !text.trim().is_empty(),
        Piece::Block(ContentBlock::Thinking { signature, .. }) => {
            signature.as_deref().is_some_and(|s| !s.is_empty())
        }
        Piece::Block(ContentBlock::RedactedThinking { data }) => !data.is_empty(),
        _ => true,
    }
}

pub fn tool_use_ids<'a>(turn: &Turn<'a>) -> Vec<&'a str> {
    let mut ids: Vec<&'a str> = Vec::new();
    for piece in &turn.pieces {
        if let Piece::Block(ContentBlock::ToolUse { id, .. }) = piece {
            if !ids.contains(&id.as_str()) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Makes every user turn answer exactly the tool calls of the assistant turn
/// before it: results first, in call order, one per call; a call without a result
/// gets an error result; a result no call owns is kept as text, not dropped.
fn pair_tool_results(turns: &mut [Turn<'_>]) {
    for i in 0..turns.len() {
        if turns[i].assistant {
            continue;
        }
        let expected = if i > 0 && turns[i - 1].assistant {
            tool_use_ids(&turns[i - 1])
        } else {
            Vec::new()
        };
        let has_results = turns[i]
            .pieces
            .iter()
            .any(|p| matches!(p, Piece::Block(ContentBlock::ToolResult { .. })));
        if expected.is_empty() && !has_results {
            continue;
        }

        let pieces = std::mem::take(&mut turns[i].pieces);
        let mut answered: Vec<Option<&ContentBlock>> = vec![None; expected.len()];
        let mut rest: Vec<Piece<'_>> = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let Piece::Block(block) = piece else {
                rest.push(piece);
                continue;
            };
            let ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = block
            else {
                rest.push(Piece::Block(block));
                continue;
            };
            match expected.iter().position(|id| *id == tool_use_id.as_str()) {
                Some(slot) if answered[slot].is_none() => answered[slot] = Some(block),
                _ => rest.push(Piece::Rendered(format!(
                    "[tool result for unknown call {tool_use_id}]\n{content}"
                ))),
            }
        }
        let mut arranged: Vec<Piece<'_>> = Vec::with_capacity(expected.len() + rest.len());
        for (id, result) in expected.iter().zip(answered) {
            arranged.push(result.map_or(Piece::MissingResult(id), Piece::Block));
        }
        arranged.extend(rest);
        turns[i].pieces = arranged;
    }
}
