//! Content class detection for RTK filter dispatch.
//!
//! `detect_class` inspects the first few hundred bytes of a text blob and
//! returns the most specific `ContentClass` it can recognise. Runs in O(n)
//! over line count; never allocates beyond the line iterator.

/// Classifies a text blob so the correct FilterPack can be applied.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentClass {
    CommandOutput,
    StackTrace,
    JsonOutput,
    FileList,
    Diff,
    Generic,
    GitStatus,
    GitLog,
    TypeScriptBuild,
    EslintOutput,
    NpmAudit,
    DockerLog,
    TestOutput,
    HexDump,
}

/// Detect the content class of `text`.
///
/// Heuristics are ordered from most-specific to least-specific to avoid
/// false positives. Returns `ContentClass::Generic` when no pattern matches.
pub fn detect_class(text: &str) -> ContentClass {
    if text.contains("\n\tat ")
        || text.contains("Traceback (most recent call")
        || text.contains("Error: at ")
    {
        ContentClass::StackTrace
    } else if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
        ContentClass::JsonOutput
    } else if text
        .lines()
        .any(|l| l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("@@"))
    {
        ContentClass::Diff
    } else if text.lines().all(|l| {
        l.trim().is_empty() || l.starts_with('/') || l.starts_with('.') || l.ends_with('/')
    }) {
        ContentClass::FileList
    } else if text.contains("On branch")
        || text.contains("Changes not staged")
        || text.contains("Untracked files:")
        || (text.contains("modified:") && text.contains("git"))
    {
        ContentClass::GitStatus
    } else if text
        .lines()
        .take(5)
        .filter(|l| !l.trim().is_empty())
        .all(|l| l.len() > 7 && l.chars().take(7).all(|c| c.is_ascii_hexdigit()))
    {
        ContentClass::GitLog
    } else if text.contains("error TS")
        || (text.contains(".ts(") && text.contains("error:"))
        || (text.contains("Cannot find module") && text.contains(".ts"))
    {
        ContentClass::TypeScriptBuild
    } else if (text.contains('✖') && (text.contains("error") || text.contains("warning")))
        || (text.contains("problems (") && text.contains("error"))
        || text.contains("Prettier:")
        || text.contains("biome check")
    {
        ContentClass::EslintOutput
    } else if text.contains("npm audit")
        || (text.contains("vulnerabilities") && text.contains("npm"))
    {
        ContentClass::NpmAudit
    } else if (text.contains("test result:")
        && (text.contains("passed") || text.contains("failed")))
        || (text.contains("PASS") && text.contains("FAIL") && text.contains("Tests:"))
        || (text.contains("passed") && text.contains("failed") && text.contains("====="))
    {
        ContentClass::TestOutput
    } else if text.lines().take(3).all(|l| {
        l.contains('|') && l.len() > 20 && l.chars().filter(|c| c.is_ascii_hexdigit()).count() > 10
    }) && !text.trim().is_empty()
    {
        ContentClass::HexDump
    } else if text.lines().any(|l| {
        l.contains('|') && l.len() > 25 && l.chars().next().is_some_and(|c| c.is_alphabetic())
    }) {
        ContentClass::DockerLog
    } else if text.contains('$') && text.contains('\n') {
        ContentClass::CommandOutput
    } else {
        ContentClass::Generic
    }
}
