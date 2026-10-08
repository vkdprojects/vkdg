//! Test output filter pack.
//!
//! Handles `cargo test`, Jest, pytest, and similar test runner output.
//! Filters to summary/failure lines only, discarding per-test `ok` noise.
//! Estimated reduction: 70-90% on large test suites.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses test runner output.
///
/// Keeps lines that contain pass/fail/error summary keywords and the final
/// result line. At most 30 lines are retained.
pub struct TestOutputPack;

impl FilterPack for TestOutputPack {
    fn id(&self) -> &str {
        "rtk:test-output"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::TestOutput
    }

    fn description(&self) -> &str {
        "Compresses test output; keeps PASS/FAIL/error/summary lines (max 30)"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        80
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        lines
            .iter()
            .filter(|l| {
                l.contains("PASS")
                    || l.contains("FAIL")
                    || l.contains("passed")
                    || l.contains("failed")
                    || l.contains("error")
                    || l.contains("FAILED")
                    || l.contains("test result:")
                    || l.contains("Finished")
                    || l.contains("=====")
            })
            .take(30)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
