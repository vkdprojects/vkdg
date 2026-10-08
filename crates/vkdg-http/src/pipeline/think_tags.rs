//! `<think>…</think>` removal on the way to the client.
//!
//! Some models (DeepSeek-R1 and friends behind OpenAI-compatible endpoints) put
//! their reasoning in the visible text between literal `<think>` tags. Clients
//! that did not opt in (`X-VKDG-Think-Tags: include`) do not want it.
//!
//! The filter works on raw bytes and leaves every chunk without a complete
//! `<think>…</think>` pair on one line untouched: same bytes, same allocation,
//! no SSE parsing. Parsing and re-serialising each event just to look for a tag
//! cost a copy of the whole stream and rewrote the upstream's framing
//! (dropped comments, normalised spacing).
//!
//! Known limits, shared with the previous parser-based filter: a block whose two
//! tags arrive in different chunks, or on different lines, is not removed.

use bytes::Bytes;
use futures::StreamExt;

use super::relay::ByteStream;

const OPEN: &[u8] = b"<think>";
const CLOSE: &[u8] = b"</think>";

/// Applies [`strip_think_blocks`] to every chunk of `body`.
pub(super) fn strip_think_tags_stream(body: ByteStream) -> ByteStream {
    Box::pin(body.map(|item| item.map(strip_think_blocks)))
}

/// Removes each `<think>…</think>` pair that sits on one line of `chunk`.
/// Returns `chunk` itself, not a copy, when there is nothing to remove.
fn strip_think_blocks(chunk: Bytes) -> Bytes {
    // JSON text rarely holds a `<`; this is the whole cost for most chunks.
    if !chunk.contains(&b'<') {
        return chunk;
    }
    let mut out: Option<Vec<u8>> = None;
    let mut rest: &[u8] = &chunk;
    while let Some(open) = find(rest, OPEN) {
        let after_open = open + OPEN.len();
        let Some(close) = find(&rest[after_open..], CLOSE) else {
            break;
        };
        let body_end = after_open + close;
        let kept = out.get_or_insert_with(|| Vec::with_capacity(chunk.len()));
        if rest[after_open..body_end].contains(&b'\n') {
            // Removing across lines could cut an SSE event in half: keep the opener.
            kept.extend_from_slice(&rest[..after_open]);
            rest = &rest[after_open..];
        } else {
            kept.extend_from_slice(&rest[..open]);
            rest = &rest[body_end + CLOSE.len()..];
        }
    }
    match out {
        Some(mut kept) => {
            kept.extend_from_slice(rest);
            Bytes::from(kept)
        }
        None => chunk,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(s: &str) -> String {
        String::from_utf8(strip_think_blocks(Bytes::copy_from_slice(s.as_bytes())).to_vec())
            .unwrap_or_default()
    }

    // Refutes: parsing and re-serialising every event. The upstream's own bytes
    // (comment line, `data:` without a space) must reach the client as the same
    // allocation, not an equivalent rewrite.
    #[test]
    fn chunk_without_a_think_pair_is_forwarded_as_the_same_allocation() {
        let chunk = Bytes::from_static(b": keepalive\n\ndata:{\"a\":1}\n\nevent: x\ndata: <b>\n\n");
        let out = strip_think_blocks(chunk.clone());
        assert_eq!(out, chunk);
        assert_eq!(out.as_ptr(), chunk.as_ptr(), "chunk was copied");
    }

    // Refutes: not stripping at all (scenario think_tags_stripped_by_default), and
    // a strip that breaks the JSON around the removed text.
    #[test]
    fn think_pair_inside_a_json_string_is_removed_and_the_json_stays_valid() {
        let out = strip("data: {\"delta\":{\"content\":\"<think>reasoning</think>answer\"}}\n\n");
        assert_eq!(out, "data: {\"delta\":{\"content\":\"answer\"}}\n\n");
    }

    // Refutes: an empty block surviving, or only the first of several pairs removed.
    #[test]
    fn empty_and_repeated_pairs_are_all_removed() {
        assert_eq!(strip("a<think></think>b<think>x</think>c"), "abc");
    }

    // Refutes: removing from an opener to a closer that lives on a later line,
    // which would delete the event boundary between them.
    #[test]
    fn pair_spanning_lines_is_left_alone() {
        let input = "data: a<think>x\n\ndata: y</think>b\n\n";
        assert_eq!(strip(input), input);
    }

    // Refutes: an unclosed opener swallowing the rest of the chunk.
    #[test]
    fn unclosed_opener_is_left_alone() {
        let input = "data: {\"t\":\"<think>still thinking\"}\n\n";
        assert_eq!(strip(input), input);
    }

    // Refutes: a skipped cross-line opener hiding a valid pair that follows it.
    #[test]
    fn valid_pair_after_a_cross_line_opener_is_still_removed() {
        assert_eq!(
            strip("<think>a\n</think>b<think>c</think>d"),
            "<think>a\n</think>bd"
        );
    }
}
