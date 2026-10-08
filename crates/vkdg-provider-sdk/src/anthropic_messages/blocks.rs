//! Turns and content blocks in the JSON shapes of the Anthropic Messages API.
//! `ContentBlock`'s own serde form is an internal shape, not Anthropic's (images
//! most visibly), so every block is written out here.

use serde_json::{json, Value};
use vkdg_operations::{CacheControl, ContentBlock, ImageData};

use crate::turns::{Piece, Turn, MISSING_RESULT_TEXT};

pub(super) fn turn_to_wire(turn: &Turn<'_>) -> Value {
    let role = if turn.assistant { "assistant" } else { "user" };

    // A lone text piece is the plain-string form, as clients usually write it;
    // a breakpoint needs the block form to ride on.
    let content = match turn.pieces.as_slice() {
        [Piece::Text(t)] => Value::String((*t).to_owned()),
        [Piece::Block(ContentBlock::Text {
            text,
            cache_control: None,
        })] => Value::String(text.clone()),
        pieces => Value::Array(pieces.iter().map(piece_to_wire).collect()),
    };
    json!({ "role": role, "content": content })
}

fn piece_to_wire(piece: &Piece<'_>) -> Value {
    match piece {
        Piece::Text(t) => json!({ "type": "text", "text": t }),
        Piece::Rendered(t) => json!({ "type": "text", "text": t }),
        Piece::MissingResult(id) => json!({
            "type": "tool_result",
            "tool_use_id": id,
            "content": MISSING_RESULT_TEXT,
            "is_error": true,
        }),
        Piece::Block(block) => block_to_wire(block),
    }
}

fn block_to_wire(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text {
            text,
            cache_control,
        } => marked(
            json!({ "type": "text", "text": text }),
            cache_control.as_ref(),
        ),
        ContentBlock::Image {
            media_type,
            data,
            cache_control,
        } => marked(image_to_wire(media_type, data), cache_control.as_ref()),
        ContentBlock::ToolUse {
            id,
            name,
            input,
            cache_control,
        } => {
            // `input` must be an object; a call replayed without arguments has none.
            let input = if input.is_null() {
                json!({})
            } else {
                input.clone()
            };
            marked(
                json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                cache_control.as_ref(),
            )
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            images,
            is_error,
            cache_control,
        } => {
            // Text-only results keep the string form; with images the content is an
            // array, and a text block is written only when there is text (Anthropic
            // rejects empty ones).
            let content = if images.is_empty() {
                Value::String(content.clone())
            } else {
                let mut parts: Vec<Value> = Vec::with_capacity(images.len() + 1);
                if !content.is_empty() {
                    parts.push(json!({ "type": "text", "text": content }));
                }
                parts.extend(images.iter().map(|i| image_to_wire(&i.media_type, &i.data)));
                Value::Array(parts)
            };
            let mut v = json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
            });
            if *is_error {
                v["is_error"] = Value::Bool(true);
            }
            marked(v, cache_control.as_ref())
        }
        ContentBlock::Thinking {
            thinking,
            signature,
        } => json!({ "type": "thinking", "thinking": thinking, "signature": signature }),
        ContentBlock::RedactedThinking { data } => {
            json!({ "type": "redacted_thinking", "data": data })
        }
    }
}

/// `{"type":"ephemeral"}` plus the client's `ttl`, verbatim.
pub(super) fn cache_control_wire(cache_control: &CacheControl) -> Value {
    let mut v = json!({ "type": "ephemeral" });
    if let Some(ttl) = &cache_control.ttl {
        v["ttl"] = Value::String(ttl.clone());
    }
    v
}

/// `block` with the client's prompt-cache breakpoint, when it had one.
pub(super) fn marked(mut block: Value, cache_control: Option<&CacheControl>) -> Value {
    if let Some(cc) = cache_control {
        block["cache_control"] = cache_control_wire(cc);
    }
    block
}

fn image_to_wire(media_type: &str, data: &ImageData) -> Value {
    match data {
        ImageData::Base64 { data } => json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data },
        }),
        ImageData::Url { url } => json!({
            "type": "image",
            "source": { "type": "url", "url": url },
        }),
    }
}
