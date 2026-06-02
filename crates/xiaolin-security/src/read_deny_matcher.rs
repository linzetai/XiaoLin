use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};

use crate::permission_profile::FileSystemSandboxPolicy;

/// Runtime matcher for read-deny entries in a filesystem sandbox policy.
///
/// Resolves deny entries against a cwd at construction time and checks
/// paths against both exact subtree deny roots and compiled glob matchers.
/// When a glob pattern is invalid, the matcher operates in "fail-closed"
/// mode — all reads are denied to prevent policy bypass from a config typo.
pub struct ReadDenyMatcher {
    denied_candidates: Vec<Vec<PathBuf>>,
    deny_glob_matchers: Vec<GlobMatcher>,
    invalid_pattern: bool,
}

#[derive(Clone, Copy)]
enum InvalidDenyReadGlobBehavior {
    FailClosed,
    ReturnError,
}

impl ReadDenyMatcher {
    /// Build a matcher from a `FileSystemSandboxPolicy` resolved against `cwd`.
    ///
    /// Returns `None` when there are no deny restrictions. Uses fail-closed
    /// semantics for invalid glob patterns.
    pub fn new(policy: &FileSystemSandboxPolicy, cwd: &Path) -> Option<Self> {
        match Self::build(policy, cwd, InvalidDenyReadGlobBehavior::FailClosed) {
            Ok(matcher) => matcher,
            Err(_) => unreachable!("fail-closed glob handling does not return errors"),
        }
    }

    /// Build a matcher that returns an error on invalid glob patterns.
    ///
    /// Use this for host-side expansion where a typo should be surfaced
    /// rather than silently broadening the deny set.
    pub fn try_new(
        policy: &FileSystemSandboxPolicy,
        cwd: &Path,
    ) -> Result<Option<Self>, String> {
        Self::build(policy, cwd, InvalidDenyReadGlobBehavior::ReturnError)
    }

    /// Build a matcher from raw path and glob data without a policy.
    ///
    /// This is a convenience constructor for callers that already have
    /// resolved paths.
    pub fn with_denied_paths(
        denied_paths: Vec<PathBuf>,
        deny_globs: &[String],
    ) -> Self {
        let denied_candidates: Vec<Vec<PathBuf>> = denied_paths
            .into_iter()
            .map(|p| normalized_and_canonical_candidates(&p))
            .collect();

        let mut deny_glob_matchers = Vec::with_capacity(deny_globs.len());
        let mut invalid_pattern = false;

        for pattern_str in deny_globs {
            match build_glob_matcher(pattern_str) {
                Ok(matcher) => deny_glob_matchers.push(matcher),
                Err(_) => {
                    tracing::warn!(
                        pattern = %pattern_str,
                        "invalid deny-read glob pattern; failing closed"
                    );
                    invalid_pattern = true;
                }
            }
        }

        Self {
            denied_candidates,
            deny_glob_matchers,
            invalid_pattern,
        }
    }

    fn build(
        policy: &FileSystemSandboxPolicy,
        cwd: &Path,
        invalid_glob_behavior: InvalidDenyReadGlobBehavior,
    ) -> Result<Option<Self>, String> {
        if !policy.has_denied_read_restrictions() {
            return Ok(None);
        }

        let denied_candidates: Vec<Vec<PathBuf>> = policy
            .get_unreadable_roots_with_cwd(cwd)
            .into_iter()
            .map(|path| normalized_and_canonical_candidates(path.as_path()))
            .collect();

        let mut invalid_pattern = false;
        let mut deny_glob_matchers = Vec::new();
        for pattern in policy.get_unreadable_globs_with_cwd(cwd) {
            match build_glob_matcher(&pattern) {
                Ok(matcher) => deny_glob_matchers.push(matcher),
                Err(err) => match invalid_glob_behavior {
                    InvalidDenyReadGlobBehavior::FailClosed => {
                        tracing::warn!(
                            pattern = %pattern,
                            "invalid deny-read glob pattern; failing closed"
                        );
                        invalid_pattern = true;
                    }
                    InvalidDenyReadGlobBehavior::ReturnError => {
                        return Err(format!(
                            "invalid deny-read glob pattern `{pattern}`: {err}"
                        ));
                    }
                },
            }
        }

        Ok(Some(Self {
            denied_candidates,
            deny_glob_matchers,
            invalid_pattern,
        }))
    }

    /// Returns `true` if `path` is denied by this matcher's policy.
    ///
    /// When any glob pattern was invalid, this always returns `true`
    /// (fail-closed) to prevent a config typo from silently allowing reads.
    pub fn is_read_denied(&self, path: &Path) -> bool {
        if self.invalid_pattern {
            return true;
        }

        let path_candidates = normalized_and_canonical_candidates(path);

        if self.denied_candidates.iter().any(|denied_candidates| {
            path_candidates.iter().any(|candidate| {
                denied_candidates
                    .iter()
                    .any(|denied_candidate| {
                        candidate == denied_candidate
                            || candidate.starts_with(denied_candidate)
                    })
            })
        }) {
            return true;
        }

        self.deny_glob_matchers.iter().any(|matcher| {
            path_candidates
                .iter()
                .any(|candidate| matcher.is_match(candidate))
        })
    }
}

fn build_glob_matcher(pattern: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|err| err.to_string())
}

/// Generate all meaningful path spellings for a path: the normalized form
/// plus the canonical (symlink-resolved) form when it exists. This lets
/// deny checks catch both a symlink path and its canonical target.
fn normalized_and_canonical_candidates(path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(normalized) = xiaolin_core::path::AbsolutePathBuf::from_absolute_path(path) {
        push_unique(&mut candidates, normalized.to_path_buf());
    } else {
        push_unique(&mut candidates, path.to_path_buf());
    }

    if let Ok(canonical) = path.canonicalize() {
        if let Ok(canonical_absolute) =
            xiaolin_core::path::AbsolutePathBuf::from_absolute_path(&canonical)
        {
            push_unique(&mut candidates, canonical_absolute.to_path_buf());
        }
    }

    candidates
}

fn push_unique(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission_profile::{
        FileSystemAccessMode, FileSystemPath, FileSystemSandboxEntry,
    };
    use std::path::PathBuf;

    fn make_deny_glob_policy(globs: Vec<&str>) -> FileSystemSandboxPolicy {
        let entries: Vec<FileSystemSandboxEntry> = globs
            .into_iter()
            .map(|g| FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: g.to_string(),
                },
                access: FileSystemAccessMode::None,
            })
            .collect();
        FileSystemSandboxPolicy::restricted(entries)
    }

    fn test_cwd() -> PathBuf {
        PathBuf::from("/home/user/project")
    }

    #[test]
    fn returns_none_for_unrestricted() {
        let policy = FileSystemSandboxPolicy::unrestricted();
        assert!(ReadDenyMatcher::new(&policy, &test_cwd()).is_none());
    }

    #[test]
    fn returns_none_for_no_deny_entries() {
        let policy = FileSystemSandboxPolicy::restricted(vec![]);
        assert!(ReadDenyMatcher::new(&policy, &test_cwd()).is_none());
    }

    #[test]
    fn glob_pattern_matches() {
        let policy = make_deny_glob_policy(vec!["**/.env"]);
        let matcher = ReadDenyMatcher::new(&policy, &test_cwd()).unwrap();
        assert!(matcher.is_read_denied(Path::new("/home/user/project/.env")));
    }

    #[test]
    fn glob_pattern_does_not_match_unrelated() {
        let policy = make_deny_glob_policy(vec!["**/.env"]);
        let matcher = ReadDenyMatcher::new(&policy, &test_cwd()).unwrap();
        assert!(!matcher.is_read_denied(Path::new("/home/user/project/readme.md")));
    }

    #[test]
    fn denied_path_subtree_matches() {
        let matcher = ReadDenyMatcher::with_denied_paths(
            vec![PathBuf::from("/secret")],
            &[],
        );
        assert!(matcher.is_read_denied(Path::new("/secret/file.txt")));
        assert!(matcher.is_read_denied(Path::new("/secret")));
        assert!(!matcher.is_read_denied(Path::new("/other/file.txt")));
    }

    #[test]
    fn invalid_glob_fails_closed() {
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "[invalid".to_string(),
            },
            access: FileSystemAccessMode::None,
        }]);
        let matcher = ReadDenyMatcher::new(&policy, &test_cwd()).unwrap();
        assert!(matcher.is_read_denied(Path::new("/any/path")));
    }

    #[test]
    fn multiple_globs_any_match_denies() {
        let policy = make_deny_glob_policy(vec!["**/.env", "**/*.key"]);
        let cwd = test_cwd();
        let matcher = ReadDenyMatcher::new(&policy, &cwd).unwrap();
        assert!(matcher.is_read_denied(Path::new("/home/user/project/sub/.env")));
        assert!(matcher.is_read_denied(Path::new("/home/user/project/ssl/cert.key")));
        assert!(!matcher.is_read_denied(Path::new("/home/user/project/readme.md")));
    }

    #[test]
    fn with_denied_paths_and_globs() {
        let matcher = ReadDenyMatcher::with_denied_paths(
            vec![PathBuf::from("/secret")],
            &["**/.ssh/**".to_string()],
        );
        assert!(matcher.is_read_denied(Path::new("/secret/data")));
        assert!(matcher.is_read_denied(Path::new("/home/user/.ssh/id_rsa")));
        assert!(!matcher.is_read_denied(Path::new("/home/user/readme.md")));
    }

    #[test]
    fn deny_path_entry_matched() {
        use xiaolin_core::path::test_support::{test_path_buf, PathBufExt};
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/secret").abs(),
            },
            access: FileSystemAccessMode::None,
        }]);
        let matcher = ReadDenyMatcher::new(&policy, &test_cwd()).unwrap();
        assert!(matcher.is_read_denied(Path::new("/secret/file.txt")));
        assert!(!matcher.is_read_denied(Path::new("/other/file.txt")));
    }

    #[test]
    fn try_new_returns_error_on_bad_glob() {
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "[invalid".to_string(),
            },
            access: FileSystemAccessMode::None,
        }]);
        let result = ReadDenyMatcher::try_new(&policy, &test_cwd());
        assert!(result.is_err());
    }
}
