//! AWS `EventStream` binary framing (`application/vnd.amazon.eventstream`), as used
//! by Kiro / `CodeWhisperer` `generateAssistantResponse`.
//!
//! Message layout (all integers big-endian):
//! - prelude: `total_len: u32` (whole message, including prelude and trailing CRC),
//!   `headers_len: u32`, `prelude_crc: u32` (CRC32 of bytes `0..8`)
//! - headers: `headers_len` bytes of `name_len: u8, name, type: u8, value`
//! - payload: `total_len - headers_len - 16` bytes
//! - `message_crc: u32` (CRC32 of bytes `0..total_len - 4`)
//!
//! Both CRCs are validated. A corrupted stream cannot be resynchronised reliably,
//! so the first error poisons the parser.

use bytes::{Buf, Bytes, BytesMut};

const PRELUDE_LEN: usize = 12;
const CRC_LEN: usize = 4;
const MIN_MESSAGE_LEN: usize = PRELUDE_LEN + CRC_LEN;
/// Upper bound on a single message; matches the AWS SDK limit (16 MiB).
const MAX_MESSAGE_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub headers: Vec<FrameHeader>,
    pub payload: Bytes,
}

impl Frame {
    /// Value of a string header, if present.
    pub fn header_str(&self, name: &str) -> Option<&str> {
        self.headers.iter().find_map(|h| match &h.value {
            HeaderValue::String(s) if h.name == name => Some(s.as_str()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub name: String,
    pub value: HeaderValue,
}

/// AWS `EventStream` header value; discriminants follow the wire type byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderValue {
    Bool(bool),
    Byte(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    ByteArray(Bytes),
    String(String),
    /// Milliseconds since the Unix epoch.
    Timestamp(i64),
    Uuid([u8; 16]),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("invalid message length {0}")]
    InvalidLength(usize),
    #[error("headers length {headers_len} exceeds message length {total_len}")]
    HeadersOverflow {
        headers_len: usize,
        total_len: usize,
    },
    #[error("prelude CRC mismatch: frame {expected:#010x}, computed {computed:#010x}")]
    PreludeCrc { expected: u32, computed: u32 },
    #[error("message CRC mismatch: frame {expected:#010x}, computed {computed:#010x}")]
    MessageCrc { expected: u32, computed: u32 },
    #[error("malformed header: {0}")]
    MalformedHeader(&'static str),
    #[error("unknown header value type {0}")]
    UnknownHeaderType(u8),
    #[error("stream ended inside a frame ({0} trailing bytes)")]
    Truncated(usize),
}

/// Incremental parser; accepts arbitrary chunk boundaries.
#[derive(Debug, Default)]
pub struct EventStreamParser {
    buffer: BytesMut,
    failed: bool,
}

impl EventStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends bytes and returns every complete frame now available, in order.
    ///
    /// An error is the last item: the parser is then poisoned and yields nothing
    /// further, since a corrupted stream cannot be resynchronised reliably.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Result<Frame, FrameError>> {
        let mut frames = Vec::new();
        if self.failed {
            return frames;
        }
        self.buffer.extend_from_slice(chunk);
        loop {
            match self.next_frame() {
                Ok(Some(frame)) => frames.push(Ok(frame)),
                Ok(None) => return frames,
                Err(err) => {
                    self.failed = true;
                    self.buffer.clear();
                    frames.push(Err(err));
                    return frames;
                }
            }
        }
    }

    /// Checks that the stream ended on a frame boundary.
    pub fn finish(&mut self) -> Result<(), FrameError> {
        if self.failed || self.buffer.is_empty() {
            return Ok(());
        }
        let trailing = self.buffer.len();
        self.failed = true;
        self.buffer.clear();
        Err(FrameError::Truncated(trailing))
    }

    fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buffer.len() < PRELUDE_LEN {
            return Ok(None);
        }
        let total_len = be_u32(&self.buffer[0..4]) as usize;
        let headers_len = be_u32(&self.buffer[4..8]) as usize;
        let expected = be_u32(&self.buffer[8..12]);
        let computed = crc32fast::hash(&self.buffer[0..8]);
        if expected != computed {
            return Err(FrameError::PreludeCrc { expected, computed });
        }
        if !(MIN_MESSAGE_LEN..=MAX_MESSAGE_LEN).contains(&total_len) {
            return Err(FrameError::InvalidLength(total_len));
        }
        if headers_len > total_len - MIN_MESSAGE_LEN {
            return Err(FrameError::HeadersOverflow {
                headers_len,
                total_len,
            });
        }
        if self.buffer.len() < total_len {
            return Ok(None);
        }

        let body_end = total_len - CRC_LEN;
        let expected = be_u32(&self.buffer[body_end..total_len]);
        let computed = crc32fast::hash(&self.buffer[..body_end]);
        if expected != computed {
            return Err(FrameError::MessageCrc { expected, computed });
        }

        let mut message = self.buffer.split_to(total_len).freeze();
        message.advance(PRELUDE_LEN);
        let headers = parse_headers(message.split_to(headers_len))?;
        message.truncate(message.len() - CRC_LEN);
        Ok(Some(Frame {
            headers,
            payload: message,
        }))
    }
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn take(buf: &mut Bytes, n: usize, what: &'static str) -> Result<Bytes, FrameError> {
    if buf.len() < n {
        return Err(FrameError::MalformedHeader(what));
    }
    Ok(buf.split_to(n))
}

fn take_array<const N: usize>(buf: &mut Bytes, what: &'static str) -> Result<[u8; N], FrameError> {
    let bytes = take(buf, N, what)?;
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn utf8(bytes: &Bytes, what: &'static str) -> Result<String, FrameError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| FrameError::MalformedHeader(what))
}

fn parse_headers(mut buf: Bytes) -> Result<Vec<FrameHeader>, FrameError> {
    let mut headers = Vec::new();
    while !buf.is_empty() {
        let name_len = take_array::<1>(&mut buf, "name length")?[0] as usize;
        let name = utf8(&take(&mut buf, name_len, "name")?, "name is not UTF-8")?;
        let value = match take_array::<1>(&mut buf, "value type")?[0] {
            0 => HeaderValue::Bool(true),
            1 => HeaderValue::Bool(false),
            2 => HeaderValue::Byte(i8::from_be_bytes(take_array(&mut buf, "byte")?)),
            3 => HeaderValue::Int16(i16::from_be_bytes(take_array(&mut buf, "int16")?)),
            4 => HeaderValue::Int32(i32::from_be_bytes(take_array(&mut buf, "int32")?)),
            5 => HeaderValue::Int64(i64::from_be_bytes(take_array(&mut buf, "int64")?)),
            6 => {
                let len = u16::from_be_bytes(take_array(&mut buf, "bytes length")?) as usize;
                HeaderValue::ByteArray(take(&mut buf, len, "bytes value")?)
            }
            7 => {
                let len = u16::from_be_bytes(take_array(&mut buf, "string length")?) as usize;
                HeaderValue::String(utf8(
                    &take(&mut buf, len, "string value")?,
                    "string is not UTF-8",
                )?)
            }
            8 => HeaderValue::Timestamp(i64::from_be_bytes(take_array(&mut buf, "timestamp")?)),
            9 => HeaderValue::Uuid(take_array(&mut buf, "uuid")?),
            other => return Err(FrameError::UnknownHeaderType(other)),
        };
        headers.push(FrameHeader { name, value });
    }
    Ok(headers)
}
