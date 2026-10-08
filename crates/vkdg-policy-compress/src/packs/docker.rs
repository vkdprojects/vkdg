//! Docker log filter pack.
//!
//! Handles `docker compose logs` / `docker logs` output, which prefixes each
//! line with `<service> |`. Limits each service to at most 5 log lines to
//! prevent one noisy container from dominating the output.
//! Estimated reduction: 60-85% on busy multi-service stacks.

use crate::class::ContentClass;
use crate::FilterPack;

/// Compresses Docker (Compose) log output.
///
/// Groups lines by service prefix (the part before `|`). Keeps at most 5
/// lines per service; non-prefixed lines are always kept.
pub struct DockerLogPack;

impl FilterPack for DockerLogPack {
    fn id(&self) -> &str {
        "rtk:docker-log"
    }

    fn handles(&self) -> ContentClass {
        ContentClass::DockerLog
    }

    fn description(&self) -> &str {
        "Compresses docker compose logs; keeps first 5 lines per service"
    }

    fn estimated_reduction_pct(&self) -> u8 {
        70
    }

    fn apply(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let mut service_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut kept: Vec<String> = Vec::new();
        for line in &lines {
            if let Some(service) = line.split('|').next() {
                let service = service.trim().to_string();
                let count = service_counts.entry(service).or_insert(0);
                if *count < 5 {
                    kept.push(line.to_string());
                    *count += 1;
                }
            } else {
                kept.push(line.to_string());
            }
        }
        kept.join("\n")
    }
}
