//! JSON output filter pack.
//!
//! Handles pretty-printed JSON blobs. Re-serialises to compact (single-line)
//! form using `serde_json`. Falls back to trimmed original if parsing fails.
//! Estimated reduction: 30-50% on indented JSON.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses pretty-printed JSON to compact form.
///
/// Uses `serde_json` round-trip: parse → compact serialize.
/// Invalid JSON is returned trimmed but otherwise unchanged.
pub struct JsonOutputPack;

impl FilterPack for JsonOutputPack {
    fn id(&self) -> &str {
        "rtk:json-output"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::JsonOutput
    }

    fn description(&self) -> &str {
        "Compacts pretty-printed JSON to single-line form via serde_json"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        40
    }

    fn apply(&self, text: &str) -> String {
        let joined = text.trim().to_string();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&joined) {
            serde_json::to_string(&val).unwrap_or(joined)
        } else {
            joined
        }
    }
}
