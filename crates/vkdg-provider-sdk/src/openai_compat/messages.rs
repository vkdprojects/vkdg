//! Arranged conversation turns written as Chat Completions `messages`.

use serde_json::{json, Value};
use vkdg_operations::{ContentBlock, ConversationRequest, ImageData};

use crate::turns::{arrange, Piece, Turn, MISSING_RESULT_TEXT};

/// The `messages` array for `req`: one leading system message, then each turn.
///
/// Tool results always sit directly behind the assistant message that made their
/// calls (`OpenAI` rejects a `tool` message anywhere else), one `tool` message
/// per result; whatever else the user turn carried follows as a user message.
pub(super) fn chat_messages(req: &ConversationRequest) -> Vec<Value> {
    let conversation = arrange(&req.messages);
    let mut messages = Vec::with_capacity(conversation.turns.len() + 1);

    let system: Vec<&str> = req
        .system
        .as_deref()
        .into_iter()
        .chain(conversation.system_text.iter().copied())
        .filter(|s| !s.trim().is_empty())
        .collect();
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system.join("\n\n") }));
    }

    for turn in &conversation.turns {
        if turn.assistant {
            messages.extend(assistant_message(turn));
        } else {
            user_turn(turn, &mut messages);
        }
    }
    messages
}

fn assistant_message(turn: &Turn<'_>) -> Option<Value> {
    let mut text: Vec<&str> = Vec::new();
    let mut calls: Vec<Value> = Vec::new();
    for piece in &turn.pieces {
        match piece {
            Piece::Text(t) => text.push(t),
            Piece::Rendered(t) => text.push(t),
            Piece::Block(ContentBlock::Text { text: t }) => text.push(t),
            Piece::Block(ContentBlock::ToolUse { id, name, input }) => {
                // `arguments` is a JSON string; a call replayed without any is `{}`.
                let arguments = if input.is_null() {
                    "{}".to_owned()
                } else {
                    input.to_string()
                };
                calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments },
                }));
            }
            // Thinking has no Chat Completions field; results and images are not
            // assistant content.
            _ => {}
        }
    }
    let content = if text.is_empty() {
        Value::Null
    } else {
        Value::String(text.join("\n\n"))
    };
    match (calls.is_empty(), content.is_null()) {
        (true, true) => None,
        (true, false) => Some(json!({ "role": "assistant", "content": content })),
        (false, _) => Some(json!({ "role": "assistant", "content": content, "tool_calls": calls })),
    }
}

fn user_turn(turn: &Turn<'_>, messages: &mut Vec<Value>) {
    // Chat Completions `tool` messages carry text only, so the images a tool returned
    // go in the user message right behind them, each batch labelled with its call.
    let mut result_images: Vec<Value> = Vec::new();
    let mut parts: Vec<Value> = Vec::new();
    for piece in &turn.pieces {
        match piece {
            Piece::Block(ContentBlock::ToolResult {
                tool_use_id,
                content,
                images,
                ..
            }) => {
                if images.is_empty() {
                    messages.push(tool_message(tool_use_id, content));
                    continue;
                }
                let pointer;
                let text = if content.is_empty() {
                    pointer = image_pointer(images.len());
                    pointer.as_str()
                } else {
                    content
                };
                messages.push(tool_message(tool_use_id, text));
                result_images.push(text_part(&format!(
                    "[{} returned by tool call {tool_use_id}]",
                    if images.len() == 1 { "image" } else { "images" }
                )));
                result_images.extend(images.iter().map(|i| image_part(&i.media_type, &i.data)));
            }
            Piece::MissingResult(id) => messages.push(tool_message(id, MISSING_RESULT_TEXT)),
            Piece::Text(t) => parts.push(text_part(t)),
            Piece::Rendered(t) => parts.push(text_part(t)),
            Piece::Block(ContentBlock::Text { text }) => parts.push(text_part(text)),
            Piece::Block(ContentBlock::Image { media_type, data }) => {
                parts.push(image_part(media_type, data));
            }
            Piece::Block(_) => {}
        }
    }
    result_images.append(&mut parts);
    let parts = result_images;
    // A lone text part is the plain-string form every compatible server accepts.
    match parts.as_slice() {
        [] => {}
        [only] if only["type"] == "text" => {
            messages.push(json!({ "role": "user", "content": only["text"] }));
        }
        _ => messages.push(json!({ "role": "user", "content": parts })),
    }
}

fn image_pointer(count: usize) -> String {
    if count == 1 {
        "[the tool returned an image, shown in the next message]".to_owned()
    } else {
        format!("[the tool returned {count} images, shown in the next message]")
    }
}

fn image_part(media_type: &str, data: &ImageData) -> Value {
    let url = match data {
        ImageData::Base64 { data } => format!("data:{media_type};base64,{data}"),
        ImageData::Url { url } => url.clone(),
    };
    json!({ "type": "image_url", "image_url": { "url": url } })
}

fn tool_message(tool_call_id: &str, content: &str) -> Value {
    json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content })
}

fn text_part(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}
