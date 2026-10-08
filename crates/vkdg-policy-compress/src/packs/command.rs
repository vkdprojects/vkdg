//! Command output filter pack.
//!
//! Handles generic shell/CLI command output (detected via `$` prompt + newlines).
//! Removes consecutive blank lines and keeps first 25 + last 25 lines when
//! output exceeds 50 lines.
//! Estimated reduction: 30-60% on long command outputs.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses shell command output.
///
/// Deduplicates consecutive blank lines and truncates outputs longer than
/// 50 lines to `head(25) + "... [truncated] ..." + tail(25)`.
pub struct CommandOutputPack;

impl FilterPack for CommandOutputPack {
    fn id(&self) -> &str {
        "rtk:command-output"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::CommandOutput
    }

    fn description(&self) -> &str {
        "Compresses shell command output; keeps first/last 25 lines for long outputs"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        45
    }

    fn apply(&self, text: &str) -> String {
        let mut lines: Vec<&str> = text.lines().collect();
        // Remove consecutive blank lines
        lines.dedup_by(|a, b| a.trim().is_empty() && b.trim().is_empty());
        // Truncate if > 50 lines: keep first 25 + last 25
        if lines.len() > 50 {
            let mut out: Vec<String> = lines[..25].iter().map(|s| s.to_string()).collect();
            out.push("... [truncated] ...".to_string());
            out.extend(lines[lines.len() - 25..].iter().map(|s| s.to_string()));
            out.join("\n")
        } else {
            lines.join("\n")
        }
    }
}
