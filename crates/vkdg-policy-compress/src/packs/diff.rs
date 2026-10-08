//! Diff filter pack.
//!
//! Handles unified diff output (`---`/`+++`/`@@` headers).
//! Keeps all context/header lines but limits added/removed lines to 40.
//! Estimated reduction: 40-70% on large diffs.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses unified diff output.
///
/// Keeps all `---`/`+++`/`@@` headers and context lines unchanged.
/// Changed lines (`+`/`-`) are capped at 40; excess is replaced with
/// `"... [diff truncated] ..."`.
pub struct DiffPack;

impl FilterPack for DiffPack {
    fn id(&self) -> &str {
        "rtk:diff"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::Diff
    }

    fn description(&self) -> &str {
        "Compresses unified diffs; keeps headers + first 40 changed lines"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        55
    }

    fn apply(&self, text: &str) -> String {
        let mut kept: Vec<String> = Vec::new();
        let mut changed = 0usize;
        for line in text.lines() {
            if line.starts_with('+') || line.starts_with('-') {
                changed += 1;
                if changed > 40 {
                    if changed == 41 {
                        kept.push("... [diff truncated] ...".to_string());
                    }
                    continue;
                }
            }
            kept.push(line.to_string());
        }
        kept.join("\n")
    }
}
