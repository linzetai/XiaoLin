use super::shell_security::{SecurityVerdict, ShellSecurityChecker};

/// Three-level classification of a shell command's side-effect risk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandClassification {
    ReadOnly,
    Write { reason: String },
    Dangerous { reason: String },
}

impl CommandClassification {
    pub fn is_readonly(&self) -> bool {
        matches!(self, CommandClassification::ReadOnly)
    }

    pub fn is_write(&self) -> bool {
        matches!(self, CommandClassification::Write { .. })
    }

    pub fn is_dangerous(&self) -> bool {
        matches!(self, CommandClassification::Dangerous { .. })
    }

    /// Merge two classifications, taking the more severe one.
    fn merge(self, other: CommandClassification) -> CommandClassification {
        match (&self, &other) {
            (CommandClassification::Dangerous { .. }, _) => self,
            (_, CommandClassification::Dangerous { .. }) => other,
            (CommandClassification::Write { .. }, _) => self,
            (_, CommandClassification::Write { .. }) => other,
            _ => CommandClassification::ReadOnly,
        }
    }
}

/// Classify a full shell command (supporting pipes, chains, and redirections).
///
/// The overall classification is the **most severe** across all segments:
/// - All segments ReadOnly → ReadOnly
/// - Any segment Write (and none Dangerous) → Write
/// - Any segment Dangerous → Dangerous
pub struct ReadOnlyClassifier;

impl ReadOnlyClassifier {
    /// Classify a complete command string.
    pub fn classify(command: &str) -> CommandClassification {
        // First check for injection/dangerous patterns
        let security = ShellSecurityChecker::check(command);
        if let SecurityVerdict::Blocked { reason, .. } = security {
            return CommandClassification::Dangerous { reason };
        }

        let mut result = CommandClassification::ReadOnly;

        for pipe_segment in command.split('|') {
            let pipe_seg = pipe_segment.trim();
            if pipe_seg.is_empty() {
                continue;
            }
            for part in pipe_seg
                .split("&&")
                .flat_map(|s| s.split("||"))
                .flat_map(|s| s.split(';'))
            {
                let cls = classify_segment(part.trim());
                result = result.merge(cls);
                if result.is_dangerous() {
                    return result;
                }
            }
        }

        // NeedsConfirmation from security checker → treat as Write
        if let SecurityVerdict::NeedsConfirmation { reason, .. } = security {
            result = result.merge(CommandClassification::Write { reason });
        }

        result
    }

    /// Classify and return classifications for every segment.
    pub fn classify_all(command: &str) -> Vec<(String, CommandClassification)> {
        let mut results = Vec::new();

        for pipe_segment in command.split('|') {
            let pipe_seg = pipe_segment.trim();
            if pipe_seg.is_empty() {
                continue;
            }
            for part in pipe_seg
                .split("&&")
                .flat_map(|s| s.split("||"))
                .flat_map(|s| s.split(';'))
            {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    let cls = classify_segment(trimmed);
                    results.push((trimmed.to_string(), cls));
                }
            }
        }

        results
    }
}

// ─── Readonly whitelist ─────────────────────────────────────────────────────

const READONLY_COMMANDS: &[&str] = &[
    // File inspection
    "ls",
    "ll",
    "la",
    "dir",
    "exa",
    "eza",
    "lsd",
    "cat",
    "bat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "file",
    "stat",
    "du",
    "df",
    // Search
    "grep",
    "rg",
    "ag",
    "ack",
    "fgrep",
    "egrep",
    "find",
    "fd",
    "fdfind",
    "locate",
    "which",
    "whereis",
    "type",
    // Text processing (readonly — sed -i checked separately)
    "sort",
    "uniq",
    "tr",
    "cut",
    "paste",
    "column",
    "awk",
    "sed",
    "diff",
    "comm",
    "cmp",
    "jq",
    "yq",
    "xq",
    // System info
    "echo",
    "printf",
    "date",
    "whoami",
    "hostname",
    "uname",
    "env",
    "printenv",
    "id",
    "groups",
    "ps",
    "top",
    "htop",
    "free",
    "uptime",
    "lsof",
    "pwd",
    "realpath",
    "dirname",
    "basename",
    // Network (readonly)
    "ping",
    "traceroute",
    "dig",
    "nslookup",
    "host",
    "curl",
    "wget",
    "ss",
    "netstat",
    "ip",
    // Development tools
    "tree",
    "tokei",
    "cloc",
    "scc",
    "python3",
    "python",
    "node",
    "ruby",
    "cargo",
    "npm",
    "npx",
    "yarn",
    "pnpm",
    "git",
    "gh",
    "docker",
    "kubectl",
    "rustc",
    "gcc",
    "g++",
    "clang",
    "make",
    "cmake",
    // Misc safe
    "test",
    "[",
    "true",
    "false",
    "sleep",
    "xargs",
    "tput",
    "clear",
    "reset",
    "man",
    "help",
    "info",
    "md5sum",
    "sha256sum",
    "sha1sum",
    "shasum",
    "base64",
    "xxd",
    "hexdump",
    "od",
    "tar", // only listing (checked separately when extracting)
];

// ─── Subcommand whitelists ──────────────────────────────────────────────────

const GIT_READONLY_SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "tag",
    "describe",
    "shortlog",
    "blame",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "rev-list",
    "remote",
    "config",
    "stash",
    "reflog",
    "worktree",
];

const GIT_WRITE_SUBCOMMANDS: &[&str] = &[
    "add",
    "commit",
    "push",
    "pull",
    "fetch",
    "merge",
    "rebase",
    "cherry-pick",
    "checkout",
    "switch",
    "restore",
    "reset",
    "revert",
    "clean",
    "rm",
    "mv",
];

const GIT_DANGEROUS_SUBCOMMANDS: &[&str] = &["push --force", "reset --hard", "clean -fd"];

const CARGO_READONLY_SUBCOMMANDS: &[&str] = &[
    "check",
    "clippy",
    "test",
    "bench",
    "doc",
    "tree",
    "metadata",
    "pkgid",
    "verify-project",
    "version",
    "help",
    "search",
    "fmt",
];

const CARGO_WRITE_SUBCOMMANDS: &[&str] = &[
    "install",
    "uninstall",
    "add",
    "remove",
    "publish",
    "yank",
    "init",
    "new",
    "build",
    "run",
];

const NPM_READONLY_SUBCOMMANDS: &[&str] = &[
    "list", "ls", "info", "show", "view", "outdated", "audit", "explain", "why", "help", "version",
    "test", "run", "exec",
];

const NPM_WRITE_SUBCOMMANDS: &[&str] = &[
    "install",
    "i",
    "ci",
    "uninstall",
    "remove",
    "publish",
    "unpublish",
    "link",
    "init",
    "create",
    "update",
    "upgrade",
];

const DOCKER_READONLY_SUBCOMMANDS: &[&str] = &[
    "ps", "images", "inspect", "logs", "stats", "top", "port", "diff", "history", "version",
    "info", "network", "volume",
];

const DOCKER_WRITE_SUBCOMMANDS: &[&str] = &[
    "run", "exec", "build", "push", "pull", "rm", "rmi", "stop", "kill", "restart", "create",
    "compose",
];

const KUBECTL_READONLY_SUBCOMMANDS: &[&str] = &[
    "get",
    "describe",
    "logs",
    "top",
    "explain",
    "api-resources",
    "api-versions",
    "cluster-info",
    "version",
    "config",
];

const KUBECTL_WRITE_SUBCOMMANDS: &[&str] = &[
    "apply", "create", "delete", "patch", "edit", "replace", "scale", "rollout", "expose", "run",
    "exec",
];

// ─── Write commands ─────────────────────────────────────────────────────────

const WRITE_COMMANDS: &[&str] = &[
    "rm",
    "rmdir",
    "mv",
    "cp",
    "mkdir",
    "mktemp",
    "touch",
    "truncate",
    "chmod",
    "chown",
    "chgrp",
    "ln",
    "unlink",
    "shred",
    "tee",
    "dd",
    "install",
    "patch",
    "pip",
    "pip3",
    "apt",
    "apt-get",
    "yum",
    "dnf",
    "brew",
    "pacman",
    "systemctl",
    "service",
    "crontab",
    "useradd",
    "userdel",
    "usermod",
    "groupadd",
    "mount",
    "umount",
];

// ─── Dangerous commands ─────────────────────────────────────────────────────

const DANGEROUS_COMMANDS: &[&str] = &[
    "mkfs",
    "fdisk",
    "parted",
    "iptables",
    "ip6tables",
    "nft",
    "reboot",
    "shutdown",
    "halt",
    "poweroff",
    "insmod",
    "rmmod",
    "modprobe",
    "chroot",
];

// ─── Classification logic ───────────────────────────────────────────────────

fn classify_segment(segment: &str) -> CommandClassification {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return CommandClassification::ReadOnly;
    }

    // Output redirection → write
    if has_output_redirection(trimmed) {
        return CommandClassification::Write {
            reason: "output redirection (> or >>) modifies files".into(),
        };
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return CommandClassification::ReadOnly;
    }

    let base_cmd = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);
    let args = &tokens[1..];

    // Dangerous commands are always dangerous (also check base before '.')
    let base_before_dot = base_cmd.split('.').next().unwrap_or(base_cmd);
    if DANGEROUS_COMMANDS.contains(&base_cmd) || DANGEROUS_COMMANDS.contains(&base_before_dot) {
        return CommandClassification::Dangerous {
            reason: format!("'{base_cmd}' is a dangerous system command"),
        };
    }

    // Subcommand-aware classification
    if base_cmd == "git" {
        return classify_git(args);
    }
    if base_cmd == "cargo" {
        return classify_with_subcommands(
            args,
            CARGO_READONLY_SUBCOMMANDS,
            CARGO_WRITE_SUBCOMMANDS,
            "cargo",
        );
    }
    if matches!(base_cmd, "npm" | "npx" | "yarn" | "pnpm") {
        return classify_with_subcommands(
            args,
            NPM_READONLY_SUBCOMMANDS,
            NPM_WRITE_SUBCOMMANDS,
            base_cmd,
        );
    }
    if base_cmd == "docker" {
        return classify_with_subcommands(
            args,
            DOCKER_READONLY_SUBCOMMANDS,
            DOCKER_WRITE_SUBCOMMANDS,
            "docker",
        );
    }
    if base_cmd == "kubectl" {
        return classify_with_subcommands(
            args,
            KUBECTL_READONLY_SUBCOMMANDS,
            KUBECTL_WRITE_SUBCOMMANDS,
            "kubectl",
        );
    }

    // sed -i is a write operation
    if base_cmd == "sed" && tokens.iter().any(|t| *t == "-i" || t.starts_with("-i")) {
        return CommandClassification::Write {
            reason: "sed -i modifies files in place".into(),
        };
    }

    // tar with extract/create flags is write (only check first arg which is the mode)
    if base_cmd == "tar" {
        if let Some(mode) = args.first() {
            let m = mode.trim_start_matches('-');
            if m.contains('x') || m.contains('c') {
                return CommandClassification::Write {
                    reason: "tar extract/create modifies the filesystem".into(),
                };
            }
        }
    }

    // curl/wget with output flags are write
    if base_cmd == "curl"
        && tokens
            .iter()
            .any(|t| matches!(*t, "-o" | "-O" | "--output"))
    {
        return CommandClassification::Write {
            reason: "curl with -o/-O writes to files".into(),
        };
    }
    if base_cmd == "wget" && !tokens.iter().any(|t| matches!(*t, "--spider" | "-q")) {
        // wget without --spider downloads files
        if tokens.len() > 1 {
            return CommandClassification::Write {
                reason: "wget downloads and writes files".into(),
            };
        }
    }

    // Explicit write commands
    if WRITE_COMMANDS.contains(&base_cmd) {
        return CommandClassification::Write {
            reason: format!("'{base_cmd}' modifies files or system state"),
        };
    }

    // Readonly whitelist
    if READONLY_COMMANDS.contains(&base_cmd) {
        return CommandClassification::ReadOnly;
    }

    // Unknown commands default to Write (safe-by-default)
    CommandClassification::Write {
        reason: format!("'{base_cmd}' is not in the read-only allowlist"),
    }
}

fn classify_git(args: &[&str]) -> CommandClassification {
    let subcommand = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("");

    if subcommand.is_empty() || GIT_READONLY_SUBCOMMANDS.contains(&subcommand) {
        return CommandClassification::ReadOnly;
    }

    // Check for dangerous git operations
    let joined = args.join(" ");
    for dangerous in GIT_DANGEROUS_SUBCOMMANDS {
        if joined.contains(dangerous) {
            return CommandClassification::Dangerous {
                reason: format!("git {dangerous} is a destructive operation"),
            };
        }
    }

    if GIT_WRITE_SUBCOMMANDS.contains(&subcommand) {
        return CommandClassification::Write {
            reason: format!("git {subcommand} modifies repository state"),
        };
    }

    CommandClassification::Write {
        reason: format!("git {subcommand} is not a known read-only git operation"),
    }
}

fn classify_with_subcommands(
    args: &[&str],
    readonly: &[&str],
    write: &[&str],
    parent: &str,
) -> CommandClassification {
    let subcommand = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .copied()
        .unwrap_or("");

    if subcommand.is_empty() || readonly.contains(&subcommand) {
        CommandClassification::ReadOnly
    } else if write.contains(&subcommand) {
        CommandClassification::Write {
            reason: format!("{parent} {subcommand} modifies state"),
        }
    } else {
        CommandClassification::Write {
            reason: format!("{parent} {subcommand} is not a known read-only operation"),
        }
    }
}

/// Check for output redirection (> or >>), skipping 2> and >(
fn has_output_redirection(s: &str) -> bool {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        let ch = bytes[i];

        if ch == b'\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }
        if ch == b'"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }
        if in_single_quote || in_double_quote {
            i += 1;
            continue;
        }

        if ch == b'>' {
            // Skip 2> (stderr redirect)
            if i > 0 && bytes[i - 1] == b'2' {
                i += 1;
                continue;
            }
            // Skip >( (process substitution)
            let next = if i + 1 < len && bytes[i + 1] == b'>' {
                i + 2
            } else {
                i + 1
            };
            if next < len && bytes[next] == b'(' {
                i = next + 1;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ReadOnly basics ─────────────────────────────────────────────

    #[test]
    fn readonly_ls() {
        assert!(ReadOnlyClassifier::classify("ls -la").is_readonly());
    }

    #[test]
    fn readonly_cat() {
        assert!(ReadOnlyClassifier::classify("cat README.md").is_readonly());
    }

    #[test]
    fn readonly_grep() {
        assert!(ReadOnlyClassifier::classify("grep -r 'TODO' src/").is_readonly());
    }

    #[test]
    fn readonly_head_tail() {
        assert!(ReadOnlyClassifier::classify("head -n 10 file.txt").is_readonly());
        assert!(ReadOnlyClassifier::classify("tail -f log.txt").is_readonly());
    }

    #[test]
    fn readonly_wc() {
        assert!(ReadOnlyClassifier::classify("wc -l *.rs").is_readonly());
    }

    #[test]
    fn readonly_find() {
        assert!(ReadOnlyClassifier::classify("find . -name '*.rs' -type f").is_readonly());
    }

    #[test]
    fn readonly_rg() {
        assert!(ReadOnlyClassifier::classify("rg 'pattern' src/").is_readonly());
    }

    #[test]
    fn readonly_echo() {
        assert!(ReadOnlyClassifier::classify("echo hello world").is_readonly());
    }

    #[test]
    fn readonly_pwd() {
        assert!(ReadOnlyClassifier::classify("pwd").is_readonly());
    }

    #[test]
    fn readonly_env() {
        assert!(ReadOnlyClassifier::classify("env").is_readonly());
    }

    #[test]
    fn readonly_ps() {
        assert!(ReadOnlyClassifier::classify("ps aux").is_readonly());
    }

    #[test]
    fn readonly_which() {
        assert!(ReadOnlyClassifier::classify("which cargo").is_readonly());
    }

    #[test]
    fn readonly_df_du() {
        assert!(ReadOnlyClassifier::classify("df -h").is_readonly());
        assert!(ReadOnlyClassifier::classify("du -sh *").is_readonly());
    }

    #[test]
    fn readonly_jq() {
        assert!(ReadOnlyClassifier::classify("jq '.name' package.json").is_readonly());
    }

    #[test]
    fn readonly_sed_without_i() {
        assert!(ReadOnlyClassifier::classify("sed 's/old/new/g' file.txt").is_readonly());
    }

    #[test]
    fn readonly_awk() {
        assert!(ReadOnlyClassifier::classify("awk '{print $1}' data.txt").is_readonly());
    }

    #[test]
    fn readonly_sort_uniq() {
        assert!(ReadOnlyClassifier::classify("sort file.txt | uniq").is_readonly());
    }

    #[test]
    fn readonly_diff() {
        assert!(ReadOnlyClassifier::classify("diff a.txt b.txt").is_readonly());
    }

    #[test]
    fn readonly_md5sum() {
        assert!(ReadOnlyClassifier::classify("md5sum file.bin").is_readonly());
    }

    #[test]
    fn readonly_base64() {
        assert!(ReadOnlyClassifier::classify("base64 file.bin").is_readonly());
    }

    // ── Git readonly ────────────────────────────────────────────────

    #[test]
    fn readonly_git_status() {
        assert!(ReadOnlyClassifier::classify("git status").is_readonly());
    }

    #[test]
    fn readonly_git_log() {
        assert!(ReadOnlyClassifier::classify("git log --oneline -10").is_readonly());
    }

    #[test]
    fn readonly_git_diff() {
        assert!(ReadOnlyClassifier::classify("git diff HEAD~1").is_readonly());
    }

    #[test]
    fn readonly_git_show() {
        assert!(ReadOnlyClassifier::classify("git show HEAD").is_readonly());
    }

    #[test]
    fn readonly_git_branch() {
        assert!(ReadOnlyClassifier::classify("git branch -a").is_readonly());
    }

    #[test]
    fn readonly_git_blame() {
        assert!(ReadOnlyClassifier::classify("git blame src/main.rs").is_readonly());
    }

    // ── Cargo readonly ──────────────────────────────────────────────

    #[test]
    fn readonly_cargo_check() {
        assert!(ReadOnlyClassifier::classify("cargo check").is_readonly());
    }

    #[test]
    fn readonly_cargo_clippy() {
        assert!(ReadOnlyClassifier::classify("cargo clippy --workspace").is_readonly());
    }

    #[test]
    fn readonly_cargo_test() {
        assert!(ReadOnlyClassifier::classify("cargo test -p xiaolin-agent").is_readonly());
    }

    #[test]
    fn readonly_cargo_tree() {
        assert!(ReadOnlyClassifier::classify("cargo tree").is_readonly());
    }

    // ── npm readonly ────────────────────────────────────────────────

    #[test]
    fn readonly_npm_list() {
        assert!(ReadOnlyClassifier::classify("npm list").is_readonly());
    }

    #[test]
    fn readonly_npm_test() {
        assert!(ReadOnlyClassifier::classify("npm test").is_readonly());
    }

    #[test]
    fn readonly_npm_run() {
        assert!(ReadOnlyClassifier::classify("npm run lint").is_readonly());
    }

    // ── Docker readonly ─────────────────────────────────────────────

    #[test]
    fn readonly_docker_ps() {
        assert!(ReadOnlyClassifier::classify("docker ps").is_readonly());
    }

    #[test]
    fn readonly_docker_logs() {
        assert!(ReadOnlyClassifier::classify("docker logs container_id").is_readonly());
    }

    // ── Pipe chains ─────────────────────────────────────────────────

    #[test]
    fn readonly_pipe_all_safe() {
        assert!(ReadOnlyClassifier::classify("cat file.txt | grep error | wc -l").is_readonly());
    }

    #[test]
    fn readonly_chain_all_safe() {
        assert!(ReadOnlyClassifier::classify("ls && cat file.txt").is_readonly());
    }

    #[test]
    fn readonly_complex_pipe() {
        assert!(
            ReadOnlyClassifier::classify("find . -name '*.rs' | head -20 | sort").is_readonly()
        );
    }

    // ── Write detection ─────────────────────────────────────────────

    #[test]
    fn write_rm() {
        let cls = ReadOnlyClassifier::classify("rm file.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn write_mv() {
        let cls = ReadOnlyClassifier::classify("mv a.txt b.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn write_cp() {
        let cls = ReadOnlyClassifier::classify("cp src dst");
        assert!(cls.is_write());
    }

    #[test]
    fn write_mkdir() {
        let cls = ReadOnlyClassifier::classify("mkdir -p new_dir");
        assert!(cls.is_write());
    }

    #[test]
    fn write_touch() {
        let cls = ReadOnlyClassifier::classify("touch new_file.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn write_chmod() {
        let cls = ReadOnlyClassifier::classify("chmod +x script.sh");
        assert!(cls.is_write());
    }

    #[test]
    fn write_sed_i() {
        let cls = ReadOnlyClassifier::classify("sed -i 's/old/new/g' file");
        assert!(cls.is_write());
    }

    #[test]
    fn write_redirect_stdout() {
        let cls = ReadOnlyClassifier::classify("echo hello > file.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn write_redirect_append() {
        let cls = ReadOnlyClassifier::classify("cat x >> output.log");
        assert!(cls.is_write());
    }

    #[test]
    fn write_tee() {
        let cls = ReadOnlyClassifier::classify("echo data | tee output.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn write_dd() {
        let cls = ReadOnlyClassifier::classify("dd if=/dev/zero of=disk.img bs=1M count=100");
        assert!(cls.is_write());
    }

    // ── Git write ───────────────────────────────────────────────────

    #[test]
    fn write_git_commit() {
        let cls = ReadOnlyClassifier::classify("git commit -m 'msg'");
        assert!(cls.is_write());
    }

    #[test]
    fn write_git_push() {
        let cls = ReadOnlyClassifier::classify("git push origin main");
        assert!(cls.is_write());
    }

    #[test]
    fn write_git_add() {
        let cls = ReadOnlyClassifier::classify("git add .");
        assert!(cls.is_write());
    }

    #[test]
    fn write_git_checkout() {
        let cls = ReadOnlyClassifier::classify("git checkout -b new-branch");
        assert!(cls.is_write());
    }

    // ── Cargo write ─────────────────────────────────────────────────

    #[test]
    fn write_cargo_install() {
        let cls = ReadOnlyClassifier::classify("cargo install foo");
        assert!(cls.is_write());
    }

    #[test]
    fn write_cargo_add() {
        let cls = ReadOnlyClassifier::classify("cargo add serde");
        assert!(cls.is_write());
    }

    #[test]
    fn write_cargo_publish() {
        let cls = ReadOnlyClassifier::classify("cargo publish");
        assert!(cls.is_write());
    }

    // ── npm write ───────────────────────────────────────────────────

    #[test]
    fn write_npm_install() {
        let cls = ReadOnlyClassifier::classify("npm install express");
        assert!(cls.is_write());
    }

    #[test]
    fn write_npm_publish() {
        let cls = ReadOnlyClassifier::classify("npm publish");
        assert!(cls.is_write());
    }

    // ── Docker write ────────────────────────────────────────────────

    #[test]
    fn write_docker_run() {
        let cls = ReadOnlyClassifier::classify("docker run -it ubuntu");
        assert!(cls.is_write());
    }

    #[test]
    fn write_docker_rm() {
        let cls = ReadOnlyClassifier::classify("docker rm container_id");
        assert!(cls.is_write());
    }

    // ── kubectl write ───────────────────────────────────────────────

    #[test]
    fn write_kubectl_apply() {
        let cls = ReadOnlyClassifier::classify("kubectl apply -f deployment.yaml");
        assert!(cls.is_write());
    }

    #[test]
    fn write_kubectl_delete() {
        let cls = ReadOnlyClassifier::classify("kubectl delete pod my-pod");
        assert!(cls.is_write());
    }

    // ── Package manager write ───────────────────────────────────────

    #[test]
    fn write_pip_install() {
        let cls = ReadOnlyClassifier::classify("pip install requests");
        assert!(cls.is_write());
    }

    #[test]
    fn write_apt_install() {
        let cls = ReadOnlyClassifier::classify("apt install curl");
        assert!(cls.is_write());
    }

    // ── Dangerous commands ──────────────────────────────────────────

    #[test]
    fn dangerous_mkfs() {
        let cls = ReadOnlyClassifier::classify("mkfs.ext4 /dev/sda1");
        assert!(cls.is_dangerous());
    }

    #[test]
    fn dangerous_reboot() {
        let cls = ReadOnlyClassifier::classify("reboot");
        assert!(cls.is_dangerous());
    }

    #[test]
    fn dangerous_eval() {
        let cls = ReadOnlyClassifier::classify("eval 'rm -rf /'");
        assert!(cls.is_dangerous());
    }

    #[test]
    fn dangerous_command_substitution() {
        let cls = ReadOnlyClassifier::classify("echo $(whoami)");
        assert!(cls.is_dangerous());
    }

    #[test]
    fn dangerous_ld_preload() {
        let cls = ReadOnlyClassifier::classify("LD_PRELOAD=/tmp/evil.so ls");
        assert!(cls.is_dangerous());
    }

    // ── Mixed chains ────────────────────────────────────────────────

    #[test]
    fn write_in_chain() {
        let cls = ReadOnlyClassifier::classify("ls && rm file.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn dangerous_in_chain() {
        let cls = ReadOnlyClassifier::classify("ls; eval 'bad command'");
        assert!(cls.is_dangerous());
    }

    #[test]
    fn write_in_pipe() {
        let cls = ReadOnlyClassifier::classify("echo data | tee output.txt");
        assert!(cls.is_write());
    }

    #[test]
    fn readonly_git_status_chain() {
        assert!(ReadOnlyClassifier::classify("git status; git diff").is_readonly());
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn empty_command() {
        assert!(ReadOnlyClassifier::classify("").is_readonly());
    }

    #[test]
    fn unknown_command_is_write() {
        let cls = ReadOnlyClassifier::classify("some_random_tool --arg");
        assert!(cls.is_write());
    }

    #[test]
    fn full_path_command() {
        assert!(ReadOnlyClassifier::classify("/usr/bin/ls -la").is_readonly());
    }

    #[test]
    fn curl_with_output() {
        let cls = ReadOnlyClassifier::classify("curl -o file.txt http://example.com");
        assert!(cls.is_write());
    }

    #[test]
    fn curl_readonly() {
        assert!(ReadOnlyClassifier::classify("curl -s http://example.com").is_readonly());
    }

    #[test]
    fn tar_list_readonly() {
        assert!(ReadOnlyClassifier::classify("tar tf archive.tar.gz").is_readonly());
    }

    #[test]
    fn tar_extract_write() {
        let cls = ReadOnlyClassifier::classify("tar xzf archive.tar.gz");
        assert!(cls.is_write());
    }

    // ── classify_all ────────────────────────────────────────────────

    #[test]
    fn classify_all_mixed_chain() {
        let results = ReadOnlyClassifier::classify_all("ls && rm file.txt");
        assert_eq!(results.len(), 2);
        assert!(results[0].1.is_readonly());
        assert!(results[1].1.is_write());
    }

    #[test]
    fn classify_all_pipe_chain() {
        let results = ReadOnlyClassifier::classify_all("cat file | grep err | wc -l");
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, c)| c.is_readonly()));
    }

    // ── Redirect edge cases ─────────────────────────────────────────

    #[test]
    fn stderr_redirect_is_ok() {
        assert!(ReadOnlyClassifier::classify("ls 2>/dev/null").is_readonly());
    }

    #[test]
    fn redirect_in_quotes_is_ok() {
        assert!(ReadOnlyClassifier::classify("echo 'a > b'").is_readonly());
    }

    #[test]
    fn git_push_force_is_dangerous() {
        let cls = ReadOnlyClassifier::classify("git push --force origin main");
        assert!(cls.is_dangerous());
    }

    #[test]
    fn git_reset_hard_is_dangerous() {
        let cls = ReadOnlyClassifier::classify("git reset --hard HEAD~1");
        assert!(cls.is_dangerous());
    }

    // ── wget ────────────────────────────────────────────────────────

    #[test]
    fn wget_download_is_write() {
        let cls = ReadOnlyClassifier::classify("wget http://example.com/file.zip");
        assert!(cls.is_write());
    }
}
