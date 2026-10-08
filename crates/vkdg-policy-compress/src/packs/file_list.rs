//! File list filter pack.
//!
//! Handles outputs that are purely file/directory paths (one per line).
//! Keeps the first 30 entries and appends a count of elided entries.
//! Estimated reduction: 50-80% on deep directory listings.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses file/directory listing output.
///
/// Shows at most 30 paths; appends `"... [N more entries]..."` for the rest.
pub struct FileListPack;

impl FilterPack for FileListPack {
    fn id(&self) -> &str {
        "rtk:file-list"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::FileList
    }

    fn description(&self) -> &str {
        "Compresses file listings; keeps first 30 paths and summarises the rest"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        65
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 30 {
            let total = lines.len();
            let mut out: Vec<String> = lines[..30].iter().map(|s| s.to_string()).collect();
            out.push(format!("... [{} more entries]...", total - 30));
            out.join("\n")
        } else {
            lines.join("\n")
        }
    }
}
