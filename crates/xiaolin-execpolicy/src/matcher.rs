use crate::config::PatternElement;

/// Check if a command token array matches a prefix pattern.
///
/// The pattern is a list of `PatternElement`s. Each element either requires
/// an exact match or allows any of several alternatives. The command tokens
/// must start with a matching prefix (extra trailing tokens are allowed).
pub fn matches_prefix(pattern: &[PatternElement], tokens: &[&str]) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if tokens.len() < pattern.len() {
        return false;
    }

    for (i, element) in pattern.iter().enumerate() {
        match element {
            PatternElement::Exact(expected) => {
                if tokens[i] != expected {
                    return false;
                }
            }
            PatternElement::Alternatives(alternatives) => {
                if !alternatives.iter().any(|alt| tokens[i] == alt) {
                    return false;
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(s: &str) -> PatternElement {
        PatternElement::Exact(s.to_string())
    }

    fn alts(values: &[&str]) -> PatternElement {
        PatternElement::Alternatives(values.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn empty_pattern_matches_everything() {
        assert!(matches_prefix(&[], &["ls", "-la"]));
        assert!(matches_prefix(&[], &[]));
    }

    #[test]
    fn exact_single_token() {
        let pat = vec![exact("ls")];
        assert!(matches_prefix(&pat, &["ls"]));
        assert!(matches_prefix(&pat, &["ls", "-la"]));
        assert!(!matches_prefix(&pat, &["cat"]));
    }

    #[test]
    fn exact_multi_token() {
        let pat = vec![exact("git"), exact("push")];
        assert!(matches_prefix(&pat, &["git", "push"]));
        assert!(matches_prefix(&pat, &["git", "push", "origin"]));
        assert!(!matches_prefix(&pat, &["git", "pull"]));
        assert!(!matches_prefix(&pat, &["git"]));
    }

    #[test]
    fn alternatives_match() {
        let pat = vec![exact("git"), alts(&["merge", "rebase"])];
        assert!(matches_prefix(&pat, &["git", "merge"]));
        assert!(matches_prefix(&pat, &["git", "rebase"]));
        assert!(matches_prefix(&pat, &["git", "merge", "main"]));
        assert!(!matches_prefix(&pat, &["git", "push"]));
        assert!(!matches_prefix(&pat, &["git", "pull"]));
    }

    #[test]
    fn mixed_exact_and_alternatives() {
        let pat = vec![exact("npm"), alts(&["install", "i"]), exact("--save-dev")];
        assert!(matches_prefix(
            &pat,
            &["npm", "install", "--save-dev", "pkg"]
        ));
        assert!(matches_prefix(&pat, &["npm", "i", "--save-dev"]));
        assert!(!matches_prefix(&pat, &["npm", "install", "pkg"]));
        assert!(!matches_prefix(&pat, &["npm", "run", "--save-dev"]));
    }

    #[test]
    fn too_few_tokens() {
        let pat = vec![exact("git"), exact("push"), exact("--force")];
        assert!(!matches_prefix(&pat, &["git", "push"]));
        assert!(!matches_prefix(&pat, &["git"]));
    }

    #[test]
    fn trailing_tokens_allowed() {
        let pat = vec![exact("git"), exact("push")];
        assert!(matches_prefix(
            &pat,
            &["git", "push", "--force", "origin", "main"]
        ));
    }
}
