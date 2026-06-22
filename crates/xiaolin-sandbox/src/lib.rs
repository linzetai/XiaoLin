#![allow(unsafe_code)]

#[cfg(target_os = "linux")]
pub mod bwrap;
pub mod landlock;
mod noop;
pub mod sandbox_policy;
pub mod seatbelt;
pub mod windows;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use xiaolin_security::policy_transforms;
use xiaolin_security::{FileSystemSandboxPolicy, NetworkSandboxPolicy};

/// The type of sandbox backend selected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxType {
    Landlock,
    /// External sandbox binary mode: serializes the policy as JSON and
    /// delegates enforcement to an external helper binary (e.g. one that
    /// wraps bubblewrap + seccomp).
    ExternalBinary,
    Seatbelt,
    RestrictedToken,
    Noop,
}

impl std::fmt::Display for SandboxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Landlock => write!(f, "landlock"),
            Self::ExternalBinary => write!(f, "external_binary"),
            Self::Seatbelt => write!(f, "seatbelt"),
            Self::RestrictedToken => write!(f, "restricted_token"),
            Self::Noop => write!(f, "noop"),
        }
    }
}

/// How seccomp should handle network-related syscalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkSeccompMode {
    /// Block all network syscalls except AF_UNIX sockets for local IPC.
    Restricted,
    /// Allow AF_INET/AF_INET6 (for local TCP bridge to a proxy), block
    /// new AF_UNIX sockets. Does **not** block connect/bind/listen etc.
    ProxyRouted,
}

/// Linux-specific sandbox configuration applied via `pre_exec`.
///
/// This struct is defined on all platforms for API uniformity but is only
/// applied in child processes on Linux through Landlock and seccomp.
#[derive(Debug, Clone, Default)]
pub struct LinuxSandboxSetup {
    /// If `Some`, apply Landlock filesystem rules granting read-write access
    /// to these roots. Everything else becomes read-only. If `None`, filesystem
    /// Landlock is not applied.
    pub writable_roots: Option<Vec<PathBuf>>,
    /// If `Some`, install a seccomp BPF filter according to the selected mode.
    pub network_seccomp: Option<NetworkSeccompMode>,
}

/// A command transformed for sandboxed execution.
#[derive(Debug, Clone)]
pub struct SandboxedCommand {
    /// The program to execute.
    pub program: String,
    /// Arguments to pass.
    pub args: Vec<String>,
    /// Working directory (if overridden).
    pub working_dir: Option<PathBuf>,
    /// Environment variables to set.
    pub env: HashMap<String, String>,
    /// Environment variables to remove.
    pub env_remove: Vec<String>,
    /// Which sandbox backend produced this command.
    pub sandbox_type: SandboxType,
    /// Linux-specific sandbox configuration applied via pre_exec in the child.
    pub linux_sandbox: Option<LinuxSandboxSetup>,
}

impl SandboxedCommand {
    /// Convert to a `tokio::process::Command` ready for execution.
    ///
    /// On Linux, if `linux_sandbox` is set, a `pre_exec` hook applies Landlock
    /// filesystem rules and/or seccomp network filtering in the child process
    /// before exec.
    pub fn into_tokio_command(self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(&self.args);
        if let Some(dir) = &self.working_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        for k in &self.env_remove {
            cmd.env_remove(k);
        }

        #[cfg(target_os = "linux")]
        if let Some(setup) = self.linux_sandbox {
            // SAFETY: The pre_exec closure runs after fork() in the child
            // process (single-threaded). Landlock and seccomp syscalls are
            // safe to call in this context and only affect the child.
            unsafe {
                cmd.pre_exec(move || landlock::apply_sandbox_to_current_process(&setup));
            }
        }

        cmd
    }
}

/// User-specified sandbox preference for dynamic strategy selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SandboxPreference {
    /// Automatically select the best available sandbox (default behavior).
    #[default]
    Auto,
    /// Require a real sandbox; fail if none is available.
    Require,
    /// Explicitly disable sandboxing.
    Forbid,
}

impl std::fmt::Display for SandboxPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Require => write!(f, "require"),
            Self::Forbid => write!(f, "forbid"),
        }
    }
}

/// Error returned when sandbox selection fails (e.g. `Require` mode with
/// no backend available).
#[derive(Debug, Clone)]
pub struct SandboxSelectionError {
    pub message: String,
}

impl std::fmt::Display for SandboxSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sandbox selection error: {}", self.message)
    }
}

impl std::error::Error for SandboxSelectionError {}

/// Manages sandbox lifecycle: detection, selection, and command transformation.
pub struct SandboxManager {
    sandbox_type: SandboxType,
}

impl SandboxManager {
    /// Detect the best available sandbox for the current platform.
    pub fn detect() -> Self {
        let sandbox_type = Self::detect_platform();
        tracing::info!(sandbox = %sandbox_type, "sandbox backend selected");
        Self { sandbox_type }
    }

    /// Select a sandbox backend according to a user preference.
    ///
    /// - `Auto`: detect the best backend, fall back to `Noop`.
    /// - `Require`: detect the best backend, fail if only `Noop`.
    /// - `Forbid`: always use `Noop`.
    pub fn select(preference: SandboxPreference) -> Result<Self, SandboxSelectionError> {
        match preference {
            SandboxPreference::Forbid => {
                tracing::info!("sandbox disabled by preference");
                Ok(Self {
                    sandbox_type: SandboxType::Noop,
                })
            }
            SandboxPreference::Auto => Ok(Self::detect()),
            SandboxPreference::Require => {
                let detected = Self::detect_platform();
                if detected == SandboxType::Noop {
                    Err(SandboxSelectionError {
                        message: "sandbox required but no backend is available on this platform"
                            .into(),
                    })
                } else {
                    tracing::info!(sandbox = %detected, "sandbox backend selected (required mode)");
                    Ok(Self {
                        sandbox_type: detected,
                    })
                }
            }
        }
    }

    /// Select a sandbox backend with policy-aware auto-detection.
    ///
    /// In `Auto` mode, if the policies require enforcement (restricted
    /// filesystem or non-`Enabled` network), behaves like `Require`.
    pub fn select_with_policy(
        preference: SandboxPreference,
        fs_policy: &FileSystemSandboxPolicy,
        net_policy: NetworkSandboxPolicy,
    ) -> Result<Self, SandboxSelectionError> {
        let effective = if preference == SandboxPreference::Auto
            && policy_transforms::should_require_platform_sandbox(fs_policy, net_policy)
        {
            SandboxPreference::Require
        } else {
            preference
        };
        Self::select(effective)
    }

    /// Create a manager with an explicit backend (for testing).
    pub fn with_type(sandbox_type: SandboxType) -> Self {
        Self { sandbox_type }
    }

    /// Which sandbox backend is active.
    pub fn sandbox_type(&self) -> SandboxType {
        self.sandbox_type
    }

    /// Whether a real (non-noop) sandbox is available.
    pub fn is_available(&self) -> bool {
        self.sandbox_type != SandboxType::Noop
    }

    /// Transform a shell command + policy into a sandboxed command.
    ///
    /// The returned `SandboxedCommand` wraps the original command with
    /// platform-specific isolation mechanisms.
    pub fn transform(
        &self,
        command: &str,
        shell: &str,
        fs_policy: &FileSystemSandboxPolicy,
        net_policy: NetworkSandboxPolicy,
        cwd: &Path,
    ) -> SandboxedCommand {
        match self.sandbox_type {
            SandboxType::Landlock | SandboxType::ExternalBinary => {
                landlock::transform(command, shell, fs_policy, net_policy, cwd)
            }
            SandboxType::Seatbelt => {
                seatbelt::transform(command, shell, fs_policy, net_policy, cwd)
            }
            SandboxType::RestrictedToken => {
                windows::transform(command, shell, fs_policy, net_policy)
            }
            SandboxType::Noop => noop::transform(command, shell),
        }
    }

    /// Transform using a structured request, merging additional permissions
    /// into the base policy before applying sandbox rules.
    pub fn transform_request(
        &self,
        request: &SandboxTransformRequest<'_>,
    ) -> Result<SandboxedCommand, SandboxTransformError> {
        let effective_fs = match request.additional_fs_policy {
            Some(additional) => {
                policy_transforms::effective_file_system_sandbox_policy(
                    Some(request.fs_policy),
                    Some(additional),
                )
            }
            None => request.fs_policy.clone(),
        };
        let effective_net = match request.additional_net_policy {
            Some(additional) => {
                policy_transforms::effective_network_sandbox_policy(
                    Some(request.net_policy),
                    Some(additional),
                )
            }
            None => request.net_policy,
        };

        #[cfg(target_os = "linux")]
        if (self.sandbox_type == SandboxType::Landlock
            || self.sandbox_type == SandboxType::ExternalBinary)
            && bwrap::is_wsl1()
        {
            return Err(SandboxTransformError::Wsl1Unsupported);
        }

        #[cfg(target_os = "linux")]
        {
            let cwd = request.sandbox_policy_cwd.unwrap_or(Path::new("/"));
            let has_deny_read =
                landlock::policy_has_deny_read_restrictions(&effective_fs, cwd);

            if has_deny_read && request.use_legacy_landlock {
                return Err(SandboxTransformError::DenyReadRequiresExternalSandbox);
            }

            let explicit_exe = request.linux_sandbox_exe.map(Path::to_path_buf);
            let resolved_exe = explicit_exe.or_else(|| {
                if request.use_legacy_landlock {
                    None
                } else {
                    landlock::discover_linux_sandbox_exe()
                }
            });

            if has_deny_read && resolved_exe.is_none() {
                return Err(SandboxTransformError::DenyReadRequiresExternalSandbox);
            }

            let prefer_external = self.sandbox_type == SandboxType::ExternalBinary
                || (self.sandbox_type == SandboxType::Landlock
                    && resolved_exe.is_some()
                    && !request.use_legacy_landlock)
                || (has_deny_read && resolved_exe.is_some());

            if prefer_external {
                let exe = resolved_exe
                    .ok_or(SandboxTransformError::MissingLinuxSandboxExecutable)?;
                return landlock::transform_external(
                    request.command,
                    request.shell,
                    &effective_fs,
                    effective_net,
                    &exe,
                    cwd,
                    request.enforce_managed_network,
                );
            }

            if has_deny_read {
                return Err(SandboxTransformError::DenyReadRequiresExternalSandbox);
            }
        }

        let cwd = request.sandbox_policy_cwd.unwrap_or(Path::new("/"));
        Ok(self.transform(
            request.command,
            request.shell,
            &effective_fs,
            effective_net,
            cwd,
        ))
    }

    #[cfg(target_os = "linux")]
    fn detect_platform() -> SandboxType {
        if bwrap::is_wsl1() {
            tracing::warn!("WSL1 detected; bubblewrap unavailable, falling back to noop sandbox");
            return SandboxType::Noop;
        }
        if landlock::is_available() {
            SandboxType::Landlock
        } else {
            tracing::warn!("Landlock not available on this kernel; falling back to noop sandbox");
            SandboxType::Noop
        }
    }

    #[cfg(target_os = "macos")]
    fn detect_platform() -> SandboxType {
        if seatbelt::is_available() {
            SandboxType::Seatbelt
        } else {
            tracing::warn!("sandbox-exec not found; falling back to noop sandbox");
            SandboxType::Noop
        }
    }

    #[cfg(target_os = "windows")]
    fn detect_platform() -> SandboxType {
        SandboxType::RestrictedToken
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn detect_platform() -> SandboxType {
        tracing::warn!("no sandbox backend available for this platform");
        SandboxType::Noop
    }
}

/// Bundled arguments for sandbox transformation.
///
/// Carries all optional context needed for policy-aware sandbox wrapping,
/// keeping call sites self-documenting.
pub struct SandboxTransformRequest<'a> {
    pub command: &'a str,
    pub shell: &'a str,
    pub fs_policy: &'a FileSystemSandboxPolicy,
    pub net_policy: NetworkSandboxPolicy,
    /// Working directory for sandbox policy resolution.
    pub sandbox_policy_cwd: Option<&'a Path>,
    /// Path to the external Linux sandbox helper binary. When provided and
    /// on Linux, the manager may choose `ExternalBinary` mode.
    pub linux_sandbox_exe: Option<&'a Path>,
    /// Whether to use legacy in-process Landlock when an external binary
    /// is available. Defaults to `false` (prefer external binary).
    pub use_legacy_landlock: bool,
    /// Whether the proxy network is managed (requires proxy routing).
    pub enforce_managed_network: bool,
    /// Extra filesystem permissions that widen the base policy.
    pub additional_fs_policy: Option<&'a FileSystemSandboxPolicy>,
    /// Extra network permissions that widen the base policy.
    pub additional_net_policy: Option<NetworkSandboxPolicy>,
}

/// Error returned when sandbox transformation fails.
#[derive(Debug, Clone)]
pub enum SandboxTransformError {
    /// A Linux sandbox executable was required but not provided.
    MissingLinuxSandboxExecutable,
    /// Failed to serialize sandbox policy JSON for the external helper.
    PolicySerializationFailed(String),
    /// WSL1 does not support bubblewrap.
    #[cfg(target_os = "linux")]
    Wsl1Unsupported,
    /// Deny-read rules require bubblewrap; legacy Landlock cannot enforce them.
    #[cfg(target_os = "linux")]
    DenyReadRequiresExternalSandbox,
    /// Seatbelt is only available on macOS.
    #[cfg(not(target_os = "macos"))]
    SeatbeltUnavailable,
}

impl std::fmt::Display for SandboxTransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingLinuxSandboxExecutable => {
                write!(f, "missing linux-sandbox executable path")
            }
            Self::PolicySerializationFailed(msg) => {
                write!(f, "failed to serialize sandbox policy: {msg}")
            }
            #[cfg(target_os = "linux")]
            Self::Wsl1Unsupported => write!(f, "{}", bwrap::WSL1_BWRAP_WARNING),
            #[cfg(target_os = "linux")]
            Self::DenyReadRequiresExternalSandbox => write!(
                f,
                "deny-read filesystem rules require the external Linux sandbox (bubblewrap); \
                 legacy Landlock cannot enforce them"
            ),
            #[cfg(not(target_os = "macos"))]
            Self::SeatbeltUnavailable => {
                write!(f, "seatbelt sandbox is only available on macOS")
            }
        }
    }
}

impl std::error::Error for SandboxTransformError {}

#[cfg(test)]
mod tests {
    use super::*;

    use xiaolin_security::FileSystemSandboxEntry;

    fn unrestricted_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::unrestricted()
    }

    #[test]
    fn noop_manager_passthrough() {
        let mgr = SandboxManager::with_type(SandboxType::Noop);
        assert!(!mgr.is_available());
        let cmd = mgr.transform(
            "echo hello",
            "bash",
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
            std::path::Path::new("/tmp/test"),
        );
        assert_eq!(cmd.sandbox_type, SandboxType::Noop);
        assert_eq!(cmd.program, "bash");
        assert!(cmd.args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn sandboxed_command_converts_to_tokio() {
        let cmd = SandboxedCommand {
            program: "bash".into(),
            args: vec!["-c".into(), "echo test".into()],
            working_dir: Some(PathBuf::from("/tmp")),
            env: HashMap::from([("FOO".into(), "bar".into())]),
            env_remove: vec!["SECRET".into()],
            sandbox_type: SandboxType::Noop,
            linux_sandbox: None,
        };
        let _tokio_cmd = cmd.into_tokio_command();
    }

    #[test]
    fn sandbox_type_display() {
        assert_eq!(SandboxType::Landlock.to_string(), "landlock");
        assert_eq!(SandboxType::Seatbelt.to_string(), "seatbelt");
        assert_eq!(SandboxType::RestrictedToken.to_string(), "restricted_token");
        assert_eq!(SandboxType::Noop.to_string(), "noop");
    }

    #[test]
    fn detect_returns_some_backend() {
        let mgr = SandboxManager::detect();
        let _ = mgr.sandbox_type();
    }

    #[test]
    fn linux_sandbox_setup_default_is_inactive() {
        let setup = LinuxSandboxSetup::default();
        assert!(setup.writable_roots.is_none());
        assert!(setup.network_seccomp.is_none());
    }

    #[test]
    fn sandbox_preference_default_is_auto() {
        assert_eq!(SandboxPreference::default(), SandboxPreference::Auto);
    }

    #[test]
    fn sandbox_preference_display() {
        assert_eq!(SandboxPreference::Auto.to_string(), "auto");
        assert_eq!(SandboxPreference::Require.to_string(), "require");
        assert_eq!(SandboxPreference::Forbid.to_string(), "forbid");
    }

    #[test]
    fn select_forbid_returns_noop() {
        let mgr = SandboxManager::select(SandboxPreference::Forbid).unwrap();
        assert_eq!(mgr.sandbox_type(), SandboxType::Noop);
        assert!(!mgr.is_available());
    }

    #[test]
    fn select_auto_succeeds() {
        let mgr = SandboxManager::select(SandboxPreference::Auto).unwrap();
        let _ = mgr.sandbox_type();
    }

    #[test]
    fn select_with_policy_auto_escalates_when_needed() {
        let restricted_fs = FileSystemSandboxPolicy::restricted(vec![]);
        let result = SandboxManager::select_with_policy(
            SandboxPreference::Auto,
            &restricted_fs,
            NetworkSandboxPolicy::Enabled,
        );
        match result {
            Ok(mgr) => assert!(mgr.is_available()),
            Err(e) => assert!(e.message.contains("required")),
        }
    }

    #[test]
    fn select_with_policy_auto_unrestricted_succeeds() {
        let mgr = SandboxManager::select_with_policy(
            SandboxPreference::Auto,
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
        )
        .unwrap();
        let _ = mgr.sandbox_type();
    }

    #[test]
    fn select_require_returns_error_or_real_backend() {
        let result = SandboxManager::select(SandboxPreference::Require);
        match result {
            Ok(mgr) => {
                assert!(mgr.is_available());
                assert_ne!(mgr.sandbox_type(), SandboxType::Noop);
            }
            Err(e) => {
                assert!(e.message.contains("required"));
            }
        }
    }

    #[test]
    fn transform_request_without_additional_permissions() {
        let mgr = SandboxManager::with_type(SandboxType::Noop);
        let fs = unrestricted_fs();
        let request = SandboxTransformRequest {
            command: "echo hello",
            shell: "bash",
            fs_policy: &fs,
            net_policy: NetworkSandboxPolicy::Enabled,
            sandbox_policy_cwd: None,
            linux_sandbox_exe: None,
            use_legacy_landlock: false,
            enforce_managed_network: false,
            additional_fs_policy: None,
            additional_net_policy: None,
        };
        let cmd = mgr.transform_request(&request).unwrap();
        assert_eq!(cmd.sandbox_type, SandboxType::Noop);
        assert!(cmd.args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn transform_request_merges_additional_permissions() {
        use xiaolin_security::{FileSystemAccessMode, FileSystemPath};

        let mgr = SandboxManager::with_type(SandboxType::Noop);
        let base_fs = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: xiaolin_core::path::AbsolutePathBuf::from_absolute_path("/tmp").unwrap(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let extra_fs = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: xiaolin_core::path::AbsolutePathBuf::from_absolute_path("/home").unwrap(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let request = SandboxTransformRequest {
            command: "ls",
            shell: "bash",
            fs_policy: &base_fs,
            net_policy: NetworkSandboxPolicy::Restricted,
            sandbox_policy_cwd: Some(Path::new("/tmp")),
            linux_sandbox_exe: None,
            use_legacy_landlock: false,
            enforce_managed_network: false,
            additional_fs_policy: Some(&extra_fs),
            additional_net_policy: Some(NetworkSandboxPolicy::Enabled),
        };
        let cmd = mgr.transform_request(&request).unwrap();
        assert_eq!(cmd.sandbox_type, SandboxType::Noop);
    }

    #[test]
    fn sandbox_transform_error_display() {
        let err = SandboxTransformError::MissingLinuxSandboxExecutable;
        assert!(err.to_string().contains("linux-sandbox"));
    }
}
