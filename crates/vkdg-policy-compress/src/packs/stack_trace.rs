//! Stack trace filter pack.
//!
//! Handles Java/JVM (`\tat `) and Python (`File "…"`) stack traces.
//! Keeps the error message and the first 5 frames; elides the rest.
//! Estimated reduction: 60-80% on deep call stacks.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses stack trace output.
///
/// Retains the exception/error header and at most 5 stack frames.
/// Deeper frames are replaced with a single `"... [stack truncated] ..."` marker.
pub struct StackTracePack;

impl FilterPack for StackTracePack {
    fn id(&self) -> &str {
        "rtk:stack-trace"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::StackTrace
    }

    fn description(&self) -> &str {
        "Compresses stack traces; keeps error header + first 5 frames"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        70
    }

    fn apply(&self, text: &str) -> String {
        let mut kept: Vec<String> = Vec::new();
        let mut frame_count = 0usize;
        for line in text.lines() {
            if line.contains("\tat ") || line.contains("  File \"") {
                frame_count += 1;
                if frame_count > 5 {
                    if frame_count == 6 {
                        kept.push("... [stack truncated] ...".to_string());
                    }
                    continue;
                }
            }
            kept.push(line.to_string());
        }
        kept.join("\n")
    }
}
