use vkdg_provider_kiro::eventstream::{EventStreamParser, HeaderValue};

fn main() {
    let data = include_bytes!("../tests/fixtures/generate_assistant_response.eventstream");
    println!("Fixture size: {} bytes", data.len());
    
    // Print first 120 bytes in hex with indices
    println!("\nFirst 120 bytes:");
    for i in (0..120.min(data.len())).step_by(16) {
        let chunk = &data[i..(i+16).min(data.len())];
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        let chars: String = chunk.iter().map(|&b| {
            if b >= 32 && b < 127 { b as char } else { '.' }
        }).collect();
        println!("{:04x}: {:48}  {}", i, hex.join(" "), chars);
    }
    
    // Print prelude values
    println!("\n=== Prelude ===");
    let total_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let headers_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    println!("total_length: {} (0x{:x})", total_len, total_len);
    println!("headers_length: {} (0x{:x})", headers_len, headers_len);
    println!("prelude_crc: {:02x}{:02x}{:02x}{:02x}", data[8], data[9], data[10], data[11]);
    
    // Headers start at byte 12
    println!("\n=== Headers (starting at byte 12) ===");
    let mut pos = 12;
    let end_of_headers = 12 + headers_len as usize;
    
    while pos < end_of_headers && pos < data.len() {
        // AWS Event Stream header format:
        // value_type (1 byte) + name_len (1 byte) + name + value
        let value_type = data[pos];
        println!("pos {}: value_type = {}", pos, value_type);
        pos += 1;
        
        if pos >= data.len() {
            println!("  ERROR: no room for name_len!");
            break;
        }
        
        let name_len = data[pos] as usize;
        println!("  name_len = {}", name_len);
        pos += 1;
        
        if pos + name_len > data.len() {
            println!("  ERROR: name extends past buffer!");
            break;
        }
        
        let name = String::from_utf8_lossy(&data[pos..pos+name_len]);
        println!("  name = \"{}\"", name);
        pos += name_len;
        
        match value_type {
            0 => { // boolean
                println!("  boolean: {}", data[pos] != 0);
                pos += 1;
            }
            7 => { // string
                if pos + 2 > data.len() {
                    println!("  ERROR: no room for string length!");
                    break;
                }
                let str_len = u16::from_be_bytes([data[pos], data[pos+1]]) as usize;
                println!("  string length = {}", str_len);
                pos += 2;
                if pos + str_len > data.len() {
                    println!("  ERROR: string extends past buffer!");
                    break;
                }
                let s = String::from_utf8_lossy(&data[pos..pos+str_len]);
                println!("  string value = \"{}\"", s);
                pos += str_len;
            }
            _ => {
                println!("  Unknown type, stopping");
                break;
            }
        }
    }
    
    println!("\n=== Parsing with parser ===");
    let chunk = bytes::Bytes::from(data.to_vec());
    
    let mut parser = EventStreamParser::new();
    let frames = parser.feed(chunk);
    
    println!("Number of frames parsed: {}", frames.len());
    
    for (i, f) in frames.iter().enumerate() {
        let event_type = f.headers.iter()
            .find(|h| h.name == ":event-type")
            .and_then(|h| match &h.value {
                HeaderValue::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("N/A");
        let payload_str = String::from_utf8_lossy(&f.payload);
        println!("Frame {}: type={}, payload={}", i, event_type, payload_str);
    }
}
