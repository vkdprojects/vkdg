use crate::plan::Combo;

/// Resolves a model name or request metadata to a Combo.
///
/// Resolution order (matches OmniRoute's behavior):
/// 1. Exact combo ID match ("coding-fast" literal)
/// 2. Combo match_patterns glob ("code:*", "combo/*")
/// 3. Falls through to bare model routing
pub struct ComboResolver {
    combos: Vec<Combo>,
}

impl ComboResolver {
    pub fn new(combos: Vec<Combo>) -> Self {
        Self { combos }
    }

    /// Resolve a model name or combo name to a Combo, if any matches.
    pub fn resolve(&self, name: &str) -> Option<&Combo> {
        // 1. Exact combo ID
        if let Some(c) = self.combos.iter().find(|c| c.id == name) {
            return Some(c);
        }
        // 2. Pattern match
        self.combos
            .iter()
            .find(|c| c.match_patterns.iter().any(|p| glob_match(p, name)))
    }

    pub fn all(&self) -> &[Combo] {
        &self.combos
    }
}

/// Simple glob: '*' matches any sequence, '?' matches one char.
fn glob_match(pattern: &str, input: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = input.chars().collect();
    glob_inner(&p, &s)
}

fn glob_inner(p: &[char], s: &[char]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (None, _) => false,
        (Some(&'*'), _) => {
            // '*' matches zero characters (skip it) or one character in s
            glob_inner(&p[1..], s) || (!s.is_empty() && glob_inner(p, &s[1..]))
        }
        (Some(&'?'), Some(_)) => glob_inner(&p[1..], &s[1..]),
        (Some(pc), Some(sc)) if pc == sc => glob_inner(&p[1..], &s[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Combo;
    use vkdg_routing::StrategyKind;

    fn make_combo(id: &str, patterns: &[&str]) -> Combo {
        Combo {
            id: id.into(),
            match_patterns: patterns.iter().map(|s| s.to_string()).collect(),
            strategy: StrategyKind::FallbackChain,
            targets: vec![],
            compression: None,
            cache: None,
            budget: None,
            mode_pack: None,
        }
    }

    // Plausible wrong impl: resolver returns first combo regardless of name
    #[test]
    fn exact_id_match() {
        let r = ComboResolver::new(vec![
            make_combo("coding-fast", &[]),
            make_combo("quality-first", &[]),
        ]);
        assert_eq!(
            r.resolve("coding-fast").map(|c| c.id.as_str()),
            Some("coding-fast")
        );
        assert_eq!(
            r.resolve("quality-first").map(|c| c.id.as_str()),
            Some("quality-first")
        );
    }

    // Plausible wrong impl: glob patterns ignored, only exact matches work
    #[test]
    fn glob_pattern_match() {
        let r = ComboResolver::new(vec![make_combo("code-combo", &["code:*"])]);
        assert!(r.resolve("code:python").is_some());
        assert!(r.resolve("code:rust").is_some());
        assert!(r.resolve("vision:photo").is_none());
    }

    // Plausible wrong impl: bare model name incorrectly resolves to a combo
    #[test]
    fn no_match_returns_none() {
        let r = ComboResolver::new(vec![make_combo("coding-fast", &["code:*"])]);
        assert!(r.resolve("claude-3-5-haiku-20241022").is_none());
    }

    // Plausible wrong impl: exact ID checked after patterns, shadowed by glob
    #[test]
    fn exact_id_wins_over_glob() {
        let r = ComboResolver::new(vec![
            make_combo("code:python", &[]),       // exact id
            make_combo("catch-all", &["code:*"]), // glob
        ]);
        assert_eq!(
            r.resolve("code:python").map(|c| c.id.as_str()),
            Some("code:python")
        );
    }

    // Plausible wrong impl: '*' at end of pattern doesn't match empty suffix
    #[test]
    fn glob_star_matches_empty_suffix() {
        let r = ComboResolver::new(vec![make_combo("combo", &["prefix*"])]);
        assert!(r.resolve("prefix").is_some());
        assert!(r.resolve("prefix-extended").is_some());
    }
}
