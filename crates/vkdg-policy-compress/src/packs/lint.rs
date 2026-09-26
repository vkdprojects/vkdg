//! Lint output filter packs.
//!
//! Two packs cover common JS ecosystem linters:
//! - `EslintOutputPack` — ESLint / Prettier / Biome output.
//! - `NpmAuditPack`     — `npm audit` vulnerability reports.
//!
//! Both filter to actionable lines only, capping output at a small limit.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses ESLint (and Prettier/Biome) output.
///
/// Retains only lines that mention `"error"`, `"warning"`, or `"problem"`.
/// At most 20 lines are kept.
/// Estimated reduction: 60-80% on large lint runs.
pub struct EslintOutputPack;

impl FilterPack for EslintOutputPack {
    fn id(&self) -> &str {
        "rtk:eslint-output"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::EslintOutput
    }

    fn description(&self) -> &str {
        "Compresses ESLint/Prettier/Biome output; keeps first 20 error/warning lines"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        70
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        lines
            .iter()
            .filter(|l| l.contains("error") || l.contains("warning") || l.contains("problem"))
            .take(20)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Compresses `npm audit` output.
///
/// Retains only lines containing severity keywords or the fix command.
/// At most 15 lines are kept.
/// Estimated reduction: 60-75% on verbose audit reports.
pub struct NpmAuditPack;

impl FilterPack for NpmAuditPack {
    fn id(&self) -> &str {
        "rtk:npm-audit"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::NpmAudit
    }

    fn description(&self) -> &str {
        "Compresses npm audit output; keeps severity/vulnerability summary lines"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        65
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        lines
            .iter()
            .filter(|l| {
                l.contains("critical")
                    || l.contains("high")
                    || l.contains("moderate")
                    || l.contains("vulnerabilities")
                    || l.contains("npm audit fix")
            })
            .take(15)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
