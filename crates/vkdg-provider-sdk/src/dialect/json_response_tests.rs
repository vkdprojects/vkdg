// Complete (non-streaming) bodies decode to the same event vocabulary as streams.
// Expected values are written by hand from the Anthropic Messages and OpenAI Chat
// Completions response references.

use vkdg_core::VkdgError;
use vkdg_operations::WireFormat;

use super::event_text::render_all;
use super::json_decoder_for;

fn decode(format: WireFormat, body: &str) -> Result<Vec<String>, VkdgError> {
    json_decoder_for(format)(body.as_bytes()).map(|events| render_all(&events))
}

fn failure(format: WireFormat, body: &str) -> (u16, String) {
    match decode(format, body) {
        Err(VkdgError::UpstreamError { code, message, .. }) => (code, message),
        other => panic!("expected an upstream error, got {other:?}"),
    }
}

const ANTHROPIC: WireFormat = WireFormat::AnthropicMessages;
const OPENAI: WireFormat = WireFormat::OpenAiChat;

// Refutes: dropping a block kind, reordering blocks, leaking the signature, surfacing server
// tool blocks, or losing the cache counts.
#[test]
fn anthropic_message_decodes_blocks_in_order_with_full_usage() {
    let body = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5",
        "content":[
          {"type":"thinking","thinking":"hm","signature":"SIGSECRET"},
          {"type":"redacted_thinking","data":"REDSECRET"},
          {"type":"text","text":"Hello \"w\u00f6rld\""},
          {"type":"server_tool_use","id":"srv_1","name":"web_search","input":{"q":"x"}},
          {"type":"tool_use","id":"toolu_1","name":"get_weather","input":{"city":"Paris","unit":"c"}}
        ],
        "stop_reason":"tool_use","stop_sequence":null,
        "usage":{"input_tokens":12,"output_tokens":9,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}}"#;
    assert_eq!(
        decode(ANTHROPIC, body).unwrap(),
        [
            "started",
            r#"reasoning[0] "hm""#,
            r#"text[2] "Hello \"wörld\"""#,
            r#"tool[4] id="toolu_1" name="get_weather" args="{\"city\":\"Paris\",\"unit\":\"c\"}""#,
            "tool_end[4]",
            "usage in=R12 out=R9 cache_read=R3 cache_create=R4",
            "completed ToolUse",
        ]
    );
}

// Refutes: a missing usage object becoming zeros, or an unknown stop_reason failing the response.
#[test]
fn anthropic_message_without_usage_or_with_unknown_stop_reason_still_decodes() {
    let body =
        r#"{"type":"message","content":[{"type":"text","text":"x"}],"stop_reason":"pause_turn"}"#;
    assert_eq!(
        decode(ANTHROPIC, body).unwrap(),
        ["started", r#"text[0] "x""#, "completed EndTurn"]
    );
}

// Refutes: an error body that arrived with a 2xx status (or a gateway page) passing as an empty answer.
#[test]
fn anthropic_error_object_fails_with_its_mapped_status() {
    let body = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
    assert_eq!(failure(ANTHROPIC, body), (529, "Overloaded".to_owned()));
}

// Refutes: echoing the body in the failure, panicking on bad input, or accepting a body of
// the wrong dialect as a valid empty message.
#[test]
fn anthropic_malformed_or_foreign_bodies_fail_with_a_fixed_message() {
    for body in [
        "",
        "not json SECRET-USER-TEXT",
        "[1,2]",
        r#"{"choices":[{"message":{"content":"x"}}]}"#,
        r#"{"type":"message"}"#,
    ] {
        let (code, message) = failure(ANTHROPIC, body);
        assert_eq!(code, 502, "{body}");
        assert_eq!(
            message,
            "upstream returned a response that is not an anthropic message"
        );
        assert!(!message.contains("SECRET"));
    }
}

// Refutes: cached tokens counted twice, reasoning merged into the answer, tool arguments
// re-serialized (they are already a JSON string), or content null breaking the decode.
#[test]
fn openai_completion_decodes_reasoning_tool_calls_and_usage() {
    let body = r#"{"id":"c1","object":"chat.completion","created":1,"model":"gpt-4o",
        "choices":[{"index":0,"message":{"role":"assistant","content":null,
            "reasoning_content":"think",
            "tool_calls":[
              {"id":"call_a","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}},
              {"id":"call_b","type":"function","function":{"name":"g","arguments":""}}
            ]},
          "finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":140,"completion_tokens":5,"total_tokens":145,
                 "prompt_tokens_details":{"cached_tokens":40}}}"#;
    assert_eq!(
        decode(OPENAI, body).unwrap(),
        [
            "started",
            r#"reasoning[0] "think""#,
            r#"tool[0] id="call_a" name="f" args="{\"a\":1}""#,
            "tool_end[0]",
            r#"tool[1] id="call_b" name="g" args="""#,
            "tool_end[1]",
            "usage in=R100 out=R5 cache_read=R40 cache_create=-",
            "completed ToolUse",
        ]
    );
}

// Refutes: plain text answers losing the finish_reason, content given as an array of parts
// (some compatible providers) being dropped, or only the first choice being honoured wrongly.
#[test]
fn openai_completion_text_forms_and_finish_reasons() {
    for (message, text) in [
        (r#""content":"Hello""#, "Hello"),
        (
            r#""content":[{"type":"text","text":"Hel"},{"type":"text","text":"lo"}]"#,
            "Hello",
        ),
    ] {
        let body = format!(
            r#"{{"choices":[{{"index":0,"message":{{"role":"assistant",{message}}},"finish_reason":"length"}},
                {{"index":1,"message":{{"role":"assistant","content":"other"}},"finish_reason":"stop"}}]}}"#
        );
        assert_eq!(
            decode(OPENAI, &body).unwrap(),
            [
                "started".to_owned(),
                format!("text[0] {text:?}"),
                "completed MaxTokens".to_owned(),
            ]
        );
    }
}

// Refutes: an in-band error object on a 2xx, or an empty `choices`, passing as a valid empty answer.
#[test]
fn openai_error_object_fails_with_its_mapped_status() {
    let body = r#"{"error":{"message":"slow down","type":"rate_limit_error","code":"rate_limit_exceeded"}}"#;
    assert_eq!(failure(OPENAI, body), (429, "slow down".to_owned()));
}

#[test]
fn openai_malformed_or_foreign_bodies_fail_with_a_fixed_message() {
    for body in [
        "",
        "not json SECRET-USER-TEXT",
        r#"{"choices":[]}"#,
        r#"{"type":"message","content":[{"type":"text","text":"x"}]}"#,
    ] {
        let (code, message) = failure(OPENAI, body);
        assert_eq!(code, 502, "{body}");
        assert_eq!(
            message,
            "upstream returned a response that is not an openai chat completion"
        );
        assert!(!message.contains("SECRET"));
    }
}
