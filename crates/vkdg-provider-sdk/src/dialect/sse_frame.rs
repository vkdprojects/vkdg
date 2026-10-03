//! Incremental, bounded Server-Sent Events framing.
//!
//! [`SseFramer`] turns arbitrary byte chunks into dispatched events. It follows the
//! WHATWG SSE line rules (LF, CRLF and lone CR terminators, `:` comments, one optional
//! space after the field colon, `id`/`retry` ignored) and adds two gateway guarantees:
//!
//! * **Bounded memory.** One event never buffers more than the configured limit. An
//!   over-limit event yields exactly one [`FrameError::EventTooLarge`]; the rest of that
//!   event is discarded up to its blank line and later events frame normally.
//! * **No copy for whole events.** When no partial event is buffered, complete events are
//!   framed straight out of the caller's chunk and borrowed; only an incomplete tail is
//!   copied. Bytes are validated as UTF-8 (lossily) once an event is complete, so a
//!   multi-byte character split across chunks is never garbled.

use std::borrow::Cow;

/// Default bound on one event's size, in bytes.
pub const DEFAULT_MAX_EVENT_BYTES: usize = 1 << 20;

/// One dispatched SSE event. Borrowed from the chunk (or the framer's buffer) and valid
/// only inside the callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame<'a> {
    /// Value of the last `event:` line; `None` when absent or empty.
    pub event: Option<&'a str>,
    /// `data:` lines joined with `\n`. Borrowed when the event has a single data line.
    pub data: Cow<'a, str>,
}

/// Why an event could not be framed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The event grew past the configured bound before its blank line arrived.
    #[error("SSE event exceeds {limit} bytes")]
    EventTooLarge { limit: usize },
}

/// Incremental SSE framer. One instance per upstream stream.
///
/// An event's size is every byte from its first line up to, but not including, its closing
/// blank line.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)] // independent line-ending scanner flags
pub struct SseFramer {
    /// Bytes of the current, still incomplete event.
    buf: Vec<u8>,
    limit: usize,
    /// The last consumed byte ended a line (or the event/stream just started).
    at_line_start: bool,
    /// The last consumed byte was a CR, so an LF that follows is half of a CRLF.
    after_cr: bool,
    /// A blank line ended with CR; an LF at the start of the next chunk belongs to it.
    skip_lf: bool,
    /// The current event overflowed and is being thrown away up to its blank line.
    discarding: bool,
}

/// Outcome of scanning for the end of the current event.
enum Scan {
    /// The blank line that closes the event starts at `event_end`; the next event at `next`.
    BlankLine { event_end: usize, next: usize },
    /// No blank line in the scanned bytes.
    Incomplete,
}

impl SseFramer {
    /// A framer bounded at [`DEFAULT_MAX_EVENT_BYTES`].
    pub fn new() -> Self {
        Self::with_max_event_bytes(DEFAULT_MAX_EVENT_BYTES)
    }

    /// A framer that rejects any event larger than `limit` bytes.
    pub fn with_max_event_bytes(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
            at_line_start: true,
            after_cr: false,
            skip_lf: false,
            discarding: false,
        }
    }

    /// Calls `on_frame` for every complete event in `chunk` (plus any buffered remainder),
    /// in order. Events with no `data:` line produce no call.
    pub fn push(
        &mut self,
        chunk: &[u8],
        mut on_frame: impl FnMut(Result<SseFrame<'_>, FrameError>),
    ) {
        let mut pos = 0;
        while pos < chunk.len() {
            if self.skip_lf {
                self.skip_lf = false;
                if chunk[pos] == b'\n' {
                    pos += 1;
                    continue;
                }
            }
            match self.scan(chunk, pos) {
                Scan::BlankLine { event_end, next } => {
                    if self.discarding {
                        self.discarding = false;
                    } else {
                        self.complete_event(&chunk[pos..event_end], &mut on_frame);
                    }
                    pos = next;
                }
                Scan::Incomplete => {
                    if !self.discarding {
                        self.retain_tail(&chunk[pos..], &mut on_frame);
                    }
                    return;
                }
            }
        }
    }

    /// End of stream: dispatches a trailing event whose blank line never arrived, then
    /// resets the framer.
    pub fn finish(&mut self, mut on_frame: impl FnMut(Result<SseFrame<'_>, FrameError>)) {
        if !self.discarding && !self.buf.is_empty() {
            parse_event(&self.buf, &mut on_frame);
        }
        self.buf.clear();
        self.at_line_start = true;
        self.after_cr = false;
        self.skip_lf = false;
        self.discarding = false;
    }

    /// Bytes currently retained for an incomplete event; never above the limit between calls.
    pub fn buffered_bytes(&self) -> usize {
        self.buf.len()
    }

    /// Finds the blank line that closes the current event, scanning `data` from `from`.
    fn scan(&mut self, data: &[u8], from: usize) -> Scan {
        let mut i = from;
        loop {
            let Some(skipped) = data[i..].iter().position(|&b| b == b'\n' || b == b'\r') else {
                if i < data.len() {
                    self.at_line_start = false;
                    self.after_cr = false;
                }
                return Scan::Incomplete;
            };
            if skipped > 0 {
                self.at_line_start = false;
                self.after_cr = false;
            }
            i += skipped;
            let byte = data[i];
            i += 1;
            if byte == b'\n' && self.after_cr {
                // Second half of a CRLF terminator.
                self.after_cr = false;
                continue;
            }
            if self.at_line_start {
                self.after_cr = false;
                self.skip_lf = byte == b'\r';
                return Scan::BlankLine {
                    event_end: i - 1,
                    next: i,
                };
            }
            self.at_line_start = true;
            self.after_cr = byte == b'\r';
        }
    }

    /// A closing blank line was found; `rest` is the part of the event in the current chunk.
    fn complete_event(
        &mut self,
        rest: &[u8],
        on_frame: &mut impl FnMut(Result<SseFrame<'_>, FrameError>),
    ) {
        if self.buf.is_empty() {
            if rest.len() > self.limit {
                on_frame(Err(self.too_large()));
            } else {
                parse_event(rest, on_frame);
            }
        } else if self.buf.len() + rest.len() > self.limit {
            self.buf.clear();
            on_frame(Err(self.too_large()));
        } else {
            self.buf.extend_from_slice(rest);
            parse_event(&self.buf, on_frame);
            self.buf.clear();
        }
    }

    /// No blank line yet: keep the tail for the next chunk, or give up on an oversized event.
    fn retain_tail(
        &mut self,
        tail: &[u8],
        on_frame: &mut impl FnMut(Result<SseFrame<'_>, FrameError>),
    ) {
        if self.buf.len() + tail.len() > self.limit {
            self.buf.clear();
            self.discarding = true;
            on_frame(Err(self.too_large()));
        } else {
            self.buf.extend_from_slice(tail);
        }
    }

    fn too_large(&self) -> FrameError {
        FrameError::EventTooLarge { limit: self.limit }
    }
}

impl Default for SseFramer {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses one complete event (its lines, without the closing blank line) and dispatches it
/// when it carries a `data` field.
fn parse_event(block: &[u8], on_frame: &mut impl FnMut(Result<SseFrame<'_>, FrameError>)) {
    let text = String::from_utf8_lossy(block);
    let mut event = None;
    let mut data = Data::None;
    for line in lines(&text) {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => event = Some(value).filter(|v| !v.is_empty()),
            "data" => data.push(value),
            _ => {}
        }
    }
    if let Some(data) = data.into_cow() {
        on_frame(Ok(SseFrame { event, data }));
    }
}

/// Lines of `text`, split on LF, CRLF or lone CR.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let bytes = rest.as_bytes();
        let Some(end) = bytes.iter().position(|&b| b == b'\n' || b == b'\r') else {
            return Some(std::mem::take(&mut rest));
        };
        let terminator = if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
            2
        } else {
            1
        };
        let line = &rest[..end];
        rest = &rest[end + terminator..];
        Some(line)
    })
}

/// `data:` accumulator that allocates only when an event has several data lines.
enum Data<'a> {
    None,
    One(&'a str),
    Many(String),
}

impl<'a> Data<'a> {
    fn push(&mut self, value: &'a str) {
        *self = match std::mem::replace(self, Self::None) {
            Self::None => Self::One(value),
            Self::One(first) => {
                let mut joined = String::with_capacity(first.len() + 1 + value.len());
                joined.push_str(first);
                joined.push('\n');
                joined.push_str(value);
                Self::Many(joined)
            }
            Self::Many(mut joined) => {
                joined.push('\n');
                joined.push_str(value);
                Self::Many(joined)
            }
        };
    }

    fn into_cow(self) -> Option<Cow<'a, str>> {
        match self {
            Self::None => None,
            Self::One(value) => Some(Cow::Borrowed(value)),
            Self::Many(joined) => Some(Cow::Owned(joined)),
        }
    }
}
