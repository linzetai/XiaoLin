use std::path::{Path, PathBuf};

/// Result of path validation for a shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathVerdict {
    Safe,
    Blocked { path: String, reason: String },
}

impl PathVerdict {
    pub fn is_safe(&self) -> bool {
        matches!(self, PathVerdict::Safe)
    }
}

/// Sensitive paths under $HOME that should never be written to.
const PROTECTED_HOME_PATHS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".gpg",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".config/git/credentials",
    ".gitconfig",
    ".npmrc",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".aws/credentials",
    ".aws/config",
    ".kube/config",
    ".docker/config.json",
    ".netrc",
    ".env",
    ".fastclaw",
    ".local/share/keyrings",
];

/// System paths that should never be written to by agents.
const PROTECTED_SYSTEM_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/hosts",
    "/etc/crontab",
    "/etc/ssh",
    "/boot",
    "/proc",
    "/sys",
    "/dev",
];

/// Commands that modify filesystem (path validation applies to these).
const WRITE_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "touch", "mkdir", "mktemp",
    "chmod", "chown", "chgrp",
    "ln", "unlink", "shred",
    "tee", "dd", "install", "patch",
];

/// Path validator for shell commands.
///
/// Validates that write-target paths in commands:
/// 1. Don't contain traversal patterns (../)
/// 2. Don't target protected/sensitive paths
/// 3. Stay within allowed root directories (if configured)
/// 4. Symlinks are resolved before validation
pub struct PathValidator {
    allowed_roots: Vec<PathBuf>,
    home_dir: PathBuf,
}

impl PathValidator {
    /// Create a validator with allowed root directories.
    /// If `allowed_roots` is empty, only protected-path checks apply.
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/root"));
        Self { allowed_roots, home_dir }
    }

    /// Create a validator with a custom home dir (for testing).
    #[cfg(test)]
    fn with_home(allowed_roots: Vec<PathBuf>, home_dir: PathBuf) -> Self {
        Self { allowed_roots, home_dir }
    }

    /// Validate all write-target paths in a command.
    pub fn validate(&self, command: &str) -> PathVerdict {
        for segment in command
            .split("&&")
            .flat_map(|s| s.split("||"))
            .flat_map(|s| s.split(';'))
            .flat_map(|s| s.split('|'))
        {
            let seg = segment.trim();
            if seg.is_empty() {
                continue;
            }

            let (base_cmd, paths) = extract_paths(seg);

            if !is_write_command(&base_cmd, seg) {
                continue;
            }

            // Also check output redirection targets
            let redirect_targets = extract_redirect_targets(seg);

            for raw_path in paths.iter().chain(redirect_targets.iter()) {
                let cleaned = raw_path.trim_matches(|c: char| c == '\'' || c == '"');

                if let PathVerdict::Blocked { .. } = self.validate_single_path(cleaned) {
                    return self.validate_single_path(cleaned);
                }
            }
        }

        PathVerdict::Safe
    }

    /// Validate a single path.
    fn validate_single_path(&self, raw_path: &str) -> PathVerdict {
        // 1. Traversal detection (before resolving)
        if has_traversal(raw_path) {
            return PathVerdict::Blocked {
                path: raw_path.to_string(),
                reason: "path contains directory traversal pattern (..)".into(),
            };
        }

        // 2. Expand and resolve the path
        let resolved = self.resolve_path(raw_path);

        // 3. Try to follow symlinks for the real target
        let real_path = resolve_symlinks(&resolved);

        // 4. Check protected paths
        if let Some(reason) = self.is_protected(&real_path) {
            return PathVerdict::Blocked {
                path: raw_path.to_string(),
                reason,
            };
        }

        // 5. Check allowed roots (if configured)
        if !self.allowed_roots.is_empty() && !self.is_within_allowed(&real_path) {
            return PathVerdict::Blocked {
                path: raw_path.to_string(),
                reason: format!(
                    "path resolves to '{}' which is outside allowed directories",
                    real_path.display()
                ),
            };
        }

        PathVerdict::Safe
    }

    /// Resolve a raw path string to an absolute PathBuf.
    fn resolve_path(&self, raw: &str) -> PathBuf {
        if raw.starts_with("~/") || raw == "~" {
            self.home_dir.join(raw.trim_start_matches("~/").trim_start_matches('~'))
        } else if raw.starts_with('/') {
            PathBuf::from(raw)
        } else {
            // Relative path — use first allowed root as base or cwd
            let base = self.allowed_roots.first()
                .cloned()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            base.join(raw)
        }
    }

    /// Check if a path targets a protected location.
    fn is_protected(&self, path: &Path) -> Option<String> {
        // Check home-relative protected paths
        for sensitive in PROTECTED_HOME_PATHS {
            let sensitive_full = self.home_dir.join(sensitive);
            if path == sensitive_full || path.starts_with(&sensitive_full) {
                return Some(format!(
                    "targets protected path ~/{sensitive}"
                ));
            }
        }

        // Check system protected paths
        for system_path in PROTECTED_SYSTEM_PATHS {
            let sys = Path::new(system_path);
            if path == sys || path.starts_with(sys) {
                return Some(format!(
                    "targets protected system path {system_path}"
                ));
            }
        }

        None
    }

    /// Check if a path is within allowed root directories.
    fn is_within_allowed(&self, path: &Path) -> bool {
        self.allowed_roots.iter().any(|root| {
            let root_canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            let path_canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            path_canonical.starts_with(&root_canonical)
        })
    }
}

// ─── Path extraction ────────────────────────────────────────────────────────

/// Extract the base command and file path arguments from a command segment.
fn extract_paths(segment: &str) -> (String, Vec<String>) {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    if tokens.is_empty() {
        return (String::new(), Vec::new());
    }

    let base_cmd = tokens[0].rsplit('/').next().unwrap_or(tokens[0]).to_string();
    let args = &tokens[1..];

    let mut paths = Vec::new();
    let mut after_double_dash = false;
    let mut skip_next = false;

    for (i, &arg) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--" {
            after_double_dash = true;
            continue;
        }
        if after_double_dash {
            paths.push(arg.to_string());
            continue;
        }
        if arg.starts_with('-') {
            // Flags that take a path value as next argument
            if matches!(arg, "-o" | "-t" | "--target-directory" | "--output" | "-d" | "--directory") {
                if let Some(&next) = args.get(i + 1) {
                    paths.push(next.to_string());
                    skip_next = true;
                }
            }
            continue;
        }
        paths.push(arg.to_string());
    }

    (base_cmd, paths)
}

/// Extract redirect target paths (> file, >> file).
fn extract_redirect_targets(segment: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let tokens: Vec<&str> = segment.split_whitespace().collect();

    for (i, &token) in tokens.iter().enumerate() {
        if (token == ">" || token == ">>") && i + 1 < tokens.len() {
            targets.push(tokens[i + 1].to_string());
        } else if token.starts_with(">>") && token.len() > 2 {
            targets.push(token[2..].to_string());
        } else if token.starts_with('>') && !token.starts_with(">(") && token.len() > 1 && token != ">" {
            // Handle >file (no space)
            let path = token.trim_start_matches('>');
            if !path.is_empty() && !path.starts_with('(') {
                targets.push(path.to_string());
            }
        }
    }

    targets
}

/// Check if a command is a write operation that needs path validation.
fn is_write_command(base_cmd: &str, segment: &str) -> bool {
    if WRITE_COMMANDS.contains(&base_cmd) {
        return true;
    }
    // sed -i is a write
    if base_cmd == "sed" {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        return tokens.iter().any(|t| *t == "-i" || t.starts_with("-i"));
    }
    // Check for output redirection (any command writing to a file)
    if segment.contains(" > ") || segment.contains(" >> ")
        || segment.contains("\t>") || segment.contains(" >")
    {
        return true;
    }
    false
}

// ─── Traversal detection ────────────────────────────────────────────────────

/// Detect path traversal attempts.
pub fn has_traversal(raw_path: &str) -> bool {
    let normalized = raw_path.replace('\\', "/");
    normalized.contains("/../")
        || normalized.starts_with("../")
        || normalized.ends_with("/..")
        || normalized == ".."
        || normalized.contains("/./") && normalized.contains("..")
}

// ─── Symlink resolution ─────────────────────────────────────────────────────

/// Resolve symlinks in a path to get the real target.
/// Falls back to the original path if resolution fails (file doesn't exist yet).
fn resolve_symlinks(path: &Path) -> PathBuf {
    // Try full canonicalization first
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    // If the file doesn't exist, try to resolve parent components
    if let Some(parent) = path.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if let Some(filename) = path.file_name() {
                return canonical_parent.join(filename);
            }
        }
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_validator() -> PathValidator {
        PathValidator::with_home(
            vec![PathBuf::from("/workspace/project")],
            PathBuf::from("/home/testuser"),
        )
    }

    fn unrestricted_validator() -> PathValidator {
        PathValidator::with_home(
            vec![],
            PathBuf::from("/home/testuser"),
        )
    }

    // ── Traversal detection ─────────────────────────────────────────

    #[test]
    fn detects_traversal_basic() {
        assert!(has_traversal("../etc/passwd"));
    }

    #[test]
    fn detects_traversal_middle() {
        assert!(has_traversal("/workspace/../etc/shadow"));
    }

    #[test]
    fn detects_traversal_end() {
        assert!(has_traversal("/workspace/subdir/.."));
    }

    #[test]
    fn detects_traversal_dotdot_only() {
        assert!(has_traversal(".."));
    }

    #[test]
    fn detects_traversal_backslash() {
        assert!(has_traversal("..\\..\\etc\\passwd"));
    }

    #[test]
    fn no_traversal_normal_path() {
        assert!(!has_traversal("/workspace/project/src/main.rs"));
    }

    #[test]
    fn no_traversal_relative() {
        assert!(!has_traversal("src/main.rs"));
    }

    #[test]
    fn no_traversal_dotfile() {
        assert!(!has_traversal("/workspace/.gitignore"));
    }

    // ── Protected paths ─────────────────────────────────────────────

    #[test]
    fn blocks_ssh_dir() {
        let v = unrestricted_validator();
        let result = v.validate("rm ~/.ssh/id_rsa");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".ssh")));
    }

    #[test]
    fn blocks_gnupg() {
        let v = unrestricted_validator();
        let result = v.validate("rm ~/.gnupg/pubring.kbx");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".gnupg")));
    }

    #[test]
    fn blocks_etc_shadow() {
        let v = unrestricted_validator();
        let result = v.validate("cp malicious /etc/shadow");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("/etc/shadow")));
    }

    #[test]
    fn blocks_etc_passwd() {
        let v = unrestricted_validator();
        let result = v.validate("tee /etc/passwd");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("/etc/passwd")));
    }

    #[test]
    fn blocks_bashrc() {
        let v = unrestricted_validator();
        let result = v.validate("touch ~/.bashrc");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".bashrc")));
    }

    #[test]
    fn blocks_aws_credentials() {
        let v = unrestricted_validator();
        let result = v.validate("cp new_creds ~/.aws/credentials");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".aws/credentials")));
    }

    #[test]
    fn blocks_docker_config() {
        let v = unrestricted_validator();
        let result = v.validate("rm ~/.docker/config.json");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".docker/config.json")));
    }

    #[test]
    fn blocks_etc_sudoers() {
        let v = unrestricted_validator();
        let result = v.validate("tee /etc/sudoers");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("/etc/sudoers")));
    }

    #[test]
    fn blocks_boot() {
        let v = unrestricted_validator();
        let result = v.validate("cp kernel /boot/vmlinuz");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("/boot")));
    }

    // ── Workspace path validation ───────────────────────────────────

    #[test]
    fn allows_workspace_path() {
        let v = PathValidator::new(vec![std::env::current_dir().unwrap()]);
        let cwd = std::env::current_dir().unwrap();
        let test_path = cwd.join("test_file.txt");
        let cmd = format!("touch {}", test_path.display());
        let result = v.validate(&cmd);
        assert!(result.is_safe());
    }

    #[test]
    fn blocks_outside_workspace() {
        let v = test_validator();
        let result = v.validate("touch /tmp/evil.sh");
        assert!(matches!(result, PathVerdict::Blocked { .. }));
    }

    // ── Traversal in commands ───────────────────────────────────────

    #[test]
    fn blocks_traversal_in_rm() {
        let v = unrestricted_validator();
        let result = v.validate("rm ../../etc/passwd");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("traversal")));
    }

    #[test]
    fn blocks_traversal_in_cp() {
        let v = unrestricted_validator();
        let result = v.validate("cp secret ../../../etc/shadow");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("traversal")));
    }

    // ── Safe commands don't trigger validation ──────────────────────

    #[test]
    fn safe_cat_any_path() {
        let v = test_validator();
        let result = v.validate("cat /etc/passwd");
        assert!(result.is_safe());
    }

    #[test]
    fn safe_ls_any_path() {
        let v = test_validator();
        let result = v.validate("ls ~/.ssh");
        assert!(result.is_safe());
    }

    #[test]
    fn safe_grep_any_path() {
        let v = test_validator();
        let result = v.validate("grep pattern /etc/hosts");
        assert!(result.is_safe());
    }

    // ── Output redirection ──────────────────────────────────────────

    #[test]
    fn blocks_redirect_to_protected() {
        let v = unrestricted_validator();
        let result = v.validate("echo evil > ~/.bashrc");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".bashrc")));
    }

    // ── Symlink resolution ──────────────────────────────────────────

    #[test]
    fn symlink_resolution_follows_links() {
        let tmp = std::env::temp_dir().join("fastclaw_pathtest_symlink");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let real_file = tmp.join("real.txt");
        fs::write(&real_file, "content").unwrap();

        let link_path = tmp.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &link_path).unwrap();

        #[cfg(unix)]
        {
            let resolved = resolve_symlinks(&link_path);
            assert_eq!(resolved, real_file.canonicalize().unwrap());
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn symlink_nonexistent_falls_back() {
        let path = PathBuf::from("/nonexistent/path/file.txt");
        let resolved = resolve_symlinks(&path);
        assert_eq!(resolved, path);
    }

    // ── Chain/pipe commands ──────────────────────────────────────────

    #[test]
    fn blocks_protected_in_chain() {
        let v = unrestricted_validator();
        let result = v.validate("ls && rm ~/.ssh/id_rsa");
        assert!(matches!(result, PathVerdict::Blocked { .. }));
    }

    #[test]
    fn blocks_protected_in_pipe() {
        let v = unrestricted_validator();
        let result = v.validate("echo data | tee /etc/passwd");
        assert!(matches!(result, PathVerdict::Blocked { .. }));
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn empty_command_is_safe() {
        let v = test_validator();
        assert!(v.validate("").is_safe());
    }

    #[test]
    fn sed_i_validates_paths() {
        let v = unrestricted_validator();
        let result = v.validate("sed -i 's/x/y/' ~/.bashrc");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains(".bashrc")));
    }

    #[test]
    fn mkdir_validates_paths() {
        let v = unrestricted_validator();
        let result = v.validate("mkdir /etc/ssh/backdoor");
        assert!(matches!(result, PathVerdict::Blocked { ref reason, .. } if reason.contains("/etc/ssh")));
    }

    // ── extract_paths tests ─────────────────────────────────────────

    #[test]
    fn extract_paths_basic() {
        let (cmd, paths) = extract_paths("rm file1.txt file2.txt");
        assert_eq!(cmd, "rm");
        assert_eq!(paths, vec!["file1.txt", "file2.txt"]);
    }

    #[test]
    fn extract_paths_with_flags() {
        let (cmd, paths) = extract_paths("cp -r src/ dest/");
        assert_eq!(cmd, "cp");
        assert_eq!(paths, vec!["src/", "dest/"]);
    }

    #[test]
    fn extract_paths_after_double_dash() {
        let (_, paths) = extract_paths("rm -- -weird-file");
        assert_eq!(paths, vec!["-weird-file"]);
    }

    #[test]
    fn extract_paths_target_dir_flag() {
        let (_, paths) = extract_paths("cp file.txt -t /target/dir");
        assert!(paths.contains(&"/target/dir".to_string()));
    }

    // ── extract_redirect_targets ────────────────────────────────────

    #[test]
    fn redirect_target_basic() {
        let targets = extract_redirect_targets("echo hello > output.txt");
        assert_eq!(targets, vec!["output.txt"]);
    }

    #[test]
    fn redirect_target_append() {
        let targets = extract_redirect_targets("echo data >> log.txt");
        assert_eq!(targets, vec!["log.txt"]);
    }
}
