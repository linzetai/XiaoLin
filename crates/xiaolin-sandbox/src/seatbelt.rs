use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use xiaolin_security::{
    FileSystemAccessMode, FileSystemPath, FileSystemSandboxKind, FileSystemSandboxPolicy,
    NetworkSandboxPolicy,
};

use crate::{SandboxType, SandboxedCommand};

const SEATBELT_BASE_POLICY: &str = include_str!("seatbelt_base_policy.sbpl");
const SEATBELT_NETWORK_POLICY: &str = include_str!("seatbelt_network_policy.sbpl");
const SEATBELT_PLATFORM_DEFAULTS: &str =
    include_str!("restricted_read_only_platform_defaults.sbpl");

/// Hardcoded path to `sandbox-exec` to defend against PATH hijacking.
/// If this path is tampered with, the attacker already has root access.
const SEATBELT_EXECUTABLE: &str = "/usr/bin/sandbox-exec";

/// Check if sandbox-exec (Seatbelt) is available on this macOS system.
#[cfg(target_os = "macos")]
pub fn is_available() -> bool {
    std::path::Path::new(SEATBELT_EXECUTABLE).exists()
}

#[cfg(not(target_os = "macos"))]
pub fn is_available() -> bool {
    false
}

/// Transform a shell command into a Seatbelt-sandboxed command.
///
/// Uses `/usr/bin/sandbox-exec -p <policy>` to run the command within a
/// Seatbelt sandbox.
pub fn transform(
    command: &str,
    shell: &str,
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    cwd: &Path,
) -> SandboxedCommand {
    let policy = build_seatbelt_policy(fs_policy, net_policy, cwd);

    let mut env = HashMap::new();
    env.insert("XIAOLIN_SANDBOXED".to_string(), "1".to_string());

    SandboxedCommand {
        program: SEATBELT_EXECUTABLE.to_string(),
        args: vec![
            "-p".to_string(),
            policy,
            "--".to_string(),
            shell.to_string(),
            "-c".to_string(),
            command.to_string(),
        ],
        working_dir: Some(cwd.to_path_buf()),
        env,
        env_remove: Vec::new(),
        sandbox_type: SandboxType::Seatbelt,
        linux_sandbox: None,
    }
}

fn build_seatbelt_policy(
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    cwd: &Path,
) -> String {
    let mut sections = vec![SEATBELT_BASE_POLICY.to_string()];

    sections.push(build_filesystem_policy(fs_policy, cwd));

    if fs_policy.kind == FileSystemSandboxKind::Restricted {
        let deny_globs: Vec<String> = fs_policy
            .entries
            .iter()
            .filter(|e| e.access == FileSystemAccessMode::None)
            .filter_map(|e| match &e.path {
                FileSystemPath::GlobPattern { pattern } => Some(pattern.clone()),
                _ => None,
            })
            .collect();
        if !deny_globs.is_empty() {
            sections.push(build_seatbelt_unreadable_glob_policy(&deny_globs));
        }

        let has_readable = fs_policy.entries.iter().any(|e| {
            e.access.can_read()
                && !e.access.can_write()
                && matches!(&e.path, FileSystemPath::Path { .. })
        });
        if has_readable {
            sections.push(SEATBELT_PLATFORM_DEFAULTS.to_string());
        }
    }

    sections.push(build_network_policy(net_policy));

    sections.join("\n")
}

/// A filesystem root with optional exclusions and protected metadata names,
/// used to generate parameterized Seatbelt policy rules.
#[derive(Debug, Clone)]
pub struct SeatbeltAccessRoot {
    pub root: PathBuf,
    pub excluded_subpaths: Vec<PathBuf>,
    pub protected_metadata_names: Vec<String>,
}

impl SeatbeltAccessRoot {
    pub fn simple(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            excluded_subpaths: Vec::new(),
            protected_metadata_names: Vec::new(),
        }
    }
}

/// Build Seatbelt policy rules for a set of access roots.
///
/// - `action`: Seatbelt permission action, e.g. `"file-write*"` or `"file-read*"`.
/// - `param_prefix`: prefix used for parameter naming, e.g. `"WRITABLE_ROOT"`.
/// - `roots`: access root descriptors.
///
/// Returns a vector of `(param_name, param_value)` for `-D` args and a
/// policy string containing the S-expression rules.
///
/// Note: paths are inlined directly as `(subpath "/path")` because the policy
/// is passed via `-p` (inline string) rather than `-f` (file), and `(param ...)`
/// references don't resolve without corresponding `-D` arguments.
pub fn build_seatbelt_access_policy(
    action: &str,
    param_prefix: &str,
    roots: &[SeatbeltAccessRoot],
) -> (Vec<(String, String)>, String) {
    let mut params = Vec::new();
    let mut rules = Vec::new();

    for (i, root) in roots.iter().enumerate() {
        let param_name = if roots.len() == 1 {
            param_prefix.to_string()
        } else {
            format!("{param_prefix}_{i}")
        };
        let path = root.root.display().to_string();
        params.push((param_name.clone(), path.clone()));

        if root.excluded_subpaths.is_empty() && root.protected_metadata_names.is_empty() {
            rules.push(format!("(allow {action} (subpath \"{path}\"))"));
        } else {
            let mut conditions = vec![format!("(subpath \"{path}\")")];

            for excl in &root.excluded_subpaths {
                conditions.push(format!("(require-not (subpath \"{}\"))", excl.display()));
            }

            for name in &root.protected_metadata_names {
                let regex = seatbelt_protected_metadata_name_regex(name);
                conditions.push(format!("(require-not (regex #\"{regex}\"))"));
            }

            let inner = conditions.join("\n    ");
            rules.push(format!("(allow {action}\n  (require-all\n    {inner}))"));
        }
    }

    (params, rules.join("\n"))
}

/// Generate a Seatbelt regex that matches a metadata filename at any depth
/// under a root directory.
pub fn seatbelt_protected_metadata_name_regex(name: &str) -> String {
    let escaped = regex_escape_seatbelt(name);
    format!(".*/{escaped}$")
}

fn regex_escape_seatbelt(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

/// Convert a git-style glob pattern into a Seatbelt-compatible regex.
///
/// Glob semantics:
/// - `*` matches any characters except `/`
/// - `**` matches any characters including `/`
/// - `**/` matches zero or more path components
/// - `?` matches exactly one character except `/`
/// - `[...]` passed through as-is (character class)
/// - No glob meta-characters: treated as an exact filename + subtree match
pub fn seatbelt_regex_for_unreadable_glob(glob: &str) -> String {
    if !glob.contains('*') && !glob.contains('?') && !glob.contains('[') {
        let escaped = regex_escape_seatbelt(glob);
        return format!("(^|.*/){escaped}(/.*)?$");
    }

    let mut regex = String::new();
    let chars: Vec<char> = glob.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if i + 2 < len && chars[i + 2] == '/' {
                regex.push_str("(.*/)?");
                i += 3;
            } else {
                regex.push_str(".*");
                i += 2;
            }
        } else if chars[i] == '*' {
            regex.push_str("[^/]*");
            i += 1;
        } else if chars[i] == '?' {
            regex.push_str("[^/]");
            i += 1;
        } else if chars[i] == '[' {
            if let Some(close) = chars[i..].iter().position(|&c| c == ']') {
                let bracket: String = chars[i..=i + close].iter().collect();
                regex.push_str(&bracket);
                i += close + 1;
            } else {
                regex.push_str(&regex_escape_seatbelt(&chars[i].to_string()));
                i += 1;
            }
        } else {
            regex.push_str(&regex_escape_seatbelt(&chars[i].to_string()));
            i += 1;
        }
    }

    regex
}

/// Build Seatbelt deny rules from a list of `deny_globs`.
///
/// Each glob is converted to a Seatbelt regex and wrapped in `(deny ...)` rules
/// for both `file-read*` and `file-write-unlink`.
pub fn build_seatbelt_unreadable_glob_policy(deny_globs: &[String]) -> String {
    if deny_globs.is_empty() {
        return String::new();
    }

    let mut rules = vec!["; deny-read glob rules".to_string()];
    for glob in deny_globs {
        let mut regexes = BTreeSet::new();
        regexes.insert(seatbelt_regex_for_unreadable_glob(glob));

        if let Some(canonical) = canonicalize_glob_static_prefix_for_sandbox(glob) {
            regexes.insert(seatbelt_regex_for_unreadable_glob(&canonical));
        }

        for regex in regexes {
            let regex = regex.replace('"', "\\\"");
            rules.push(format!("(deny file-read* (regex #\"{regex}\"))"));
            rules.push(format!("(deny file-write-unlink (regex #\"{regex}\"))"));
        }
    }
    rules.join("\n")
}

/// Normalize a path for sandbox policy generation.
///
/// Rejects non-absolute paths. Attempts `std::fs::canonicalize()` to resolve
/// symlinks; falls back to the original path on failure (e.g. if the path
/// does not exist yet).
pub fn normalize_path_for_sandbox(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        tracing::warn!(?path, "rejecting non-absolute path for sandbox policy");
        return None;
    }
    Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// Canonicalize the static prefix portion of a glob pattern for sandbox policy.
///
/// Finds the first glob meta-character (`*`, `?`, `[`, `]`), extracts the static
/// directory prefix, resolves symlinks via `normalize_path_for_sandbox`, and
/// recombines with the glob suffix. Returns `None` if the result is identical to
/// the input or if there is no resolvable prefix.
pub fn canonicalize_glob_static_prefix_for_sandbox(pattern: &str) -> Option<String> {
    let first_glob_index = pattern
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '*' | '?' | '[' | ']').then_some(index));

    let Some(first_glob_index) = first_glob_index else {
        return normalize_path_for_sandbox(Path::new(pattern))
            .map(|path| path.to_string_lossy().to_string());
    };

    let static_prefix = &pattern[..first_glob_index];
    let prefix_end = if static_prefix.ends_with('/') {
        static_prefix.len() - 1
    } else {
        static_prefix.rfind('/').unwrap_or(0)
    };
    if prefix_end == 0 {
        return None;
    }

    let root = normalize_path_for_sandbox(Path::new(&pattern[..prefix_end]))?;
    let root = root.to_string_lossy();
    let suffix = &pattern[prefix_end..];
    let normalized_pattern = format!("{root}{suffix}");
    (normalized_pattern != pattern).then_some(normalized_pattern)
}

fn build_filesystem_policy(fs_policy: &FileSystemSandboxPolicy, cwd: &Path) -> String {
    match fs_policy.kind {
        FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
            "; allow full filesystem access\n(allow file-read* file-write*)".to_string()
        }
        FileSystemSandboxKind::Restricted => {
            let mut rules = Vec::new();
            rules.push("; filesystem access rules".to_string());

            if fs_policy.has_full_disk_read_access() {
                rules.push("(allow file-read* (subpath \"/\"))".to_string());
            } else {
                let readable_paths: Vec<PathBuf> = fs_policy
                    .entries
                    .iter()
                    .filter(|e| e.access.can_read() && !e.access.can_write())
                    .filter_map(|e| match &e.path {
                        FileSystemPath::Path { path } => normalize_path_for_sandbox(path),
                        _ => None,
                    })
                    .collect();

                if !readable_paths.is_empty() {
                    let roots: Vec<SeatbeltAccessRoot> = readable_paths
                        .iter()
                        .map(SeatbeltAccessRoot::simple)
                        .collect();
                    let (_params, policy) =
                        build_seatbelt_access_policy("file-read*", "READABLE_ROOT", &roots);
                    rules.push(policy);
                }

                for sys_path in &[
                    "/usr/lib",
                    "/usr/share",
                    "/usr/bin",
                    "/bin",
                    "/dev",
                    "/private/var/tmp",
                    "/Library/Developer",
                ] {
                    rules.push(format!("(allow file-read* (subpath \"{sys_path}\"))"));
                }
            }

            let rich_writable_roots = fs_policy.get_rich_writable_roots_with_cwd(cwd);
            if !rich_writable_roots.is_empty() {
                let roots: Vec<SeatbeltAccessRoot> = rich_writable_roots
                    .into_iter()
                    .filter_map(|wr| {
                        let root = normalize_path_for_sandbox(&wr.root)?;
                        let excluded_subpaths = wr
                            .read_only_subpaths
                            .iter()
                            .filter_map(|sp| normalize_path_for_sandbox(sp))
                            .collect();
                        Some(SeatbeltAccessRoot {
                            root,
                            excluded_subpaths,
                            protected_metadata_names: wr.protected_metadata_names,
                        })
                    })
                    .collect();
                if !roots.is_empty() {
                    let (_params, policy) = build_seatbelt_access_policy(
                        "file-read* file-write*",
                        "WRITABLE_ROOT",
                        &roots,
                    );
                    rules.push(policy);
                }
            }

            rules.push("(allow file-write* (literal \"/dev/null\"))".to_string());
            rules.push("(allow file-write* (literal \"/dev/tty\"))".to_string());

            rules.join("\n")
        }
    }
}

/// Policy for Unix domain socket access within the sandbox.
#[derive(Debug, Clone)]
pub enum UnixDomainSocketPolicy {
    /// Allow all AF_UNIX socket operations.
    AllowAll,
    /// Only allow specific socket paths.
    Restricted { allowed: Vec<PathBuf> },
}

/// Inputs for generating dynamic network policy in a proxy-aware Seatbelt sandbox.
///
/// Aligned with Codex's `ProxyPolicyInputs` — carries loopback port allowlists,
/// proxy configuration state, local binding permission, and Unix socket policy.
#[derive(Debug, Clone)]
pub struct ProxyPolicyInputs {
    /// Loopback TCP ports to whitelist (e.g. proxy bridge ports).
    pub ports: Vec<u16>,
    /// Whether a proxy is configured (affects DNS/connect rules).
    pub has_proxy_config: bool,
    /// Allow binding to local ports (e.g. for dev servers).
    pub allow_local_binding: bool,
    /// Unix domain socket access policy.
    pub unix_socket_policy: UnixDomainSocketPolicy,
}

impl Default for ProxyPolicyInputs {
    fn default() -> Self {
        Self {
            ports: Vec::new(),
            has_proxy_config: false,
            allow_local_binding: false,
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
        }
    }
}

impl ProxyPolicyInputs {
    /// Construct from environment variables using the network-proxy crate.
    ///
    /// Reads `http_proxy`/`https_proxy`/`all_proxy` env vars and extracts
    /// loopback ports for Seatbelt allowlisting.
    pub fn from_env() -> Self {
        let (http_port, socks_port) = xiaolin_network_proxy::proxy_loopback_ports_from_env();
        let has_proxy = xiaolin_network_proxy::has_proxy_url_env_vars();

        let mut ports = Vec::new();
        if let Some(p) = http_port {
            ports.push(p);
        }
        if let Some(p) = socks_port {
            ports.push(p);
        }

        Self {
            ports,
            has_proxy_config: has_proxy,
            allow_local_binding: std::env::var(xiaolin_network_proxy::ALLOW_LOCAL_BINDING_ENV_KEY)
                .is_ok(),
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
        }
    }
}

/// Legacy alias for backward compatibility.
pub type SeatbeltNetworkOptions = ProxyPolicyInputs;

/// Generate Seatbelt policy rules for Unix domain socket access.
///
/// - `AllowAll`: allows `system-socket AF_UNIX`, `network-bind` and `network-outbound`
///   for all unix sockets.
/// - `Restricted`: allows per-path `subpath` rules for bind and outbound.
fn unix_socket_policy(proxy: &ProxyPolicyInputs) -> String {
    let has_access = matches!(proxy.unix_socket_policy, UnixDomainSocketPolicy::AllowAll)
        || matches!(
            &proxy.unix_socket_policy,
            UnixDomainSocketPolicy::Restricted { allowed } if !allowed.is_empty()
        );
    if !has_access {
        return String::new();
    }

    let mut policy = String::new();
    policy.push_str("(allow system-socket (socket-domain AF_UNIX))\n");

    if matches!(proxy.unix_socket_policy, UnixDomainSocketPolicy::AllowAll) {
        policy.push_str("(allow network-bind (local unix-socket))\n");
        policy.push_str("(allow network-outbound (remote unix-socket))\n");
        return policy;
    }

    if let UnixDomainSocketPolicy::Restricted { allowed } = &proxy.unix_socket_policy {
        for path in allowed {
            let display = path.display();
            let _ = writeln!(
                policy,
                "(allow network-bind (local unix-socket (subpath \"{display}\")))"
            );
            let _ = writeln!(
                policy,
                "(allow network-outbound (remote unix-socket (subpath \"{display}\")))"
            );
        }
    }

    policy
}

/// Generate dynamic Seatbelt network policy based on proxy and socket options.
///
/// When a proxy is configured or ports are specified, the policy generates
/// restricted rules:
/// 1. Optional local binding (`allow_local_binding`).
/// 2. DNS pass-through when local binding + proxy ports are both present.
/// 3. Per-port loopback outbound rules.
/// 4. Unix domain socket policy.
///
/// Without proxy or ports, and with an enabled base network policy, generates
/// full outbound/inbound permissions. Otherwise fails closed (empty string).
pub fn dynamic_network_policy(
    base_network: NetworkSandboxPolicy,
    proxy: &ProxyPolicyInputs,
) -> String {
    let has_some_unix_socket_access = match &proxy.unix_socket_policy {
        UnixDomainSocketPolicy::AllowAll => true,
        UnixDomainSocketPolicy::Restricted { allowed } => !allowed.is_empty(),
    };

    let network_enabled = base_network.is_enabled();

    let should_use_restricted = !proxy.ports.is_empty()
        || proxy.has_proxy_config
        || (!network_enabled && has_some_unix_socket_access);

    if should_use_restricted {
        let mut policy = String::new();

        if proxy.allow_local_binding {
            policy.push_str("; allow local binding and loopback traffic\n");
            policy.push_str("(allow network-bind (local ip \"*:*\"))\n");
            policy.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
            policy.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
        }

        if proxy.allow_local_binding && !proxy.ports.is_empty() {
            policy.push_str("; allow DNS lookups while traffic remains proxy-routed\n");
            policy.push_str("(allow network-outbound (remote ip \"*:53\"))\n");
        }

        for port in &proxy.ports {
            let _ = writeln!(
                policy,
                "(allow network-outbound (remote ip \"localhost:{port}\"))"
            );
        }

        let socket_policy = unix_socket_policy(proxy);
        if !socket_policy.is_empty() {
            policy.push_str("; allow unix domain sockets for local IPC\n");
            policy.push_str(&socket_policy);
        }

        return format!("{policy}{SEATBELT_NETWORK_POLICY}");
    }

    if network_enabled {
        let mut policy = String::from("(allow network-outbound)\n(allow network-inbound)\n");
        let socket_policy = unix_socket_policy(proxy);
        if !socket_policy.is_empty() {
            policy.push_str("; allow unix domain sockets for local IPC\n");
            policy.push_str(&socket_policy);
        }
        format!("{policy}{SEATBELT_NETWORK_POLICY}")
    } else {
        String::new()
    }
}

/// Parameters for creating a proxy-aware Seatbelt sandbox command.
#[derive(Debug, Clone)]
pub struct CreateSeatbeltCommandArgsParams<'a> {
    pub command: &'a str,
    pub shell: &'a str,
    pub fs_policy: &'a FileSystemSandboxPolicy,
    pub net_policy: NetworkSandboxPolicy,
    /// Working directory for sandbox policy resolution.
    pub cwd: &'a Path,
    /// If true, only loopback network is allowed (managed network mode).
    pub enforce_managed_network: bool,
    /// Network proxy configuration, if available.
    pub network_proxy: Option<ProxyPolicyInputs>,
    /// Additional Unix socket paths to whitelist.
    pub extra_allow_unix_sockets: Vec<String>,
}

/// Transform a shell command into a Seatbelt-sandboxed command with proxy awareness.
///
/// When `enforce_managed_network` is true and a proxy is configured, the sandbox
/// only allows loopback connections to the proxy ports. When `enforce_managed_network`
/// is false, the base `net_policy` controls network access.
pub fn transform_with_proxy(params: &CreateSeatbeltCommandArgsParams<'_>) -> SandboxedCommand {
    let policy = build_seatbelt_policy_with_proxy(
        params.fs_policy,
        params.net_policy,
        params.cwd,
        params.enforce_managed_network,
        params.network_proxy.as_ref(),
        &params.extra_allow_unix_sockets,
    );

    let mut env = HashMap::new();
    env.insert("XIAOLIN_SANDBOXED".to_string(), "1".to_string());

    SandboxedCommand {
        program: SEATBELT_EXECUTABLE.to_string(),
        args: vec![
            "-p".to_string(),
            policy,
            "--".to_string(),
            params.shell.to_string(),
            "-c".to_string(),
            params.command.to_string(),
        ],
        working_dir: Some(params.cwd.to_path_buf()),
        env,
        env_remove: Vec::new(),
        sandbox_type: SandboxType::Seatbelt,
        linux_sandbox: None,
    }
}

fn build_seatbelt_policy_with_proxy(
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    cwd: &Path,
    enforce_managed_network: bool,
    network_proxy: Option<&ProxyPolicyInputs>,
    extra_unix_sockets: &[String],
) -> String {
    let mut sections = vec![SEATBELT_BASE_POLICY.to_string()];

    sections.push(build_filesystem_policy(fs_policy, cwd));

    if fs_policy.kind == FileSystemSandboxKind::Restricted {
        let deny_globs: Vec<String> = fs_policy
            .entries
            .iter()
            .filter(|e| e.access == FileSystemAccessMode::None)
            .filter_map(|e| match &e.path {
                FileSystemPath::GlobPattern { pattern } => Some(pattern.clone()),
                _ => None,
            })
            .collect();
        if !deny_globs.is_empty() {
            sections.push(build_seatbelt_unreadable_glob_policy(&deny_globs));
        }

        let has_readable = fs_policy.entries.iter().any(|e| {
            e.access.can_read()
                && !e.access.can_write()
                && matches!(&e.path, FileSystemPath::Path { .. })
        });
        if has_readable {
            sections.push(SEATBELT_PLATFORM_DEFAULTS.to_string());
        }
    }

    // Build network policy with proxy awareness
    if enforce_managed_network {
        let proxy = network_proxy.cloned().unwrap_or_else(|| {
            let mut p = ProxyPolicyInputs::default();
            if !extra_unix_sockets.is_empty() {
                p.unix_socket_policy = UnixDomainSocketPolicy::Restricted {
                    allowed: extra_unix_sockets.iter().map(PathBuf::from).collect(),
                };
            }
            p
        });

        let mut merged_proxy = proxy;
        if !extra_unix_sockets.is_empty() {
            match &mut merged_proxy.unix_socket_policy {
                UnixDomainSocketPolicy::AllowAll => {}
                UnixDomainSocketPolicy::Restricted { allowed } => {
                    for sock in extra_unix_sockets {
                        let p = PathBuf::from(sock);
                        if !allowed.contains(&p) {
                            allowed.push(p);
                        }
                    }
                }
            }
        }

        sections.push(dynamic_network_policy(net_policy, &merged_proxy));
    } else {
        sections.push(build_network_policy(net_policy));
    }

    sections.join("\n")
}

fn build_network_policy(net_policy: NetworkSandboxPolicy) -> String {
    if net_policy.is_enabled() {
        let mut policy = String::from(
            "; allow full network access\n(allow network-outbound)\n(allow network-inbound)\n",
        );
        policy.push_str(SEATBELT_NETWORK_POLICY);
        policy
    } else {
        "; network access denied by default policy".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::path::AbsolutePathBuf;
    use xiaolin_security::FileSystemSandboxEntry;

    fn test_cwd() -> PathBuf {
        PathBuf::from("/tmp/test")
    }

    fn unrestricted_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::unrestricted()
    }

    fn restricted_fs(
        writable: &[&str],
        readable: &[&str],
        deny_globs: &[&str],
    ) -> FileSystemSandboxPolicy {
        let mut entries = Vec::new();
        for p in readable {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path(*p).unwrap(),
                },
                access: FileSystemAccessMode::Read,
            });
        }
        for p in writable {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path(*p).unwrap(),
                },
                access: FileSystemAccessMode::Write,
            });
        }
        for g in deny_globs {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: g.to_string(),
                },
                access: FileSystemAccessMode::None,
            });
        }
        FileSystemSandboxPolicy::restricted(entries)
    }

    #[test]
    fn seatbelt_transform_uses_absolute_path() {
        let cmd = transform(
            "echo hello",
            "bash",
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            &test_cwd(),
        );
        assert_eq!(cmd.program, "/usr/bin/sandbox-exec");
        assert_eq!(cmd.sandbox_type, SandboxType::Seatbelt);
        assert!(cmd.args.contains(&"-p".to_string()));
        assert!(cmd.args.contains(&"--".to_string()));
        assert!(cmd.args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn seatbelt_policy_uses_base_template() {
        let policy = build_seatbelt_policy(
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            &test_cwd(),
        );
        assert!(policy.contains("(version 1)"));
        assert!(policy.contains("(deny default)"));
        assert!(policy.contains("(allow process-exec)"));
        assert!(policy.contains("(allow process-fork)"));
    }

    #[test]
    fn seatbelt_policy_unrestricted_allows_all() {
        let policy = build_seatbelt_policy(
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            &test_cwd(),
        );
        assert!(policy.contains("(allow file-read* file-write*)"));
        assert!(policy.contains("(allow network-outbound)"));
        assert!(policy.contains(SEATBELT_NETWORK_POLICY));
    }

    #[test]
    fn seatbelt_policy_restricted_denies_by_default() {
        let fs = restricted_fs(&["/tmp"], &["/usr"], &[]);
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Restricted, &test_cwd());
        assert!(policy.contains("(deny default)"));
        assert!(policy.contains("file-read* file-write*"));
        assert!(policy.contains("file-read*"));
        assert!(!policy.contains("(allow network-outbound)"));
        assert!(policy.contains("com.apple.system.opendirectoryd.libinfo"));
    }

    #[test]
    fn build_access_policy_simple_roots() {
        let roots = vec![
            SeatbeltAccessRoot::simple("/home/user"),
            SeatbeltAccessRoot::simple("/tmp"),
        ];
        let (params, policy) = build_seatbelt_access_policy("file-write*", "WRITABLE_ROOT", &roots);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0, "WRITABLE_ROOT_0");
        assert_eq!(params[1].0, "WRITABLE_ROOT_1");
        assert!(policy.contains("/home/user"));
        assert!(policy.contains("/tmp"));
    }

    #[test]
    fn build_access_policy_with_exclusions() {
        let root = SeatbeltAccessRoot {
            root: PathBuf::from("/home/user"),
            excluded_subpaths: vec![PathBuf::from("/home/user/.ssh")],
            protected_metadata_names: vec![".env".to_string()],
        };
        let (params, policy) = build_seatbelt_access_policy("file-write*", "ROOT", &[root]);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0, "ROOT");
        assert!(policy.contains("require-all"));
        assert!(policy.contains("require-not"));
        assert!(policy.contains("/home/user/.ssh"));
        assert!(policy.contains(".env"));
    }

    #[test]
    fn protected_metadata_regex() {
        let regex = seatbelt_protected_metadata_name_regex(".env");
        assert_eq!(regex, r".*/\.env$");
        let regex = seatbelt_protected_metadata_name_regex("file.txt");
        assert_eq!(regex, r".*/file\.txt$");
    }

    #[test]
    fn unreadable_glob_star() {
        let regex = seatbelt_regex_for_unreadable_glob("*.log");
        assert_eq!(regex, r"[^/]*\.log");
    }

    #[test]
    fn unreadable_glob_double_star() {
        let regex = seatbelt_regex_for_unreadable_glob("**/.env");
        assert_eq!(regex, r"(.*/)?\.env");
    }

    #[test]
    fn unreadable_glob_question_mark() {
        let regex = seatbelt_regex_for_unreadable_glob("file?.txt");
        assert_eq!(regex, r"file[^/]\.txt");
    }

    #[test]
    fn unreadable_glob_no_metachar() {
        let regex = seatbelt_regex_for_unreadable_glob(".env");
        assert_eq!(regex, r"(^|.*/)\.env(/.*)?$");
    }

    #[test]
    fn build_unreadable_glob_policy_output() {
        let policy = build_seatbelt_unreadable_glob_policy(&["**/.env".into()]);
        assert!(policy.contains("deny file-read*"));
        assert!(policy.contains("deny file-write-unlink"));
        assert!(policy.contains("(.*/)?"));
    }

    #[test]
    fn build_unreadable_glob_policy_empty() {
        let policy = build_seatbelt_unreadable_glob_policy(&[]);
        assert!(policy.is_empty());
    }

    #[test]
    fn unix_socket_policy_allow_all() {
        let proxy = ProxyPolicyInputs {
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
            ..Default::default()
        };
        let policy = unix_socket_policy(&proxy);
        assert!(policy.contains("system-socket"));
        assert!(policy.contains("AF_UNIX"));
        assert!(policy.contains("network-bind"));
        assert!(policy.contains("network-outbound"));
        assert!(policy.contains("remote unix-socket"));
    }

    #[test]
    fn unix_socket_policy_restricted() {
        let proxy = ProxyPolicyInputs {
            unix_socket_policy: UnixDomainSocketPolicy::Restricted {
                allowed: vec![PathBuf::from("/var/run/docker.sock")],
            },
            ..Default::default()
        };
        let policy = unix_socket_policy(&proxy);
        assert!(policy.contains("subpath"));
        assert!(policy.contains("docker.sock"));
    }

    #[test]
    fn unix_socket_policy_restricted_empty() {
        let proxy = ProxyPolicyInputs {
            unix_socket_policy: UnixDomainSocketPolicy::Restricted { allowed: vec![] },
            ..Default::default()
        };
        let policy = unix_socket_policy(&proxy);
        assert!(policy.is_empty());
    }

    #[test]
    fn dynamic_network_policy_with_proxy() {
        let policy = dynamic_network_policy(
            NetworkSandboxPolicy::Restricted,
            &ProxyPolicyInputs {
                ports: vec![8080, 9090],
                has_proxy_config: true,
                allow_local_binding: false,
                unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
            },
        );
        assert!(policy.contains("localhost:8080"));
        assert!(policy.contains("localhost:9090"));
        assert!(policy.contains("AF_UNIX"));
    }

    #[test]
    fn dynamic_network_policy_with_local_binding() {
        let proxy = ProxyPolicyInputs {
            ports: vec![3128],
            has_proxy_config: true,
            allow_local_binding: true,
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
        };
        let policy = dynamic_network_policy(NetworkSandboxPolicy::Enabled, &proxy);
        assert!(policy.contains("network-bind"));
        assert!(policy.contains("*:*"));
        assert!(policy.contains("*:53"));
    }

    #[test]
    fn dynamic_network_policy_disabled_no_proxy_empty() {
        let policy = dynamic_network_policy(
            NetworkSandboxPolicy::Restricted,
            &ProxyPolicyInputs {
                ports: vec![],
                has_proxy_config: false,
                allow_local_binding: false,
                unix_socket_policy: UnixDomainSocketPolicy::Restricted { allowed: vec![] },
            },
        );
        assert!(policy.is_empty());
    }

    #[test]
    fn dynamic_network_policy_enabled_full_access() {
        let policy =
            dynamic_network_policy(NetworkSandboxPolicy::Enabled, &ProxyPolicyInputs::default());
        assert!(policy.contains("(allow network-outbound)"));
        assert!(policy.contains("(allow network-inbound)"));
    }

    #[test]
    fn seatbelt_policy_with_deny_globs() {
        let fs = restricted_fs(&[], &["/usr"], &["**/.env", "*.key"]);
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Restricted, &test_cwd());
        assert!(policy.contains("deny file-read*"));
        assert!(policy.contains(".env"));
        assert!(policy.contains(".key"));
    }

    #[test]
    fn seatbelt_policy_network_enabled() {
        let policy = build_seatbelt_policy(
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            &test_cwd(),
        );
        assert!(policy.contains("(allow network-outbound)"));
    }

    #[test]
    fn seatbelt_policy_network_restricted_denies() {
        let policy = build_seatbelt_policy(
            &unrestricted_fs(),
            NetworkSandboxPolicy::Restricted,
            &test_cwd(),
        );
        assert!(!policy.contains("(allow network-outbound)"));
        assert!(policy.contains("network access denied"));
    }

    #[test]
    fn normalize_path_accepts_absolute() {
        let result = normalize_path_for_sandbox(Path::new("/tmp"));
        assert!(result.is_some());
        assert!(result.unwrap().is_absolute());
    }

    #[test]
    fn normalize_path_rejects_relative() {
        let result = normalize_path_for_sandbox(Path::new("relative/path"));
        assert!(result.is_none());
    }

    #[test]
    fn normalize_path_falls_back_for_nonexistent() {
        let path = Path::new("/nonexistent/path/for/sandbox/test");
        let result = normalize_path_for_sandbox(path);
        assert_eq!(result, Some(path.to_path_buf()));
    }

    #[test]
    fn canonicalize_glob_no_metachar_returns_none_for_relative() {
        let result = canonicalize_glob_static_prefix_for_sandbox("relative/path");
        assert!(result.is_none());
    }

    #[test]
    fn canonicalize_glob_no_prefix_returns_none() {
        let result = canonicalize_glob_static_prefix_for_sandbox("*.log");
        assert!(result.is_none());
    }

    #[test]
    fn canonicalize_glob_with_absolute_prefix() {
        let result =
            canonicalize_glob_static_prefix_for_sandbox("/nonexistent/sandbox/test/dir/*.log");
        // The prefix /nonexistent/sandbox/test/dir doesn't exist, so canonicalize
        // falls back to the original path and the result is None (same as input).
        assert!(result.is_none());
    }

    #[test]
    fn canonicalize_glob_preserves_suffix() {
        let result = canonicalize_glob_static_prefix_for_sandbox("/tmp/**/.env");
        // /tmp exists, so canonicalize may resolve it. The suffix should be preserved.
        if let Some(pattern) = result {
            assert!(pattern.ends_with("/**/.env"));
        }
    }

    #[test]
    fn unreadable_glob_policy_deduplicates_canonical() {
        let policy = build_seatbelt_unreadable_glob_policy(&["/tmp/**/.env".into()]);
        assert!(policy.contains("deny file-read*"));
        assert!(policy.contains("deny file-write-unlink"));
    }

    // ── Phase 6.2: Extended test coverage ──

    #[test]
    fn transform_with_proxy_basic() {
        let params = CreateSeatbeltCommandArgsParams {
            command: "echo hello",
            shell: "bash",
            fs_policy: &unrestricted_fs(),
            net_policy: NetworkSandboxPolicy::Enabled,
            cwd: &test_cwd(),
            enforce_managed_network: false,
            network_proxy: None,
            extra_allow_unix_sockets: vec![],
        };
        let cmd = transform_with_proxy(&params);
        assert_eq!(cmd.program, "/usr/bin/sandbox-exec");
        assert!(cmd.args.contains(&"-p".to_string()));
        assert!(cmd.args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn transform_with_proxy_managed_network() {
        let proxy = ProxyPolicyInputs {
            ports: vec![8080],
            has_proxy_config: true,
            allow_local_binding: false,
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
        };
        let params = CreateSeatbeltCommandArgsParams {
            command: "curl http://example.com",
            shell: "bash",
            fs_policy: &unrestricted_fs(),
            net_policy: NetworkSandboxPolicy::Restricted,
            cwd: &test_cwd(),
            enforce_managed_network: true,
            network_proxy: Some(proxy),
            extra_allow_unix_sockets: vec![],
        };
        let cmd = transform_with_proxy(&params);
        let policy = &cmd.args[1];
        assert!(policy.contains("localhost:8080"));
    }

    #[test]
    fn transform_with_proxy_extra_unix_sockets() {
        let params = CreateSeatbeltCommandArgsParams {
            command: "docker ps",
            shell: "bash",
            fs_policy: &unrestricted_fs(),
            net_policy: NetworkSandboxPolicy::Restricted,
            cwd: &test_cwd(),
            enforce_managed_network: true,
            network_proxy: Some(ProxyPolicyInputs {
                ports: vec![],
                has_proxy_config: false,
                allow_local_binding: false,
                unix_socket_policy: UnixDomainSocketPolicy::Restricted { allowed: vec![] },
            }),
            extra_allow_unix_sockets: vec!["/var/run/docker.sock".into()],
        };
        let cmd = transform_with_proxy(&params);
        let policy = &cmd.args[1];
        assert!(policy.contains("docker.sock"));
    }

    #[test]
    fn transform_with_proxy_no_managed_network_uses_base() {
        let params = CreateSeatbeltCommandArgsParams {
            command: "ls",
            shell: "bash",
            fs_policy: &unrestricted_fs(),
            net_policy: NetworkSandboxPolicy::Enabled,
            cwd: &test_cwd(),
            enforce_managed_network: false,
            network_proxy: None,
            extra_allow_unix_sockets: vec![],
        };
        let cmd = transform_with_proxy(&params);
        let policy = &cmd.args[1];
        assert!(policy.contains("(allow network-outbound)"));
    }

    #[test]
    fn transform_with_proxy_restricted_fs_and_managed_net() {
        let fs = restricted_fs(&["/tmp"], &["/usr"], &["**/.env"]);
        let proxy = ProxyPolicyInputs {
            ports: vec![3128, 1080],
            has_proxy_config: true,
            allow_local_binding: true,
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
        };
        let params = CreateSeatbeltCommandArgsParams {
            command: "npm install",
            shell: "bash",
            fs_policy: &fs,
            net_policy: NetworkSandboxPolicy::Restricted,
            cwd: &test_cwd(),
            enforce_managed_network: true,
            network_proxy: Some(proxy),
            extra_allow_unix_sockets: vec![],
        };
        let cmd = transform_with_proxy(&params);
        let policy = &cmd.args[1];
        assert!(policy.contains("/tmp"));
        assert!(policy.contains("deny file-read*"));
        assert!(policy.contains("localhost:3128"));
        assert!(policy.contains("localhost:1080"));
        assert!(policy.contains("network-bind"));
    }

    #[test]
    fn dynamic_network_disabled_unix_socket_only() {
        let proxy = ProxyPolicyInputs {
            ports: vec![],
            has_proxy_config: false,
            allow_local_binding: false,
            unix_socket_policy: UnixDomainSocketPolicy::Restricted {
                allowed: vec![PathBuf::from("/tmp/myapp.sock")],
            },
        };
        let policy = dynamic_network_policy(NetworkSandboxPolicy::Restricted, &proxy);
        assert!(policy.contains("AF_UNIX"));
        assert!(policy.contains("myapp.sock"));
    }

    #[test]
    fn build_access_policy_single_root_no_index() {
        let roots = vec![SeatbeltAccessRoot::simple("/home/user")];
        let (params, policy) = build_seatbelt_access_policy("file-read*", "ROOT", &roots);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0, "ROOT");
        assert!(policy.contains("/home/user"));
    }

    #[test]
    fn seatbelt_env_contains_sandbox_marker() {
        let cmd = transform(
            "echo",
            "sh",
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            &test_cwd(),
        );
        assert_eq!(cmd.env.get("XIAOLIN_SANDBOXED"), Some(&"1".to_string()));
    }

    #[test]
    fn restricted_fs_writable_only() {
        let fs = restricted_fs(&["/home/user/project"], &[], &[]);
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Restricted, &test_cwd());
        assert!(policy.contains("/home/user/project"));
        assert!(policy.contains("file-read* file-write*"));
    }

    #[test]
    fn restricted_fs_readable_only() {
        let fs = restricted_fs(&[], &["/usr/local"], &[]);
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Restricted, &test_cwd());
        assert!(policy.contains("/usr/local"));
        assert!(policy.contains("file-read*"));
    }

    #[test]
    fn seatbelt_policy_allows_dev_null() {
        let fs = restricted_fs(&["/tmp"], &["/usr"], &[]);
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Restricted, &test_cwd());
        assert!(policy.contains("/dev/null"));
        assert!(policy.contains("/dev/tty"));
    }

    #[test]
    fn seatbelt_policy_allows_system_paths() {
        let fs = restricted_fs(&[], &["/usr"], &[]);
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Restricted, &test_cwd());
        assert!(policy.contains("/usr/lib"));
        assert!(policy.contains("/usr/share"));
        assert!(policy.contains("/usr/bin"));
        assert!(policy.contains("/bin"));
        assert!(policy.contains("/dev"));
    }

    #[test]
    fn multiple_deny_globs_produce_separate_rules() {
        let globs = vec!["**/.env".into(), "**/.ssh".into(), "*.key".into()];
        let policy = build_seatbelt_unreadable_glob_policy(&globs);
        let deny_count = policy.matches("deny file-read*").count();
        assert!(deny_count >= 3);
    }

    #[test]
    fn transform_with_proxy_env_marker() {
        let params = CreateSeatbeltCommandArgsParams {
            command: "test",
            shell: "sh",
            fs_policy: &unrestricted_fs(),
            net_policy: NetworkSandboxPolicy::Enabled,
            cwd: &test_cwd(),
            enforce_managed_network: false,
            network_proxy: None,
            extra_allow_unix_sockets: vec![],
        };
        let cmd = transform_with_proxy(&params);
        assert_eq!(cmd.env.get("XIAOLIN_SANDBOXED"), Some(&"1".to_string()));
        assert_eq!(cmd.sandbox_type, SandboxType::Seatbelt);
    }

    #[test]
    fn dynamic_network_proxy_with_multiple_ports_dns() {
        let proxy = ProxyPolicyInputs {
            ports: vec![8080, 1080, 3128],
            has_proxy_config: true,
            allow_local_binding: true,
            unix_socket_policy: UnixDomainSocketPolicy::AllowAll,
        };
        let policy = dynamic_network_policy(NetworkSandboxPolicy::Restricted, &proxy);
        assert!(policy.contains("localhost:8080"));
        assert!(policy.contains("localhost:1080"));
        assert!(policy.contains("localhost:3128"));
        assert!(policy.contains("*:53"));
    }

    #[test]
    fn unreadable_glob_bracket_class() {
        let regex = seatbelt_regex_for_unreadable_glob("[abc].txt");
        assert_eq!(regex, r"[abc]\.txt");
    }

    #[test]
    fn glob_double_star_trailing() {
        let regex = seatbelt_regex_for_unreadable_glob("/home/user/**");
        assert_eq!(regex, r"/home/user/.*");
    }

    #[test]
    fn full_disk_read_generates_subpath_root() {
        let fs = FileSystemSandboxPolicy::default(); // has Root read
        assert!(fs.has_full_disk_read_access());
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Enabled, &test_cwd());
        assert!(policy.contains("(allow file-read* (subpath \"/\"))"));
        // Should NOT contain hardcoded system paths since subpath "/" covers everything
        assert!(!policy.contains("(allow file-read* (subpath \"/usr/lib\"))"));
    }

    #[test]
    fn non_full_disk_read_uses_specific_paths() {
        let fs = restricted_fs(&["/tmp"], &["/usr/local"], &[]);
        assert!(!fs.has_full_disk_read_access());
        let policy = build_seatbelt_policy(&fs, NetworkSandboxPolicy::Enabled, &test_cwd());
        assert!(!policy.contains("(allow file-read* (subpath \"/\"))"));
        assert!(policy.contains("/usr/local"));
        assert!(policy.contains("/usr/lib")); // hardcoded fallback
    }

    #[test]
    fn seatbelt_transform_sets_working_dir() {
        let cwd = PathBuf::from("/home/user/project");
        let cmd = transform(
            "echo hi",
            "sh",
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            &cwd,
        );
        assert_eq!(cmd.working_dir, Some(cwd));
    }

    #[test]
    fn seatbelt_transform_with_proxy_sets_working_dir() {
        let cwd = test_cwd();
        let params = CreateSeatbeltCommandArgsParams {
            command: "ls",
            shell: "sh",
            fs_policy: &unrestricted_fs(),
            net_policy: NetworkSandboxPolicy::Enabled,
            cwd: &cwd,
            enforce_managed_network: false,
            network_proxy: None,
            extra_allow_unix_sockets: vec![],
        };
        let cmd = transform_with_proxy(&params);
        assert_eq!(cmd.working_dir, Some(cwd));
    }
}
