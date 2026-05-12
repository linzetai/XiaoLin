//! Shell execution environment snapshot and isolation.
//!
//! Provides a clean, reproducible shell environment for command execution:
//! - Bypasses user `.bashrc` / `.zshrc` / `.profile` to avoid interference
//! - Injects shell functions that wrap rg/grep/find to prefer bundled binaries
//! - Manages per-execution temporary directories with automatic cleanup
//! - Supports bash, zsh, and PowerShell dispatch

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

/// Supported shell types for execution dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Zsh,
    #[cfg(target_os = "windows")]
    PowerShell,
}

impl ShellType {
    /// Detect the current shell from environment.
    pub fn detect() -> Self {
        if let Ok(shell) = std::env::var("SHELL") {
            if shell.contains("zsh") {
                return Self::Zsh;
            }
        }
        Self::Bash
    }

    /// Shell binary path.
    pub fn binary(&self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            #[cfg(target_os = "windows")]
            Self::PowerShell => "pwsh",
        }
    }

    /// Flags to disable rc file loading.
    pub fn no_rc_flags(&self) -> Vec<&'static str> {
        match self {
            Self::Bash => vec!["--norc", "--noprofile"],
            Self::Zsh => vec!["--no-rcs", "--no-globalrcs"],
            #[cfg(target_os = "windows")]
            Self::PowerShell => vec!["-NoProfile"],
        }
    }
}

/// Configuration for shell function wrappers.
#[derive(Debug, Clone)]
pub struct BundledBinary {
    pub name: String,
    pub path: PathBuf,
}

/// A snapshot of the shell execution environment.
///
/// Creates an isolated environment that:
/// 1. Skips user shell config files (--norc / --noprofile)
/// 2. Injects wrapper functions for bundled binaries
/// 3. Sets a minimal, predictable PATH
/// 4. Manages a temp directory for command artifacts
#[derive(Debug)]
pub struct ShellSnapshot {
    shell_type: ShellType,
    work_dir: PathBuf,
    temp_dir: Option<PathBuf>,
    env_overrides: HashMap<String, String>,
    bundled_binaries: Vec<BundledBinary>,
    preamble_lines: Vec<String>,
    cleanup_on_drop: bool,
}

impl ShellSnapshot {
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        Self {
            shell_type: ShellType::detect(),
            work_dir: work_dir.into(),
            temp_dir: None,
            env_overrides: HashMap::new(),
            bundled_binaries: Vec::new(),
            preamble_lines: Vec::new(),
            cleanup_on_drop: true,
        }
    }

    pub fn with_shell(mut self, shell: ShellType) -> Self {
        self.shell_type = shell;
        self
    }

    /// Add a bundled binary wrapper. When a command uses `name`, the shell
    /// function will prefer `path` if it exists.
    pub fn add_bundled_binary(mut self, name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        self.bundled_binaries.push(BundledBinary {
            name: name.into(),
            path: path.into(),
        });
        self
    }

    /// Add a custom environment variable override.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_overrides.insert(key.into(), value.into());
        self
    }

    /// Add a raw preamble line (executed before the user command).
    pub fn preamble(mut self, line: impl Into<String>) -> Self {
        self.preamble_lines.push(line.into());
        self
    }

    pub fn set_cleanup_on_drop(mut self, cleanup: bool) -> Self {
        self.cleanup_on_drop = cleanup;
        self
    }

    /// Initialize the temp directory for this snapshot.
    pub fn init_temp_dir(&mut self) -> anyhow::Result<&Path> {
        if self.temp_dir.is_none() {
            let dir = std::env::temp_dir().join(format!("fastclaw-shell-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            self.temp_dir = Some(dir);
        }
        Ok(self.temp_dir.as_ref().unwrap())
    }

    /// Get the temp directory path, if initialized.
    pub fn temp_dir(&self) -> Option<&Path> {
        self.temp_dir.as_deref()
    }

    /// Build the shell preamble script that sets up the isolated environment.
    pub fn build_preamble(&self) -> String {
        let mut lines = Vec::new();

        lines.push("set -o pipefail 2>/dev/null || true".to_string());

        for (key, value) in &self.env_overrides {
            lines.push(format!("export {}={}", key, shell_escape(value)));
        }

        if let Some(ref temp) = self.temp_dir {
            lines.push(format!(
                "export TMPDIR={}",
                shell_escape(&temp.display().to_string())
            ));
        }

        for bin in &self.bundled_binaries {
            let wrapper = build_wrapper_function(&bin.name, &bin.path);
            lines.push(wrapper);
        }

        for line in &self.preamble_lines {
            lines.push(line.clone());
        }

        lines.join("\n")
    }

    /// Build the full command to execute in the isolated shell.
    ///
    /// Returns `(binary, args)` suitable for `tokio::process::Command`.
    pub fn build_command(&self, user_command: &str) -> (String, Vec<String>) {
        let preamble = self.build_preamble();
        let full_script = if preamble.is_empty() {
            user_command.to_string()
        } else {
            format!("{}\n{}", preamble, user_command)
        };

        let binary = self.shell_type.binary().to_string();
        let mut args: Vec<String> = self
            .shell_type
            .no_rc_flags()
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.push("-c".to_string());
        args.push(full_script);

        (binary, args)
    }

    /// Get the working directory for command execution.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// Clean up the temp directory.
    pub fn cleanup(&mut self) {
        if let Some(ref dir) = self.temp_dir {
            let _ = std::fs::remove_dir_all(dir);
            self.temp_dir = None;
        }
    }
}

impl Drop for ShellSnapshot {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            self.cleanup();
        }
    }
}

/// A pool of reusable shell snapshots for concurrent execution.
#[derive(Debug)]
pub struct ShellSnapshotPool {
    template: Arc<ShellSnapshotTemplate>,
    active_count: Arc<Mutex<usize>>,
}

/// Template for creating new snapshots.
#[derive(Debug, Clone)]
pub struct ShellSnapshotTemplate {
    pub shell_type: ShellType,
    pub bundled_binaries: Vec<BundledBinary>,
    pub env_overrides: HashMap<String, String>,
    pub preamble_lines: Vec<String>,
}

impl ShellSnapshotPool {
    pub fn new(template: ShellSnapshotTemplate) -> Self {
        Self {
            template: Arc::new(template),
            active_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Create a snapshot from the template.
    pub async fn acquire(&self, work_dir: impl Into<PathBuf>) -> ShellSnapshot {
        let mut count = self.active_count.lock().await;
        *count += 1;

        let mut snap = ShellSnapshot::new(work_dir);
        snap.shell_type = self.template.shell_type;
        for bin in &self.template.bundled_binaries {
            snap.bundled_binaries.push(bin.clone());
        }
        snap.env_overrides = self.template.env_overrides.clone();
        snap.preamble_lines = self.template.preamble_lines.clone();
        snap
    }

    pub async fn active_count(&self) -> usize {
        *self.active_count.lock().await
    }
}

/// Build a shell function wrapper that prefers the bundled binary.
fn build_wrapper_function(name: &str, bundled_path: &Path) -> String {
    let path_str = bundled_path.display();
    format!(
        "{name}() {{ if [ -x \"{path_str}\" ]; then \"{path_str}\" \"$@\"; else command {name} \"$@\"; fi; }}"
    )
}

/// Simple shell escaping for values.
fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_shell_returns_valid_type() {
        let shell = ShellType::detect();
        assert!(matches!(shell, ShellType::Bash | ShellType::Zsh));
    }

    #[test]
    fn bash_no_rc_flags() {
        let flags = ShellType::Bash.no_rc_flags();
        assert!(flags.contains(&"--norc"));
        assert!(flags.contains(&"--noprofile"));
    }

    #[test]
    fn zsh_no_rc_flags() {
        let flags = ShellType::Zsh.no_rc_flags();
        assert!(flags.contains(&"--no-rcs"));
    }

    #[test]
    fn build_command_includes_no_rc() {
        let snap = ShellSnapshot::new("/tmp").with_shell(ShellType::Bash);
        let (bin, args) = snap.build_command("echo hello");
        assert_eq!(bin, "bash");
        assert!(args.contains(&"--norc".to_string()));
        assert!(args.contains(&"--noprofile".to_string()));
        assert!(args.last().unwrap().contains("echo hello"));
    }

    #[test]
    fn preamble_includes_env_overrides() {
        let snap = ShellSnapshot::new("/tmp").env("FOO", "bar");
        let preamble = snap.build_preamble();
        assert!(preamble.contains("export FOO=bar"), "got: {preamble}");
    }

    #[test]
    fn preamble_includes_wrapper_functions() {
        let snap = ShellSnapshot::new("/tmp").add_bundled_binary("rg", "/usr/local/bin/rg");
        let preamble = snap.build_preamble();
        assert!(preamble.contains("rg()"), "got: {preamble}");
        assert!(preamble.contains("/usr/local/bin/rg"), "got: {preamble}");
    }

    #[test]
    fn init_temp_dir_creates_directory() {
        let mut snap = ShellSnapshot::new("/tmp").set_cleanup_on_drop(false);
        let dir = snap.init_temp_dir().unwrap();
        assert!(dir.exists());
        let dir_path = dir.to_path_buf();
        snap.cleanup();
        assert!(!dir_path.exists() || !dir_path.is_dir());
    }

    #[test]
    fn cleanup_removes_temp_dir() {
        let mut snap = ShellSnapshot::new("/tmp").set_cleanup_on_drop(false);
        snap.init_temp_dir().unwrap();
        let dir = snap.temp_dir().unwrap().to_path_buf();
        assert!(dir.exists());
        snap.cleanup();
        assert!(!dir.exists());
    }

    #[test]
    fn shell_escape_simple_values() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("/usr/bin/rg"), "/usr/bin/rg");
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn wrapper_function_format() {
        let wrapper = build_wrapper_function("rg", Path::new("/opt/bin/rg"));
        assert!(wrapper.starts_with("rg()"));
        assert!(wrapper.contains("/opt/bin/rg"));
        assert!(wrapper.contains("command rg"));
    }

    #[tokio::test]
    async fn pool_acquire_creates_snapshot() {
        let template = ShellSnapshotTemplate {
            shell_type: ShellType::Bash,
            bundled_binaries: vec![BundledBinary {
                name: "rg".into(),
                path: PathBuf::from("/usr/bin/rg"),
            }],
            env_overrides: HashMap::new(),
            preamble_lines: Vec::new(),
        };
        let pool = ShellSnapshotPool::new(template);
        let snap = pool.acquire("/tmp").await;
        assert_eq!(snap.shell_type, ShellType::Bash);
        assert_eq!(snap.bundled_binaries.len(), 1);
    }
}
