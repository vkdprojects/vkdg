//! AWS Event Stream binary framing parser.
//!
//! Frame format:
//! - Prelude: total_len (4) + headers_len (4) + prelude_crc (4)
//! - Headers: variable length
//! - Payload: variable length
//! - Message CRC: 4 bytes
//!
//! Header format (Kiro/Amazon):
//! - header_type: u8 (always 7=String for name)
//! - name_len: u8
//! - name: [u8; name_len]
//! - value_type: u8
//! - value: depends on value_type

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
        if self.buffer.len() - self.pos < 12 {
            return None;
        }

        let start = self.pos;

        // Read prelude: total_length (4 bytes) + headers_length (4 bytes)
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

        self.pos += 8;

        // Frame size = total_len (the total length of everything after the 8-byte prelude)
        // This includes: prelude_crc(4) + headers + payload + message_crc(4)
        let frame_size = total_len;
        
        if self.buffer.len() - start < frame_size {
            self.pos = start;
            return None;
        }

        // Skip prelude_crc
        self.pos += 4;

        // Parse headers
        let headers = self.parse_headers(headers_len)?;

        // Payload: frame_size - prelude(8) - prelude_crc(4) - headers_len - message_crc(4)
        let payload_len = frame_size - 8 - 4 - headers_len - 4;
        let payload = self.buffer[self.pos..self.pos + payload_len].to_vec();
        self.pos += payload_len;
        
        // Skip message_crc
        self.pos += 4;

        Some(Frame { headers, payload })
    }
    
    fn parse_headers(&mut self, len: usize) -> Option<Vec<FrameHeader>> {
        let end = self.pos + len;
        if end > self.buffer.len() {
            return None;
        }

        let mut headers = Vec::new();
        while self.pos < end {
            // Kiro/Amazon header format:
            // header_type (1 byte, always 7=String for name)
            // + name_len (1 byte) + name (N bytes)
            // + value_type (1 byte) + value (depends on type)
            let _header_type = self.buffer[self.pos];
            self.pos += 1;

            if self.pos >= end {
                return None;
            }

            let name_len = self.buffer[self.pos] as usize;
            self.pos += 1;

            if self.pos + name_len > end {
                return None;
            }

            let name = String::from_utf8_lossy(
                &self.buffer[self.pos..self.pos + name_len],
            )
            .to_string();
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
                    let v = u16::from_be_bytes([
                        self.buffer[self.pos],
                        self.buffer[self.pos + 1],
                    ]);
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
                    let len = u16::from_be_bytes([
                        self.buffer[self.pos],
                        self.buffer[self.pos + 1],
                    ]) as usize;
                    self.pos += 2;
                    if self.pos + len > end {
                        return None;
                    }
                    let v = self.buffer[self.pos..self.pos + len].to_vec();
                    self.pos += len;
                    HeaderValue::ByteArray(v)
                }
                7 => {
                    // Kiro String value: u8 length prefix + bytes + trailing NUL terminator
                    // (NOT AWS-spec u16 BE length — confirmed against real Kiro fixture bytes)
                    let len = self.buffer[self.pos] as usize;
                    self.pos += 1;
                    if self.pos + len > end {
                        return None;
                    }
                    let v = String::from_utf8_lossy(
                        &self.buffer[self.pos..self.pos + len],
                    )
                    .to_string();
                    self.pos += len;
                    // Skip trailing NUL terminator
                    if self.pos < end && self.buffer[self.pos] == 0 {
                        self.pos += 1;
                    }
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
                    uuid.copy_from_slice(
                        &self.buffer[self.pos..self.pos + 16],
                    );
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
        // Minimal valid frame with one header
        let mut data = Vec::new();
        
        // Prelude: total_len (8 + 4 + header + 4) + headers_len
        let header_data = vec![
            7, // header_type (always 7=String for header names)
            4, // name_len
            b't', b'e', b's', b't', // name "test"
            7, // value_type = String
            0, 5, // value_len (5)
            b'h', b'e', b'l', b'l', b'o', // value "hello"
        ];
        
        let total_len = 8 + 4 + header_data.len() as u32 + 4;
        let headers_len = header_data.len() as u32;
        
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
    }
}
