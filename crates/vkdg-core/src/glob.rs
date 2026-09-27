//! Model-name patterns: `*` matches any run of characters, `?` exactly one.
//!
//! One matcher for connections, routes, combos, and per-key model lists, so a
//! pattern means the same thing everywhere it appears in config.

/// True when `name` matches `pattern`.
///
/// Iterative with single-star backtracking: linear in practice and never
/// exponential, whatever the pattern (a recursive matcher is, on `*a*a*a*b`).
pub fn matches(pattern: &str, name: &str) -> bool {
    let (p, s) = (pattern.as_bytes(), name.as_bytes());
    let (mut pi, mut si) = (0, 0);
    // Position of the last `*` seen and the input index it currently absorbs up to.
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match p.get(pi) {
            Some(b'*') => {
                star = Some((pi, si));
                pi += 1;
            }
            Some(&c) if c == b'?' || c == s[si] => {
                pi += 1;
                si += 1;
            }
            _ => match star {
                // Let the last `*` absorb one more character and retry.
                Some((sp, ss)) => {
                    pi = sp + 1;
                    si = ss + 1;
                    star = Some((sp, ss + 1));
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// True when `name` matches any of `patterns`.
pub fn matches_any<S: AsRef<str>>(patterns: &[S], name: &str) -> bool {
    patterns.iter().any(|p| matches(p.as_ref(), name))
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn literal_prefix_suffix_and_infix_stars() {
        assert!(matches("claude-3-5-haiku", "claude-3-5-haiku"));
        assert!(!matches("claude-3-5-haiku", "claude-3-5-haiku-2024"));
        assert!(matches("claude-*", "claude-sonnet-4.5"));
        assert!(matches("claude-*", "claude-"));
        assert!(!matches("claude-*", "gpt-4o"));
        assert!(matches("*-mini", "gpt-4o-mini"));
        assert!(matches("gpt-*-mini", "gpt-4o-mini"));
        assert!(!matches("gpt-*-mini", "gpt-4o"));
        assert!(matches("*", ""));
        assert!(!matches("", "x"));
    }

    #[test]
    fn question_mark_is_exactly_one_character() {
        assert!(matches("o?-mini", "o3-mini"));
        assert!(!matches("o?-mini", "o-mini"));
        assert!(!matches("o?-mini", "o33-mini"));
    }

    // A recursive matcher takes exponential time here; this must return at once.
    #[test]
    fn pathological_pattern_is_fast() {
        let name = "a".repeat(64);
        assert!(!matches("*a*a*a*a*a*a*a*a*b", &name));
    }
}
