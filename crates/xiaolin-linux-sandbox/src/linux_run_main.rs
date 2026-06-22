use anyhow::{Context, Result, bail};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use tracing::info;
use xiaolin_security::{FileSystemSandboxPolicy, NetworkSandboxPolicy};

use crate::bwrap::{BwrapNetworkMode, BwrapOptions, create_bwrap_command_args};

/// Parsed CLI matching `create_linux_sandbox_command_args` in xiaolin-sandbox.
#[derive(Debug)]
struct ParsedCli {
    sandbox_policy_cwd: PathBuf,
    command_cwd: PathBuf,
    fs_policy: FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    use_legacy_landlock: bool,
    allow_network_for_proxy: bool,
    child_args: Vec<String>,
}

/// Legacy JSON policy passed via `--policy` (backward compatibility).
#[derive(Debug, serde::Deserialize)]
pub struct SandboxPolicy {
    #[serde(default)]
    pub file_system: Option<FileSystemSandboxPolicy>,
    #[serde(default)]
    pub writable_roots: Vec<PathBuf>,
    #[serde(default)]
    pub readable_roots: Vec<PathBuf>,
    #[serde(default)]
    pub deny_read_paths: Vec<PathBuf>,
    #[serde(default)]
    pub use_bwrap: bool,
    #[serde(default)]
    pub use_landlock: bool,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub network_namespace: bool,
    /// Seccomp BPF filter mode: "restricted" blocks network syscalls,
    /// "proxy-routed" allows AF_INET for the local proxy bridge.
    #[serde(default)]
    pub seccomp_mode: Option<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub bwrap_network_mode: Option<String>,
}

pub fn run_main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let parsed = parse_args(&args)?;

    if parsed.child_args.is_empty() {
        bail!("no command specified after '--'");
    }

    info!(
        "sandbox: legacy_landlock={} allow_network_for_proxy={} net={:?} program={}",
        parsed.use_legacy_landlock,
        parsed.allow_network_for_proxy,
        parsed.net_policy,
        parsed.child_args[0],
    );

    set_no_new_privs()?;

    if parsed.use_legacy_landlock {
        if xiaolin_sandbox::landlock::policy_has_deny_read_restrictions(
            &parsed.fs_policy,
            &parsed.sandbox_policy_cwd,
        ) {
            bail!(
                "legacy Landlock cannot enforce deny-read rules; remove --use-legacy-landlock \
                 and use bubblewrap instead"
            );
        }

        let setup = build_legacy_landlock_setup(&parsed)?;
        xiaolin_sandbox::landlock::apply_sandbox_to_current_process(&setup)
            .context("apply legacy landlock sandbox")?;
        return fork_exec_wait_in_cwd(&parsed.child_args, &parsed.command_cwd);
    }

    if let Some(mode) = xiaolin_sandbox::landlock::network_seccomp_mode_for_policy(
        parsed.net_policy,
        parsed.allow_network_for_proxy,
    ) {
        xiaolin_sandbox::landlock::install_network_seccomp_filter(mode)
            .context("install network seccomp filter")?;
    }

    let network_mode = if parsed.allow_network_for_proxy {
        BwrapNetworkMode::ProxyOnly
    } else if parsed.net_policy.is_enabled() {
        BwrapNetworkMode::FullAccess
    } else {
        BwrapNetworkMode::Isolated
    };

    let options = BwrapOptions {
        mount_proc: true,
        network_mode,
        glob_scan_max_depth: Some(10),
    };

    let bwrap_args = create_bwrap_command_args(
        parsed.child_args,
        &parsed.fs_policy,
        &parsed.sandbox_policy_cwd,
        &parsed.command_cwd,
        options,
    )?;

    crate::bwrap::exec_with_bwrap(bwrap_args, None)
}

fn build_legacy_landlock_setup(
    parsed: &ParsedCli,
) -> Result<xiaolin_sandbox::LinuxSandboxSetup> {
    let writable_roots: Vec<PathBuf> = parsed
        .fs_policy
        .get_writable_roots_with_cwd(&parsed.sandbox_policy_cwd)
        .into_iter()
        .map(|abs| abs.to_path_buf())
        .collect();

    Ok(xiaolin_sandbox::LinuxSandboxSetup {
        writable_roots: Some(writable_roots),
        network_seccomp: xiaolin_sandbox::landlock::network_seccomp_mode_for_policy(
            parsed.net_policy,
            parsed.allow_network_for_proxy,
        ),
    })
}

fn parse_args(args: &[String]) -> Result<ParsedCli> {
    let mut sandbox_policy_cwd = None;
    let mut command_cwd = None;
    let mut fs_policy_json = None;
    let mut net_policy_json = None;
    let mut use_legacy_landlock = false;
    let mut allow_network_for_proxy = false;
    let mut legacy_policy_json = None;
    let mut child_args = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--sandbox-policy-cwd" => {
                i += 1;
                sandbox_policy_cwd = Some(next_value(args, i, "--sandbox-policy-cwd")?);
            }
            "--command-cwd" => {
                i += 1;
                command_cwd = Some(next_value(args, i, "--command-cwd")?);
            }
            "--fs-sandbox-policy" | "--fs-policy" => {
                let flag = args[i].as_str();
                i += 1;
                fs_policy_json = Some(next_value(args, i, flag)?);
            }
            "--net-sandbox-policy" => {
                i += 1;
                net_policy_json = Some(next_value(args, i, "--net-sandbox-policy")?);
            }
            "--use-legacy-landlock" => {
                use_legacy_landlock = true;
            }
            "--allow-network-for-proxy" => {
                allow_network_for_proxy = true;
            }
            "--policy" => {
                i += 1;
                legacy_policy_json = Some(next_value(args, i, "--policy")?);
            }
            "--" => {
                child_args = args[i + 1..].to_vec();
                break;
            }
            other => bail!("unknown argument: {other}"),
        }
        i += 1;
    }

    if let Some(legacy_json) = legacy_policy_json {
        return parse_legacy_policy(
            legacy_json,
            sandbox_policy_cwd,
            command_cwd,
            child_args,
        );
    }

    let fs_policy: FileSystemSandboxPolicy = match fs_policy_json {
        Some(json) => serde_json::from_str(&json).context("parse --fs-sandbox-policy JSON")?,
        None => bail!("missing --fs-sandbox-policy"),
    };
    let net_policy: NetworkSandboxPolicy = match net_policy_json {
        Some(json) => serde_json::from_str(&json).context("parse --net-sandbox-policy JSON")?,
        None => NetworkSandboxPolicy::Restricted,
    };

    let sandbox_policy_cwd = sandbox_policy_cwd
        .map(PathBuf::from)
        .context("missing --sandbox-policy-cwd")?;
    let command_cwd = command_cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| sandbox_policy_cwd.clone());

    Ok(ParsedCli {
        sandbox_policy_cwd,
        command_cwd,
        fs_policy,
        net_policy,
        use_legacy_landlock,
        allow_network_for_proxy,
        child_args,
    })
}

fn parse_legacy_policy(
    legacy_json: String,
    sandbox_policy_cwd: Option<String>,
    command_cwd: Option<String>,
    child_args: Vec<String>,
) -> Result<ParsedCli> {
    let policy: SandboxPolicy =
        serde_json::from_str(&legacy_json).context("parse legacy --policy JSON")?;

    let sandbox_policy_cwd = sandbox_policy_cwd
        .map(PathBuf::from)
        .or_else(|| policy.cwd.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
    let command_cwd = command_cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| sandbox_policy_cwd.clone());

    let fs_policy = policy.file_system.clone().unwrap_or_else(|| {
        let writable_abs: Vec<xiaolin_core::path::AbsolutePathBuf> = policy
            .writable_roots
            .iter()
            .filter_map(|p| xiaolin_core::path::AbsolutePathBuf::from_absolute_path(p).ok())
            .collect();
        FileSystemSandboxPolicy::workspace_write(&writable_abs, false, false)
    });

    let allow_network_for_proxy = policy.bwrap_network_mode.as_deref() == Some("proxy_only")
        || policy.proxy_port.is_some();
    let use_legacy_landlock = policy.use_landlock && !policy.use_bwrap;
    let net_policy = if policy.network_namespace && !allow_network_for_proxy {
        NetworkSandboxPolicy::Restricted
    } else if allow_network_for_proxy {
        NetworkSandboxPolicy::Restricted
    } else {
        tracing::warn!(
            "legacy --policy: no explicit network restriction configured; defaulting to Restricted network"
        );
        NetworkSandboxPolicy::Restricted
    };

    Ok(ParsedCli {
        sandbox_policy_cwd,
        command_cwd,
        fs_policy,
        net_policy,
        use_legacy_landlock,
        allow_network_for_proxy,
        child_args,
    })
}

fn next_value(args: &[String], index: usize, flag: &str) -> Result<String> {
    args.get(index)
        .cloned()
        .with_context(|| format!("{flag} requires a value"))
}

fn set_no_new_privs() -> Result<()> {
    let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret != 0 {
        bail!(
            "prctl(PR_SET_NO_NEW_PRIVS) failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn fork_exec_wait_in_cwd(args: &[String], cwd: &Path) -> Result<()> {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::{ForkResult, fork};

    let child_pid = match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            std::env::set_current_dir(cwd).ok();
            exec_child(args)?;
            unreachable!()
        }
        Ok(ForkResult::Parent { child }) => child,
        Err(e) => bail!("fork failed: {e}"),
    };

    install_signal_forwarder(child_pid);

    loop {
        match waitpid(child_pid, None) {
            Ok(WaitStatus::Exited(_, code)) => {
                std::process::exit(code);
            }
            Ok(WaitStatus::Signaled(_, sig, _)) => {
                std::process::exit(128 + sig as i32);
            }
            Ok(_) => continue,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => bail!("waitpid failed: {e}"),
        }
    }
}

fn install_signal_forwarder(child: nix::unistd::Pid) {
    use nix::sys::signal::{Signal, kill};

    let child_pid = child;
    let _ = ctrlc::set_handler(move || {
        let _ = kill(child_pid, Signal::SIGINT);
    });
}

fn exec_child(args: &[String]) -> Result<()> {
    let program = CString::new(args[0].as_bytes()).context("invalid program name")?;
    let c_args: Vec<CString> = args
        .iter()
        .map(|a| CString::new(a.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .context("invalid argument")?;

    nix::unistd::execvp(&program, &c_args).context("execvp failed")?;
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unified_cli_args() {
        let args = vec![
            "xiaolin-linux-sandbox".into(),
            "--sandbox-policy-cwd".into(),
            "/home/user/project".into(),
            "--command-cwd".into(),
            "/home/user".into(),
            "--fs-sandbox-policy".into(),
            r#"{"kind":"unrestricted","entries":[]}"#.into(),
            "--net-sandbox-policy".into(),
            r#""enabled""#.into(),
            "--".into(),
            "/bin/ls".into(),
        ];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.sandbox_policy_cwd, PathBuf::from("/home/user/project"));
        assert_eq!(parsed.command_cwd, PathBuf::from("/home/user"));
        assert!(parsed.net_policy.is_enabled());
        assert_eq!(parsed.child_args, vec!["/bin/ls"]);
    }

    #[test]
    fn parse_transform_external_compat_args() {
        let args = vec![
            "xiaolin-linux-sandbox".into(),
            "--sandbox-policy-cwd".into(),
            "/tmp".into(),
            "--fs-sandbox-policy".into(),
            r#"{"kind":"restricted","entries":[]}"#.into(),
            "--allow-network-for-proxy".into(),
            "--".into(),
            "bash".into(),
            "-c".into(),
            "echo hi".into(),
        ];
        let parsed = parse_args(&args).unwrap();
        assert!(parsed.allow_network_for_proxy);
        assert_eq!(parsed.command_cwd, PathBuf::from("/tmp"));
        assert_eq!(
            parsed.child_args,
            vec!["bash", "-c", "echo hi"]
        );
    }

    #[test]
    fn parse_legacy_policy_flag() {
        let args = vec![
            "xiaolin-linux-sandbox".into(),
            "--policy".into(),
            r#"{"use_landlock":true}"#.into(),
            "--".into(),
            "/bin/ls".into(),
            "/tmp".into(),
        ];
        let parsed = parse_args(&args).unwrap();
        assert!(parsed.use_legacy_landlock);
        assert_eq!(parsed.child_args, vec!["/bin/ls", "/tmp"]);
    }

    #[test]
    fn parse_args_unknown_flag() {
        let args = vec!["xiaolin-linux-sandbox".into(), "--bogus".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn sandbox_policy_deserialize_defaults() {
        let policy: SandboxPolicy = serde_json::from_str("{}").unwrap();
        assert!(!policy.use_bwrap);
        assert!(!policy.use_landlock);
        assert!(policy.writable_roots.is_empty());
        assert!(policy.proxy_port.is_none());
    }
}
