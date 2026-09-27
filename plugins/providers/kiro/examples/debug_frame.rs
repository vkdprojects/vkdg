use vkdg_provider_kiro::eventstream::EventStreamParser;
use bytes::Bytes;

fn main() {
    let fixture = include_bytes!("../tests/fixtures/generate_assistant_response.eventstream");
    let mut parser = EventStreamParser::new();
    let frames = parser.feed(Bytes::from(fixture.to_vec()));
    
    // Debug Frame 1 (contentBlockDelta)
    if frames.len() > 1 {
        let f = &frames[1];
        println!("Frame 1 debug:");
        println!("  Payload len: {}", f.payload.len());
        println!("  Payload bytes: {:?}", f.payload);
        println!("  Payload text: {}", String::from_utf8_lossy(&f.payload));
        
        // Try to parse as JSON
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&f.payload) {
            println!("  Parsed JSON: {:?}", value);
            if let Some(delta) = value.get("delta") {
                println!("  Delta: {:?}", delta);
                if let Some(text) = delta.get("text") {
                    println!("  Text: {:?}", text);
                }
            }
        }
    }
}
