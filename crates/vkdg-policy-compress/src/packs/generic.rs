//! Generic content filter pack.
//!
//! Fallback for content that does not match any specific class.
//! Only removes consecutive blank lines; all other content is preserved.
//! Estimated reduction: 1-5% (minimal, safety net only).

use crate::class::ContentClass;
use crate::FilterPack;

/// Minimal filter for unclassified content.
///
/// Deduplicates consecutive blank lines. Never truncates or drops content.
pub struct GenericPack;

impl FilterPack for GenericPack {
    fn id(&self) -> &str {
        "rtk:generic"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::Generic
    }

    fn description(&self) -> &str {
        "Fallback filter; deduplicates consecutive blank lines only"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        3
    }

    fn apply(&self, text: &str) -> String {
        let mut lines: Vec<&str> = text.lines().collect();
        lines.dedup_by(|a, b| a.trim().is_empty() && b.trim().is_empty());
        lines.join("\n")
    }
}
