//! Prompt-cache breakpoints in a finished Anthropic Messages body.
//!
//! The client's own `cache_control` markers are passed through (see
//! [`super::blocks`]); this module only decides what to do with the set as a
//! whole: add the default breakpoints when the client set none, and keep the
//! request within the API's limit when it set too many.

use serde_json::{json, Map, Value};

/// Anthropic accepts at most this many `cache_control` breakpoints per request.
pub(super) const MAX_BREAKPOINTS: usize = 4;

/// Settles the breakpoints of `body`, which only Anthropic-format providers send.
///
/// - The client set none: the default breakpoints are added, see
///   [`add_default_breakpoints`].
/// - The client set up to [`MAX_BREAKPOINTS`]: untouched, so its placement and
///   `ttl` reach the provider exactly as written.
/// - The client set more: the API would reject the request, so the earliest are
///   dropped and the last [`MAX_BREAKPOINTS`] (in `tools`, `system`, `messages`
///   order, which is the order the API reads them) stay.
///
/// `system_preamble` says the first system block is the gateway's own identity
/// block, which is never a breakpoint on its own.
pub(super) fn settle_breakpoints(body: &mut Map<String, Value>, system_preamble: bool) {
    let mut marked = 0;
    for_each_block(body, &mut |b| marked += usize::from(is_marked(b)));
    if marked == 0 {
        add_default_breakpoints(body, system_preamble);
    } else if marked > MAX_BREAKPOINTS {
        let mut surplus = marked - MAX_BREAKPOINTS;
        for_each_block(body, &mut |b| {
            if surplus > 0 && is_marked(b) {
                if let Some(o) = b.as_object_mut() {
                    o.remove("cache_control");
                }
                surplus -= 1;
            }
        });
    }
}

/// The default breakpoints for a request whose client sent none: the last custom
/// tool, the last system block and the last block of the last message. Together
/// they let the next turn of a conversation read the whole prefix from the cache.
///
/// A `thinking` or `redacted_thinking` block is never marked (the API rejects
/// it): the breakpoint moves to the nearest earlier block, or is skipped when
/// the message has none. Empty text blocks are skipped for the same reason.
/// Three breakpoints at most, so the limit holds. Below the model's minimum
/// cacheable length the API ignores a breakpoint; that is harmless.
fn add_default_breakpoints(body: &mut Map<String, Value>, system_preamble: bool) {
    // Provider-run tools carry a `type`; a custom tool has none.
    if let Some(Value::Array(tools)) = body.get_mut("tools") {
        if let Some(tool) = tools.iter_mut().rfind(|t| t.get("type").is_none()) {
            mark(tool);
        }
    }

    match body.get_mut("system") {
        Some(system @ Value::String(_)) => {
            if let Value::String(text) = system.take() {
                *system = if text.trim().is_empty() {
                    Value::String(text)
                } else {
                    json!([{ "type": "text", "text": text, "cache_control": ephemeral() }])
                };
            }
        }
        // The identity block alone is not worth a breakpoint: the client has no
        // system prompt after it.
        Some(Value::Array(blocks)) if !(system_preamble && blocks.len() == 1) => {
            if let Some(last) = blocks.last_mut() {
                mark(last);
            }
        }
        _ => {}
    }

    let Some(Value::Array(messages)) = body.get_mut("messages") else {
        return;
    };
    let Some(last) = messages.last_mut() else {
        return;
    };
    match last.get_mut("content") {
        Some(content @ Value::String(_)) => {
            if let Value::String(text) = content.take() {
                *content = if text.is_empty() {
                    Value::String(text)
                } else {
                    json!([{ "type": "text", "text": text, "cache_control": ephemeral() }])
                };
            }
        }
        Some(Value::Array(blocks)) => {
            if let Some(block) = blocks.iter_mut().rfind(|b| can_carry_marker(b)) {
                mark(block);
            }
        }
        _ => {}
    }
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

fn mark(block: &mut Value) {
    block["cache_control"] = ephemeral();
}

fn is_marked(block: &Value) -> bool {
    block.get("cache_control").is_some()
}

/// Whether the API takes a `cache_control` on `block`.
fn can_carry_marker(block: &Value) -> bool {
    match block.get("type").and_then(Value::as_str) {
        Some("thinking" | "redacted_thinking") => false,
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|t| !t.is_empty()),
        _ => true,
    }
}

/// Visits every block a breakpoint can sit on, in the order the API reads them:
/// tools, system, then messages.
fn for_each_block(body: &mut Map<String, Value>, f: &mut impl FnMut(&mut Value)) {
    if let Some(Value::Array(tools)) = body.get_mut("tools") {
        tools.iter_mut().for_each(&mut *f);
    }
    if let Some(Value::Array(blocks)) = body.get_mut("system") {
        blocks.iter_mut().for_each(&mut *f);
    }
    if let Some(Value::Array(messages)) = body.get_mut("messages") {
        for message in messages {
            if let Some(Value::Array(blocks)) = message.get_mut("content") {
                blocks.iter_mut().for_each(&mut *f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(mut body: Value, preamble: bool) -> Value {
        settle_breakpoints(body.as_object_mut().unwrap(), preamble);
        body
    }

    fn marker_count(body: &Value) -> usize {
        body.to_string().matches("cache_control").count()
    }

    #[test]
    fn a_request_without_markers_gets_the_three_default_breakpoints() {
        let body = settle(
            json!({
                "tools": [
                    { "name": "a", "input_schema": {} },
                    { "name": "b", "input_schema": {} },
                    { "type": "web_search_20250305", "name": "web_search" },
                ],
                "system": [
                    { "type": "text", "text": "s1" },
                    { "type": "text", "text": "s2" },
                ],
                "messages": [
                    { "role": "user", "content": "first" },
                    { "role": "assistant", "content": "reply" },
                    { "role": "user", "content": [
                        { "type": "text", "text": "x" },
                        { "type": "text", "text": "y" },
                    ] },
                ],
            }),
            false,
        );
        let eph = json!({ "type": "ephemeral" });
        // The last custom tool, not the provider-run one after it.
        assert_eq!(body["tools"][1]["cache_control"], eph);
        assert!(body["tools"][0].get("cache_control").is_none());
        assert!(body["tools"][2].get("cache_control").is_none());
        // The last system block only.
        assert_eq!(body["system"][1]["cache_control"], eph);
        assert!(body["system"][0].get("cache_control").is_none());
        // The last block of the last message only.
        assert_eq!(body["messages"][2]["content"][1]["cache_control"], eph);
        assert!(body["messages"][2]["content"][0]
            .get("cache_control")
            .is_none());
        assert_eq!(body["messages"][0]["content"], "first");
        assert_eq!(marker_count(&body), 3);
    }

    #[test]
    fn plain_string_system_and_last_message_become_marked_blocks() {
        let body = settle(
            json!({ "system": "sys", "messages": [{ "role": "user", "content": "hi" }] }),
            false,
        );
        let eph = json!({ "type": "ephemeral" });
        assert_eq!(
            body["system"],
            json!([{ "type": "text", "text": "sys", "cache_control": eph }])
        );
        assert_eq!(
            body["messages"][0]["content"],
            json!([{ "type": "text", "text": "hi", "cache_control": eph }])
        );
    }

    #[test]
    fn a_client_marker_anywhere_switches_the_defaults_off() {
        let body = json!({
            "tools": [{ "name": "a", "input_schema": {} }],
            "system": [{ "type": "text", "text": "s", "cache_control": { "type": "ephemeral", "ttl": "1h" } }],
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "x" }] }],
        });
        assert_eq!(settle(body.clone(), false), body);

        // Even one marker deep in the history is the client's choice.
        let body = json!({
            "tools": [{ "name": "a", "input_schema": {} }],
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "x", "cache_control": { "type": "ephemeral" } }] },
                { "role": "user", "content": [{ "type": "text", "text": "y" }] },
            ],
        });
        assert_eq!(settle(body.clone(), false), body);
    }

    #[test]
    fn the_default_breakpoint_never_lands_on_a_thinking_block() {
        let body = settle(
            json!({ "messages": [{ "role": "assistant", "content": [
                { "type": "text", "text": "answer" },
                { "type": "thinking", "thinking": "t", "signature": "s" },
                { "type": "redacted_thinking", "data": "d" },
            ] }] }),
            false,
        );
        let content = &body["messages"][0]["content"];
        assert_eq!(content[0]["cache_control"], json!({ "type": "ephemeral" }));
        assert!(content[1].get("cache_control").is_none());
        assert!(content[2].get("cache_control").is_none());

        // Nothing earlier to move to: the message is skipped, not marked.
        let body = settle(
            json!({ "messages": [{ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "t", "signature": "s" },
            ] }] }),
            false,
        );
        assert_eq!(marker_count(&body), 0);
    }

    #[test]
    fn the_identity_block_alone_is_not_marked_but_a_client_block_after_it_is() {
        let only_identity = settle(
            json!({ "system": [{ "type": "text", "text": "IDENTITY" }], "messages": [] }),
            true,
        );
        assert_eq!(marker_count(&only_identity), 0);

        let with_client = settle(
            json!({ "system": [
                { "type": "text", "text": "IDENTITY" },
                { "type": "text", "text": "client" },
            ], "messages": [] }),
            true,
        );
        assert!(with_client["system"][0].get("cache_control").is_none());
        assert!(with_client["system"][1].get("cache_control").is_some());
    }

    #[test]
    fn more_than_four_markers_keep_only_the_last_four_in_api_order() {
        let cc = json!({ "type": "ephemeral", "ttl": "1h" });
        let body = settle(
            json!({
                "tools": [{ "name": "a", "input_schema": {}, "cache_control": cc }],
                "system": [{ "type": "text", "text": "s", "cache_control": cc }],
                "messages": [
                    { "role": "user", "content": [{ "type": "text", "text": "1", "cache_control": cc }] },
                    { "role": "user", "content": [{ "type": "text", "text": "2", "cache_control": cc }] },
                    { "role": "user", "content": [{ "type": "text", "text": "3", "cache_control": cc }] },
                    { "role": "user", "content": [{ "type": "text", "text": "4", "cache_control": cc }] },
                ],
            }),
            false,
        );
        assert_eq!(marker_count(&body), MAX_BREAKPOINTS);
        assert!(body["tools"][0].get("cache_control").is_none());
        assert!(body["system"][0].get("cache_control").is_none());
        for i in 0..4 {
            // `ttl` survives on the ones that stay.
            assert_eq!(body["messages"][i]["content"][0]["cache_control"], cc);
        }
    }

    #[test]
    fn exactly_four_markers_are_left_alone() {
        let cc = json!({ "type": "ephemeral" });
        let body = json!({ "messages": (0..4).map(|i| json!(
            { "role": "user", "content": [{ "type": "text", "text": i.to_string(), "cache_control": cc }] }
        )).collect::<Vec<_>>() });
        assert_eq!(settle(body.clone(), false), body);
    }
}
