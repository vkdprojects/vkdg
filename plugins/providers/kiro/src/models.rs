//! Model id mapping for Kiro.
//!
//! Kiro names models with dots (`claude-sonnet-4.5`) where client APIs use dashes
//! (`claude-sonnet-4-5`), and route patterns may carry a glob. `auto` lets the
//! service pick.
//!
//! AWS's own CLI does not hardcode a model list: it calls `ListAvailableModels`
//! and caches the result, so any table here would drift. We only normalise the
//! name and let upstream reject an unknown one.

/// Model id used when a connection names no specific model.
pub const AUTO_MODEL: &str = "auto";

/// Kiro's model id for a configured model name or route pattern.
///
/// A glob pattern (`claude-*`) names a family rather than a model, so it becomes
/// `auto`; sending the pattern verbatim would be rejected upstream.
pub fn resolve_model_id(configured: Option<&str>) -> String {
    let name = configured.unwrap_or("").trim();
    if name.is_empty() || name == AUTO_MODEL || name.contains('*') || name.contains('?') {
        return AUTO_MODEL.to_owned();
    }
    normalise_version_separator(name)
}

/// Rewrites a trailing version segment from dashes to dots:
/// `claude-sonnet-4-5` → `claude-sonnet-4.5`, leaving names without a numeric
/// tail untouched.
fn normalise_version_separator(name: &str) -> String {
    let parts: Vec<&str> = name.split('-').collect();
    // Find where the trailing run of numeric segments starts.
    let first_numeric = parts
        .iter()
        .rposition(|p| !p.chars().all(|c| c.is_ascii_digit()))
        .map_or(0, |i| i + 1);
    if first_numeric == 0 || first_numeric >= parts.len() {
        return name.to_owned();
    }
    let (head, version) = parts.split_at(first_numeric);
    format!("{}-{}", head.join("-"), version.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Refutes: sending a route glob as a model id, which upstream rejects.
    #[test]
    fn patterns_and_blanks_become_auto() {
        assert_eq!(resolve_model_id(Some("claude-*")), AUTO_MODEL);
        assert_eq!(resolve_model_id(Some("gpt-5.6-?")), AUTO_MODEL);
        assert_eq!(resolve_model_id(Some("")), AUTO_MODEL);
        assert_eq!(resolve_model_id(Some("  ")), AUTO_MODEL);
        assert_eq!(resolve_model_id(None), AUTO_MODEL);
        assert_eq!(resolve_model_id(Some("auto")), AUTO_MODEL);
    }

    // Refutes: OmniRoute's blanket `replace('-', ".")`, which mangles the whole
    // name (`claude.sonnet.4.5`) instead of just the version.
    #[test]
    fn only_the_version_tail_switches_to_dots() {
        assert_eq!(
            resolve_model_id(Some("claude-sonnet-4-5")),
            "claude-sonnet-4.5"
        );
        assert_eq!(resolve_model_id(Some("claude-opus-5")), "claude-opus-5");
        assert_eq!(resolve_model_id(Some("gpt-5-6-mini")), "gpt-5-6-mini");
        assert_eq!(
            resolve_model_id(Some("claude-sonnet-4.5")),
            "claude-sonnet-4.5"
        );
        // `v3` is not a numeric segment, so only a trailing numeric run converts.
        assert_eq!(resolve_model_id(Some("deepseek-v3-1")), "deepseek-v3-1");
    }
}
