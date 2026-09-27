//! Script to generate the eventstream fixture.
//! Run with: cargo run --example generate_fixture

use std::io::Write;

fn main() {
    let mut fixture = Vec::new();
    
    // Helper to add a frame
    fn add_frame(fixture: &mut Vec<u8>, headers: &[(&str, u8, &[u8])], payload: &[u8]) {
        let mut headers_data = Vec::new();
        for (name, value_type, value) in headers {
            headers_data.push((*name).len() as u8);
            headers_data.extend_from_slice(name.as_bytes());
            headers_data.push(*value_type);
            headers_data.extend_from_slice(value);
        }
        
        let headers_len = headers_data.len() as u32;
        let payload_len = payload.len() as u32;
        let total_len = headers_len + payload_len + 4; // +4 for crc placeholder
        
        // Prelude: total_len + headers_len
        let mut prelude = Vec::new();
        prelude.extend_from_slice(&total_len.to_be_bytes());
        prelude.extend_from_slice(&headers_len.to_be_bytes());
        
        // Prelude CRC (simplified - just 4 bytes)
        let prelude_crc = crc32(&prelude);
        
        // Message CRC (headers + payload)
        let mut message = Vec::new();
        message.extend_from_slice(&headers_data);
        message.extend_from_slice(payload);
        let message_crc = crc32(&message);
        
        // Write frame
        fixture.extend_from_slice(&total_len.to_be_bytes());
        fixture.extend_from_slice(&headers_len.to_be_bytes());
        fixture.extend_from_slice(&prelude_crc.to_be_bytes());
        fixture.extend_from_slice(&headers_data);
        fixture.extend_from_slice(payload);
        fixture.extend_from_slice(&message_crc.to_be_bytes());
    }
    
    // CRC32 implementation (simplified)
    fn crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFFFFFF;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB88320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    
    // Frame 1: initial-response (Started)
    add_frame(&mut fixture, &[(":event-type", 7, b"initial-response")], b"{}");
    
    // Frame 2: assistantResponseEvent with "hello" (OutputDelta)
    add_frame(&mut fixture, &[(":event-type", 7, b"assistantResponseEvent")], b"{\"content\": \"hello\"}");
    
    // Frame 3: assistantResponseEvent with " from kiro" (OutputDelta)
    add_frame(&mut fixture, &[(":event-type", 7, b"assistantResponseEvent")], b"{\"content\": \" from kiro\"}");
    
    // Frame 4: contextUsageEvent (Usage)
    add_frame(&mut fixture, &[(":event-type", 7, b"contextUsageEvent")], b"{\"contextUsagePercentage\": 0.5}");
    
    // Frame 5: metadataEvent with END_TURN (Completed)
    add_frame(&mut fixture, &[(":event-type", 7, b"metadataEvent")], b"{\"stopReason\": \"END_TURN\"}");
    
    // Frame 6: meteringEvent (ignored)
    add_frame(&mut fixture, &[(":event-type", 7, b"meteringEvent")], b"{}");
    
    std::fs::write("tests/fixtures/generate_assistant_response.eventstream", &fixture).unwrap();
    println!("Generated fixture with {} bytes", fixture.len());
}