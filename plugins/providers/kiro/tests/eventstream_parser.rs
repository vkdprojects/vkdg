//! Integration tests for AWS Event Stream parser.

use vkdg_provider_kiro::eventstream::EventStreamParser;

#[test]
fn test_parse_fixture() {
    let data = include_bytes!("../tests/fixtures/generate_assistant_response.eventstream");
    eprintln!("Fixture size: {} bytes", data.len());
    
    // Print first few bytes in hex
    eprintln!("First 32 bytes: {:02x?}", &data[..32.min(data.len())]);
    
    let chunk = bytes::Bytes::from(data.to_vec());
    
    let mut parser = EventStreamParser::new();
    let frames = parser.feed(chunk);
    
    eprintln!("Number of frames parsed: {}", frames.len());
    
    // Verify we got 6 frames
    assert_eq!(frames.len(), 6, "Expected 6 frames, got {}", frames.len());
}

#[test]
fn test_incremental_parsing() {
    let data = include_bytes!("../tests/fixtures/generate_assistant_response.eventstream");
    
    let mut parser = EventStreamParser::new();
    
    // Feed in small chunks (less than one frame)
    let mut all_frames = Vec::new();
    for (i, chunk) in data.chunks(50).enumerate() {
        let frames = parser.feed(bytes::Bytes::from(chunk.to_vec()));
        eprintln!("Chunk {}: got {} frames", i, frames.len());
        all_frames.extend(frames);
    }
    
    eprintln!("Total frames: {}", all_frames.len());
    assert_eq!(all_frames.len(), 6);
}

#[test]
fn test_fragment_in_middle_of_frame() {
    let data = include_bytes!("../tests/fixtures/generate_assistant_response.eventstream");
    
    // Find a position in the middle of first frame (after prelude)
    let mut parser = EventStreamParser::new();
    
    // Feed first 100 bytes (middle of first frame)
    let frames1 = parser.feed(bytes::Bytes::from(data[..100].to_vec()));
    eprintln!("After first 100 bytes: {} frames", frames1.len());
    // Should not get complete frame yet - we're in the middle of a frame
    // The parser should handle this gracefully
    
    // Feed rest
    let frames2 = parser.feed(bytes::Bytes::from(data[100..].to_vec()));
    eprintln!("After remaining bytes: {} frames", frames2.len());
    // Now we should get frames
    assert!(!frames2.is_empty());
}