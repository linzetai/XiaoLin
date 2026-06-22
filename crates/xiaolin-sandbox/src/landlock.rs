use std::collections::HashMap;
use std::path::{Path, PathBuf};

use xiaolin_security::{
    FileSystemAccessMode, FileSystemPath, FileSystemSandboxKind, FileSystemSandboxPolicy,
    NetworkSandboxPolicy,
};

use crate::{LinuxSandboxSetup, SandboxTransformError, SandboxType, SandboxedCommand};

/// Check if Landlock is available on this Linux kernel.
///
/// Landlock requires Linux 5.13+ and `prctl(PR_SET_NO_NEW_PRIVS)` support.
#[cfg(target_os = "linux")]
pub fn is_available() -> bool {
    use std::fs;
    let Ok(version) = fs::read_to_string("/proc/sys/kernel/osrelease") else {
        return false;
    };
    let parts: Vec<&str> = version.trim().split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let major: u32 = parts[0].parse().unwrap_or(0);
    let minor: u32 = parts[1]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    major > 5 || (major == 5 && minor >= 13)
}

#[cfg(not(target_os = "linux"))]
pub fn is_available() -> bool {
    false
}

/// Transform a shell command into a Landlock-sandboxed command.
///
/// The returned `SandboxedCommand` carries a `LinuxSandboxSetup` that is
/// applied via `pre_exec` in the child process, directly calling Landlock
/// and seccomp kernel APIs.
pub fn transform(
    command: &str,
    shell: &str,
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    cwd: &Path,
) -> SandboxedCommand {
    let mut env = HashMap::new();
    env.insert("XIAOLIN_SANDBOXED".to_string(), "1".to_string());

    SandboxedCommand {
        program: shell.to_string(),
        args: vec!["-c".to_string(), command.to_string()],
        working_dir: Some(cwd.to_path_buf()),
        env,
        env_remove: build_env_remove_list(fs_policy, net_policy),
        sandbox_type: SandboxType::Landlock,
        linux_sandbox: build_linux_sandbox_setup(fs_policy, net_policy, cwd),
    }
}

/// Basename used when XiaoLin self-invokes as the Linux sandbox helper.
pub const XIAOLIN_LINUX_SANDBOX_ARG0: &str = "xiaolin-linux-sandbox";

/// Attempt to auto-discover the `xiaolin-linux-sandbox` binary.
///
/// Search order:
/// 1. Sibling of the current executable (same directory).
/// 2. On `PATH` via `which`.
///
/// Returns `None` if the binary is not found or not on Linux.
#[cfg(target_os = "linux")]
pub fn discover_linux_sandbox_exe() -> Option<PathBuf> {
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join(XIAOLIN_LINUX_SANDBOX_ARG0);
            if candidate.is_file() {
                tracing::debug!(path = %candidate.display(), "found linux-sandbox binary next to current exe");
                return Some(candidate);
            }
        }
    }

    if let Ok(found) = which::which(XIAOLIN_LINUX_SANDBOX_ARG0) {
        tracing::debug!(path = %found.display(), "found linux-sandbox binary on PATH");
        return Some(found);
    }

    tracing::debug!("linux-sandbox binary not found");
    None
}

#[cfg(not(target_os = "linux"))]
pub fn discover_linux_sandbox_exe() -> Option<PathBuf> {
    None
}

/// Transform a command into one that delegates to an external sandbox binary.
///
/// The sandbox binary receives the full `FileSystemSandboxPolicy` as
/// serialized JSON so it can apply bubblewrap + seccomp enforcement in
/// its own process. This avoids needing `pre_exec` Landlock in the parent.
pub fn transform_external(
    command: &str,
    shell: &str,
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    sandbox_exe: &std::path::Path,
    sandbox_policy_cwd: &std::path::Path,
    enforce_managed_network: bool,
) -> Result<SandboxedCommand, SandboxTransformError> {
    let policy_json = serde_json::to_string(fs_policy).map_err(|err| {
        tracing::error!(error = %err, "failed to serialize filesystem policy for external sandbox");
        SandboxTransformError::PolicySerializationFailed(err.to_string())
    })?;
    let cwd_str = sandbox_policy_cwd
        .to_str()
        .unwrap_or_else(|| panic!("sandbox policy cwd must be valid UTF-8"))
        .to_string();

    let mut sandbox_args: Vec<String> = vec![
        "--sandbox-policy-cwd".into(),
        cwd_str,
        "--fs-policy".into(),
        policy_json,
    ];

    if !net_policy.is_enabled() || enforce_managed_network {
        sandbox_args.push("--allow-network-for-proxy".into());
    }

    sandbox_args.push("--".into());
    sandbox_args.push(shell.into());
    sandbox_args.push("-c".into());
    sandbox_args.push(command.into());

    let mut env = HashMap::new();
    env.insert("XIAOLIN_SANDBOXED".to_string(), "1".to_string());

    Ok(SandboxedCommand {
        program: sandbox_exe.to_string_lossy().into_owned(),
        args: sandbox_args,
        working_dir: Some(sandbox_policy_cwd.to_path_buf()),
        env,
        env_remove: build_env_remove_list(fs_policy, net_policy),
        sandbox_type: SandboxType::ExternalBinary,
        linux_sandbox: None,
    })
}

fn build_linux_sandbox_setup(
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    cwd: &Path,
) -> Option<LinuxSandboxSetup> {
    use crate::NetworkSeccompMode;

    let writable_roots = match fs_policy.kind {
        FileSystemSandboxKind::Restricted => {
            let roots: Vec<PathBuf> = fs_policy
                .get_writable_roots_with_cwd(cwd)
                .into_iter()
                .map(|abs| abs.to_path_buf())
                .collect();
            Some(roots)
        }
        FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => None,
    };

    let network_seccomp = if net_policy.is_enabled() {
        None
    } else {
        Some(NetworkSeccompMode::Restricted)
    };

    if writable_roots.is_none() && network_seccomp.is_none() {
        return None;
    }

    Some(LinuxSandboxSetup {
        writable_roots,
        network_seccomp,
    })
}

fn build_env_remove_list(
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
) -> Vec<String> {
    let mut remove = Vec::new();
    let is_restricted =
        fs_policy.kind != FileSystemSandboxKind::Unrestricted || !net_policy.is_enabled();
    if is_restricted {
        remove.extend(["LD_PRELOAD".to_string(), "LD_LIBRARY_PATH".to_string()]);
    }
    remove
}

// ---------------------------------------------------------------------------
// Linux-specific: real Landlock + seccomp implementation
// ---------------------------------------------------------------------------

/// Apply Landlock filesystem rules and optionally seccomp network filtering
/// to the current process/thread.
///
/// Called from `pre_exec` in the child process after `fork()`, before `exec()`.
#[cfg(target_os = "linux")]
pub fn apply_sandbox_to_current_process(setup: &LinuxSandboxSetup) -> std::io::Result<()> {
    let needs_no_new_privs = setup.writable_roots.is_some() || setup.network_seccomp.is_some();
    if needs_no_new_privs {
        set_no_new_privs()?;
    }
    if let Some(writable_roots) = &setup.writable_roots {
        install_filesystem_landlock_rules(writable_roots)?;
    }
    if let Some(mode) = setup.network_seccomp {
        install_network_seccomp_filter(mode)?;
    }
    Ok(())
}

/// Enable `PR_SET_NO_NEW_PRIVS` so seccomp filters can be applied and
/// privilege escalation via setuid binaries is blocked.
#[cfg(target_os = "linux")]
fn set_no_new_privs() -> std::io::Result<()> {
    // SAFETY: prctl(PR_SET_NO_NEW_PRIVS, 1) is a well-defined Linux syscall
    // that prevents the process from gaining new privileges.
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Install Landlock filesystem rules: read-only access to "/" with read-write
/// access to `/dev/null` and the provided `writable_roots`.
#[cfg(target_os = "linux")]
fn install_filesystem_landlock_rules(writable_roots: &[PathBuf]) -> std::io::Result<()> {
    use landlock::{
        Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI,
    };

    let abi = ABI::V5;
    let access_rw = AccessFs::from_all(abi);
    let access_ro = AccessFs::from_read(abi);

    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(access_rw)
        .map_err(io_err)?
        .create()
        .map_err(io_err)?
        .add_rules(landlock::path_beneath_rules(&["/"], access_ro))
        .map_err(io_err)?
        .add_rules(landlock::path_beneath_rules(&["/dev/null"], access_rw))
        .map_err(io_err)?
        .set_no_new_privs(true);

    if !writable_roots.is_empty() {
        ruleset = ruleset
            .add_rules(landlock::path_beneath_rules(writable_roots, access_rw))
            .map_err(io_err)?;
    }

    let status = ruleset.restrict_self().map_err(io_err)?;
    if status.ruleset == landlock::RulesetStatus::NotEnforced {
        return Err(std::io::Error::other(
            "Landlock rules were not enforced by the kernel",
        ));
    }

    Ok(())
}

/// Install a seccomp BPF filter that restricts network syscalls.
///
/// - `Restricted`: blocks all network ops except AF_UNIX for local IPC.
/// - `ProxyRouted`: allows AF_INET/AF_INET6 (for local TCP proxy bridge),
///   blocks AF_UNIX new sockets, does not block connect/bind/listen.
#[cfg(target_os = "linux")]
fn install_network_seccomp_filter(mode: crate::NetworkSeccompMode) -> std::io::Result<()> {
    use std::collections::BTreeMap;

    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, TargetArch,
    };

    fn deny_syscall(rules: &mut BTreeMap<i64, Vec<SeccompRule>>, nr: i64) {
        rules.insert(nr, vec![]);
    }

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();

    deny_syscall(&mut rules, libc::SYS_ptrace);
    deny_syscall(&mut rules, libc::SYS_process_vm_readv);
    deny_syscall(&mut rules, libc::SYS_process_vm_writev);
    deny_syscall(&mut rules, libc::SYS_io_uring_setup);
    deny_syscall(&mut rules, libc::SYS_io_uring_enter);
    deny_syscall(&mut rules, libc::SYS_io_uring_register);

    match mode {
        crate::NetworkSeccompMode::Restricted => {
            deny_syscall(&mut rules, libc::SYS_connect);
            deny_syscall(&mut rules, libc::SYS_accept);
            deny_syscall(&mut rules, libc::SYS_accept4);
            deny_syscall(&mut rules, libc::SYS_bind);
            deny_syscall(&mut rules, libc::SYS_listen);
            deny_syscall(&mut rules, libc::SYS_getpeername);
            deny_syscall(&mut rules, libc::SYS_getsockname);
            deny_syscall(&mut rules, libc::SYS_shutdown);
            deny_syscall(&mut rules, libc::SYS_sendto);
            deny_syscall(&mut rules, libc::SYS_sendmmsg);
            deny_syscall(&mut rules, libc::SYS_recvmmsg);
            deny_syscall(&mut rules, libc::SYS_getsockopt);
            deny_syscall(&mut rules, libc::SYS_setsockopt);

            let unix_only_rule = SeccompRule::new(vec![SeccompCondition::new(
                0,
                SeccompCmpArgLen::Dword,
                SeccompCmpOp::Ne,
                libc::AF_UNIX as u64,
            )
            .map_err(io_err)?])
            .map_err(io_err)?;

            rules.insert(libc::SYS_socket, vec![unix_only_rule.clone()]);
            rules.insert(libc::SYS_socketpair, vec![unix_only_rule]);
        }
        crate::NetworkSeccompMode::ProxyRouted => {
            let deny_unix_socket = SeccompRule::new(vec![SeccompCondition::new(
                0,
                SeccompCmpArgLen::Dword,
                SeccompCmpOp::Eq,
                libc::AF_UNIX as u64,
            )
            .map_err(io_err)?])
            .map_err(io_err)?;

            let deny_non_inet_socket = SeccompRule::new(vec![
                SeccompCondition::new(
                    0,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Ne,
                    libc::AF_INET as u64,
                )
                .map_err(io_err)?,
                SeccompCondition::new(
                    0,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Ne,
                    libc::AF_INET6 as u64,
                )
                .map_err(io_err)?,
            ])
            .map_err(io_err)?;

            rules.insert(
                libc::SYS_socket,
                vec![deny_unix_socket, deny_non_inet_socket],
            );
            rules.insert(libc::SYS_socketpair, vec![]);
        }
    }

    let target_arch = if cfg!(target_arch = "x86_64") {
        TargetArch::x86_64
    } else if cfg!(target_arch = "aarch64") {
        TargetArch::aarch64
    } else {
        return Err(std::io::Error::other(
            "unsupported architecture for seccomp filter",
        ));
    };

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        target_arch,
    )
    .map_err(io_err)?;

    let prog: BpfProgram = filter.try_into().map_err(io_err)?;

    seccompiler::apply_filter(&prog).map_err(io_err)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// CLI args builder (for potential future helper binary)
// ---------------------------------------------------------------------------

/// Build Landlock CLI arguments from the rich policy types.
pub fn build_landlock_args(
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
) -> Vec<String> {
    let mut args = Vec::new();

    match fs_policy.kind {
        FileSystemSandboxKind::Restricted => {
            for entry in &fs_policy.entries {
                match (&entry.path, entry.access) {
                    (FileSystemPath::Path { path }, access) if access.can_write() => {
                        args.push("--writable".to_string());
                        args.push(path.display().to_string());
                    }
                    (FileSystemPath::Path { path }, access) if access.can_read() => {
                        args.push("--readable".to_string());
                        args.push(path.display().to_string());
                    }
                    (FileSystemPath::GlobPattern { pattern }, FileSystemAccessMode::None) => {
                        args.push("--deny-glob".to_string());
                        args.push(pattern.clone());
                    }
                    _ => {}
                }
            }
        }
        FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
            args.push("--fs-unrestricted".to_string());
        }
    }

    if net_policy.is_enabled() {
        args.push("--net-enabled".to_string());
    } else {
        args.push("--net-disabled".to_string());
    }

    args
}

/// Determine whether the Linux sandbox should allow network for proxy routing.
///
/// When managed network requirements are active, request proxy-only networking
/// from the sandbox helper. Without managed requirements, preserve existing behavior.
pub fn allow_network_for_proxy(enforce_managed_network: bool) -> bool {
    enforce_managed_network
}

/// Build CLI arguments for the Linux sandbox helper binary.
///
/// Serializes the `FileSystemSandboxPolicy` as JSON and constructs the argument vector:
/// `[--sandbox-policy-cwd, <cwd>, --command-cwd, <cmd_cwd>,
///   --fs-sandbox-policy, <json>, [--use-legacy-landlock],
///   [--allow-network-for-proxy], --, <command...>]`
pub fn create_linux_sandbox_command_args(
    command: Vec<String>,
    command_cwd: &Path,
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    sandbox_policy_cwd: &Path,
    use_legacy_landlock: bool,
    allow_network: bool,
) -> Vec<String> {
    let fs_json = serde_json::to_string(fs_policy)
        .unwrap_or_else(|err| panic!("failed to serialize fs sandbox policy: {err}"));
    let net_json = serde_json::to_string(&net_policy)
        .unwrap_or_else(|err| panic!("failed to serialize net sandbox policy: {err}"));
    let policy_cwd = sandbox_policy_cwd
        .to_str()
        .unwrap_or_else(|| panic!("sandbox policy cwd must be valid UTF-8"))
        .to_string();
    let cmd_cwd = command_cwd
        .to_str()
        .unwrap_or_else(|| panic!("command cwd must be valid UTF-8"))
        .to_string();

    let mut args: Vec<String> = vec![
        "--sandbox-policy-cwd".to_string(),
        policy_cwd,
        "--command-cwd".to_string(),
        cmd_cwd,
        "--fs-sandbox-policy".to_string(),
        fs_json,
        "--net-sandbox-policy".to_string(),
        net_json,
    ];
    if use_legacy_landlock {
        args.push("--use-legacy-landlock".to_string());
    }
    if allow_network {
        args.push("--allow-network-for-proxy".to_string());
    }
    args.push("--".to_string());
    args.extend(command);
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::path::AbsolutePathBuf;
    use xiaolin_security::{FileSystemSandboxEntry, FileSystemSpecialPath};

    fn test_cwd() -> PathBuf {
        PathBuf::from("/tmp/test")
    }

    fn unrestricted_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::unrestricted()
    }

    fn development_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path("/home/user/project").unwrap(),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path(std::env::temp_dir()).unwrap(),
                },
                access: FileSystemAccessMode::Write,
            },
        ])
    }

    fn locked_down_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::restricted(vec![])
    }

    #[test]
    fn transform_unrestricted_has_no_sandbox_setup() {
        let cmd = transform("echo hello", "bash", &unrestricted_fs(), NetworkSandboxPolicy::Enabled, &test_cwd());
        assert_eq!(cmd.program, "bash");
        assert_eq!(cmd.sandbox_type, SandboxType::Landlock);
        assert_eq!(cmd.args, vec!["-c", "echo hello"]);
        assert!(cmd.linux_sandbox.is_none());
    }

    #[test]
    fn transform_restricted_has_sandbox_setup() {
        let cmd = transform("ls", "bash", &development_fs(), NetworkSandboxPolicy::Enabled, &test_cwd());
        assert!(cmd.env.contains_key("XIAOLIN_SANDBOXED"));
        assert!(cmd.env_remove.contains(&"LD_PRELOAD".to_string()));

        let setup = cmd.linux_sandbox.expect("should have sandbox setup");
        assert!(setup.writable_roots.is_some());
        assert!(setup.network_seccomp.is_none());
    }

    #[test]
    fn transform_network_disabled_enables_seccomp() {
        use crate::NetworkSeccompMode;
        let cmd = transform(
            "curl example.com",
            "bash",
            &unrestricted_fs(),
            NetworkSandboxPolicy::Restricted,
            &test_cwd(),
        );
        let setup = cmd.linux_sandbox.expect("should have sandbox setup");
        assert!(setup.writable_roots.is_none());
        assert_eq!(setup.network_seccomp, Some(NetworkSeccompMode::Restricted));
    }

    #[test]
    fn transform_locked_down_has_full_setup() {
        use crate::NetworkSeccompMode;
        let cmd = transform("ls", "bash", &locked_down_fs(), NetworkSandboxPolicy::Restricted, &test_cwd());
        let setup = cmd.linux_sandbox.expect("should have sandbox setup");
        assert_eq!(setup.writable_roots, Some(vec![]));
        assert_eq!(setup.network_seccomp, Some(NetworkSeccompMode::Restricted));
    }

    #[test]
    fn build_args_restricted() {
        let fs = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path("/tmp").unwrap(),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path("/usr").unwrap(),
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: "**/.env".into(),
                },
                access: FileSystemAccessMode::None,
            },
        ]);
        let args = build_landlock_args(&fs, NetworkSandboxPolicy::Restricted);
        assert!(args.contains(&"--writable".to_string()));
        assert!(args.contains(&"/tmp".to_string()));
        assert!(args.contains(&"--readable".to_string()));
        assert!(args.contains(&"--deny-glob".to_string()));
        assert!(args.contains(&"--net-disabled".to_string()));
    }

    #[test]
    fn build_args_unrestricted() {
        let args = build_landlock_args(&unrestricted_fs(), NetworkSandboxPolicy::Enabled);
        assert!(args.contains(&"--fs-unrestricted".to_string()));
        assert!(args.contains(&"--net-enabled".to_string()));
    }

    #[test]
    fn build_args_network_restricted() {
        let args = build_landlock_args(&unrestricted_fs(), NetworkSandboxPolicy::Restricted);
        assert!(args.contains(&"--net-disabled".to_string()));
    }

    #[test]
    fn allow_network_for_proxy_returns_flag() {
        assert!(allow_network_for_proxy(true));
        assert!(!allow_network_for_proxy(false));
    }

    #[test]
    fn create_sandbox_args_basic() {
        let args = create_linux_sandbox_command_args(
            vec!["echo".into(), "hello".into()],
            Path::new("/home/user"),
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            Path::new("/home/user/project"),
            false,
            false,
        );
        assert!(args.contains(&"--sandbox-policy-cwd".to_string()));
        assert!(args.contains(&"/home/user/project".to_string()));
        assert!(args.contains(&"--command-cwd".to_string()));
        assert!(args.contains(&"/home/user".to_string()));
        assert!(args.contains(&"--fs-sandbox-policy".to_string()));
        assert!(args.contains(&"--net-sandbox-policy".to_string()));
        assert!(args.contains(&"--".to_string()));
        assert!(args.contains(&"echo".to_string()));
        assert!(args.contains(&"hello".to_string()));
        assert!(!args.contains(&"--use-legacy-landlock".to_string()));
        assert!(!args.contains(&"--allow-network-for-proxy".to_string()));
    }

    #[test]
    fn create_sandbox_args_with_flags() {
        let args = create_linux_sandbox_command_args(
            vec!["ls".into()],
            Path::new("/tmp"),
            &locked_down_fs(),
            NetworkSandboxPolicy::Restricted,
            Path::new("/tmp"),
            true,
            true,
        );
        assert!(args.contains(&"--use-legacy-landlock".to_string()));
        assert!(args.contains(&"--allow-network-for-proxy".to_string()));

        let sep_pos = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[sep_pos + 1], "ls");
    }

    #[test]
    fn create_sandbox_args_policy_json_valid() {
        let args = create_linux_sandbox_command_args(
            vec!["cargo".into(), "test".into()],
            Path::new("/home/user/project"),
            &development_fs(),
            NetworkSandboxPolicy::Enabled,
            Path::new("/home/user/project"),
            false,
            false,
        );
        let idx = args
            .iter()
            .position(|a| a == "--fs-sandbox-policy")
            .unwrap();
        let json_str = &args[idx + 1];
        let parsed: serde_json::Value =
            serde_json::from_str(json_str).expect("fs policy JSON should be valid");
        assert!(parsed.is_object());
    }
}
