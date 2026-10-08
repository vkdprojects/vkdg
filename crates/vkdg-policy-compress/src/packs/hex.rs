//! Hex dump filter pack.
//!
//! Handles `xxd` / `hexdump` output. Shows only the first 8 rows since
//! hex dumps are rarely useful past the magic bytes / header region.
//! Estimated reduction: 80-95% on large binary blobs.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses hex dump output.
///
/// Keeps the first 8 lines and appends `"... [hex dump truncated]"` when the
/// dump is longer.
pub struct HexDumpPack;

impl FilterPack for HexDumpPack {
    fn id(&self) -> &str {
        "rtk:hex-dump"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::HexDump
    }

    fn description(&self) -> &str {
        "Compresses hex dumps; keeps first 8 rows only"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        88
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 8 {
            let mut out: Vec<String> = lines[..8].iter().map(|s| s.to_string()).collect();
            out.push("... [hex dump truncated]".to_string());
            out.join("\n")
        } else {
            text.to_string()
        }
    }
}
