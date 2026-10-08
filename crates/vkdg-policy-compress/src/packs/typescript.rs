//! TypeScript build output filter pack.
//!
//! Handles `tsc` compiler output: filters to error lines only and caps at 20.
//! Estimated reduction: 70-85% on builds with many info/verbose lines.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses TypeScript compiler (`tsc`) output.
///
/// Retains only lines containing `"error TS"` or `"error:"`.
/// At most 20 error lines are kept; a count of suppressed errors is appended.
pub struct TypeScriptBuildPack;

impl FilterPack for TypeScriptBuildPack {
    fn id(&self) -> &str {
        "rtk:typescript-build"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::TypeScriptBuild
    }

    fn description(&self) -> &str {
        "Compresses tsc output; keeps first 20 error lines"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        75
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let total_errors = lines.iter().filter(|l| l.contains("error TS")).count();
        let mut out: Vec<String> = lines
            .iter()
            .filter(|l| l.contains("error TS") || l.contains("error:"))
            .take(20)
            .map(|s| s.to_string())
            .collect();
        if total_errors > 20 {
            out.push(format!("... {} more errors", total_errors - 20));
        }
        out.join("\n")
    }
}
