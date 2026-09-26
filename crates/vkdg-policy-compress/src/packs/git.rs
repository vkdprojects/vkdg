//! Git output filter packs.
//!
//! Two packs cover the two most common git output classes:
//! - `GitStatusPack` — `git status` verbose output.
//! - `GitLogPack`    — `git log` one-line or oneline-ish output.
//!
//! Both preserve the information an agent needs (branch, recent commits)
//! while discarding repetitive detail.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses `git status` output.
///
/// Keeps the branch header and all non-file-change lines verbatim.
/// File-change lines (`modified:`, `new file:`, `deleted:`) are kept up to
/// 10; additional files are replaced with a summary count.
/// Estimated reduction: 40-60% on repos with many changed files.
pub struct GitStatusPack;

impl FilterPack for GitStatusPack {
    fn id(&self) -> &str {
        "rtk:git-status"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::GitStatus
    }

    fn description(&self) -> &str {
        "Compresses git status; keeps branch + first 10 changed files"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        50
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let mut kept: Vec<String> = Vec::new();
        let mut file_count = 0usize;
        for line in &lines {
            let trimmed = line.trim();
            if trimmed.starts_with("modified:")
                || trimmed.starts_with("new file:")
                || trimmed.starts_with("deleted:")
            {
                file_count += 1;
                if file_count <= 10 {
                    kept.push(line.to_string());
                }
            } else {
                kept.push(line.to_string());
            }
        }
        if file_count > 10 {
            kept.push(format!("  ... ({} more files)", file_count - 10));
        }
        kept.join("\n")
    }
}

/// Compresses `git log` output.
///
/// Keeps the first 20 commit lines; appends a count of remaining commits.
/// Estimated reduction: 50-80% on repos with long histories.
pub struct GitLogPack;

impl FilterPack for GitLogPack {
    fn id(&self) -> &str {
        "rtk:git-log"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::GitLog
    }

    fn description(&self) -> &str {
        "Compresses git log; keeps first 20 commits and summarises the rest"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        65
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 20 {
            let total = lines.len();
            let mut out: Vec<String> = lines[..20].iter().map(|s| s.to_string()).collect();
            out.push(format!("... ({} more commits)", total - 20));
            out.join("\n")
        } else {
            text.to_string()
        }
    }
}
