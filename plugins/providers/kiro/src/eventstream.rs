//! AWS Event Stream–derived binary framing parser for Kiro's wire protocol.
//!
//! Frame format (verified byte-for-byte against a real Kiro API capture):
//! - total_len: u32 BE — headers_len + payload_len + 4 (message_crc size).
//!   NOTE: this does NOT include the 8-byte prelude fields (total_len +
//!   headers_len) or the 4-byte prelude_crc, unlike the generic AWS Event
//!   Stream spec's `total_length` field.
//! - headers_len: u32 BE
//! - prelude_crc: u32 BE (not validated)
//! - headers: headers_len bytes, see below
//! - payload: (total_len - headers_len - 4) bytes
//! - message_crc: u32 BE (not validated)
//!
//! Header entry format:
//! - name_len: u8
//! - name: [u8; name_len]
//! - value_type: u8
//! - value: depends on value_type
//!   - String (7): NO length prefix. Runs to the end of the header block
//!     (consumes all remaining bytes up to headers_len). Confirmed against
//!     two real frames with different string lengths (16 and 22 bytes) —
//!     Kiro omits the u16 BE length the generic AWS spec defines for string
//!     header values.
//!   - Int32 (3): i32 BE, 4 bytes
//!   - Int64 (4): i64 BE, 8 bytes
//!   - ByteArray (5): u16 BE length + bytes (spec-compliant, unverified
//!     against real Kiro data — Kiro only ever sends String headers)

use bytes::Bytes;

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub headers: Vec<FrameHeader>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameHeader {
    pub name: String,
    pub value: HeaderValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeaderValue {
    Bool(bool),
    Byte(u8),
    Int16(u16),
    Int32(i32),
    Int64(i64),
    ByteArray(Vec<u8>),
    String(String),
    Timestamp(i64),
    Uuid([u8; 16]),
}

pub struct EventStreamParser {
    buffer: Vec<u8>,
    pos: usize,
}

impl EventStreamParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            pos: 0,
        }
    }

    pub fn feed(&mut self, chunk: Bytes) -> Vec<Frame> {
        // Append new chunk to buffer
        self.buffer.extend_from_slice(&chunk);

        let mut frames = Vec::new();

        // Try to parse as many frames as possible
        while let Some(frame) = self.parse_next_frame() {
            frames.push(frame);
        }

        // Reset buffer for next feed
        if self.pos > 0 {
            self.buffer.drain(..self.pos);
            self.pos = 0;
        }

        frames
    }

    /// Parse next frame. Returns None if buffer incomplete.
    fn parse_next_frame(&mut self) -> Option<Frame> {
        if self.buffer.len() - self.pos < 8 {
            return None;
        }

        let start = self.pos;

        let total_len = u32::from_be_bytes([
            self.buffer[self.pos],
            self.buffer[self.pos + 1],
            self.buffer[self.pos + 2],
            self.buffer[self.pos + 3],
        ]) as usize;

        let headers_len = u32::from_be_bytes([
            self.buffer[self.pos + 4],
            self.buffer[self.pos + 5],
            self.buffer[self.pos + 6],
            self.buffer[self.pos + 7],
        ]) as usize;

        // total_len = headers_len + payload_len + message_crc(4); reject frames
        // where that doesn't leave room for the message_crc.
        if total_len < headers_len + 4 {
            self.pos = start;
            return None;
        }
        let payload_len = total_len - headers_len - 4;

        // Bytes needed from `start`: prelude(8) + prelude_crc(4) + headers_len + payload_len + message_crc(4)
        let frame_total = 16 + headers_len + payload_len;
        if self.buffer.len() - start < frame_total {
            self.pos = start;
            return None;
        }

        self.pos += 8; // consumed total_len + headers_len fields
        self.pos += 4; // skip prelude_crc

        let headers = self.parse_headers(headers_len)?;

        let payload = self.buffer[self.pos..self.pos + payload_len].to_vec();
        self.pos += payload_len;

        self.pos += 4; // skip message_crc

        Some(Frame { headers, payload })
    }

    fn parse_headers(&mut self, len: usize) -> Option<Vec<FrameHeader>> {
        let end = self.pos + len;
        if end > self.buffer.len() {
            return None;
        }

        let mut headers = Vec::new();
        while self.pos < end {
            // Kiro header entry: name_len(1) + name(N) + value_type(1) + value.
            // No leading header_type byte (deviates from generic AWS Event
            // Stream headers, which prefix each entry with a type byte).
            let name_len = self.buffer[self.pos] as usize;
            self.pos += 1;

            if self.pos + name_len > end {
                return None;
            }

            let name =
                String::from_utf8_lossy(&self.buffer[self.pos..self.pos + name_len]).to_string();
            self.pos += name_len;

            if self.pos >= end {
                return None;
            }

            // Value type — determines how to parse the value
            let value_type = self.buffer[self.pos];
            self.pos += 1;

            let value = match value_type {
                0 => {
                    let v = self.buffer[self.pos] != 0;
                    self.pos += 1;
                    HeaderValue::Bool(v)
                }
                1 => {
                    let v = self.buffer[self.pos];
                    self.pos += 1;
                    HeaderValue::Byte(v)
                }
                2 => {
                    let v = u16::from_be_bytes([self.buffer[self.pos], self.buffer[self.pos + 1]]);
                    self.pos += 2;
                    HeaderValue::Int16(v)
                }
                3 => {
                    let v = i32::from_be_bytes([
                        self.buffer[self.pos],
                        self.buffer[self.pos + 1],
                        self.buffer[self.pos + 2],
                        self.buffer[self.pos + 3],
                    ]);
                    self.pos += 4;
                    HeaderValue::Int32(v)
                }
                4 => {
                    let v = i64::from_be_bytes([
                        self.buffer[self.pos],
                        self.buffer[self.pos + 1],
                        self.buffer[self.pos + 2],
                        self.buffer[self.pos + 3],
                        self.buffer[self.pos + 4],
                        self.buffer[self.pos + 5],
                        self.buffer[self.pos + 6],
                        self.buffer[self.pos + 7],
                    ]);
                    self.pos += 8;
                    HeaderValue::Int64(v)
                }
                5 => {
                    let len = u16::from_be_bytes([self.buffer[self.pos], self.buffer[self.pos + 1]])
                        as usize;
                    self.pos += 2;
                    if self.pos + len > end {
                        return None;
                    }
                    let v = self.buffer[self.pos..self.pos + len].to_vec();
                    self.pos += len;
                    HeaderValue::ByteArray(v)
                }
                7 => {
                    // Kiro String value: no length prefix, no NUL terminator.
                    // Consumes all remaining bytes in the header block.
                    let v = String::from_utf8_lossy(&self.buffer[self.pos..end]).to_string();
                    self.pos = end;
                    HeaderValue::String(v)
                }
                8 => {
                    let v = i64::from_be_bytes([
                        self.buffer[self.pos],
                        self.buffer[self.pos + 1],
                        self.buffer[self.pos + 2],
                        self.buffer[self.pos + 3],
                        self.buffer[self.pos + 4],
                        self.buffer[self.pos + 5],
                        self.buffer[self.pos + 6],
                        self.buffer[self.pos + 7],
                    ]);
                    self.pos += 8;
                    HeaderValue::Timestamp(v)
                }
                10 => {
                    let mut uuid = [0u8; 16];
                    uuid.copy_from_slice(&self.buffer[self.pos..self.pos + 16]);
                    self.pos += 16;
                    HeaderValue::Uuid(uuid)
                }
                _ => {
                    self.pos = end;
                    break;
                }
            };

            headers.push(FrameHeader { name, value });
        }

        Some(headers)
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.pos = 0;
    }
}

impl Default for EventStreamParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_frame() {
        // Minimal valid frame with one header, no payload
        let mut data = Vec::new();

        let header_data = vec![
            4, // name_len
            b't', b'e', b's', b't', // name "test"
            7,    // value_type = String
            b'h', b'e', b'l', b'l', b'o', // value "hello" (no length prefix, no NUL)
        ];

        let headers_len = header_data.len() as u32;
        let payload_len: u32 = 0;
        // total_len = headers_len + payload_len + message_crc(4)
        let total_len = headers_len + payload_len + 4;

        data.extend_from_slice(&total_len.to_be_bytes());
        data.extend_from_slice(&headers_len.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes()); // prelude_crc
        data.extend_from_slice(&header_data);
        data.extend_from_slice(&0u32.to_be_bytes()); // message_crc

        let mut parser = EventStreamParser::new();
        let frames = parser.feed(Bytes::from(data));

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].headers.len(), 1);
        assert_eq!(frames[0].headers[0].name, "test");
        assert_eq!(
            frames[0].headers[0].value,
            HeaderValue::String("hello".to_string())
        );
    }
}
