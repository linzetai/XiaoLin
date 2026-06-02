use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const SYSTEM_BWRAP_PROGRAM: &str = "bwrap";

const MISSING_BWRAP_WARNING: &str = concat!(
    "XiaoLin could not find bubblewrap on PATH. ",
    "Install bubblewrap with your OS package manager. ",
    "XiaoLin's Linux sandbox requires bubblewrap for full isolation.",
);

const USER_NAMESPACE_WARNING: &str =
    "XiaoLin's Linux sandbox uses bubblewrap and needs access to create user namespaces.";

pub(crate) const WSL1_BWRAP_WARNING: &str = concat!(
    "XiaoLin's Linux sandbox uses bubblewrap, which is not supported on WSL1 ",
    "because WSL1 cannot create the required user namespaces. ",
    "Use WSL2 for sandboxed shell commands."
);

const USER_NAMESPACE_FAILURES: [&str; 4] = [
    "loopback: Failed RTM_NEWADDR",
    "loopback: Failed RTM_NEWLINK",
    "setting up uid map: Permission denied",
    "No permissions to create a new namespace",
];

const SYSTEM_BWRAP_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const SYSTEM_BWRAP_PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const SYSTEM_BWRAP_PROBE_STDERR_LIMIT_BYTES: u64 = 64 * 1024;

/// Check whether bubblewrap is available and functional on this system.
///
/// Returns `Some(warning)` if there is a problem (missing bwrap, WSL1, or
/// missing user namespace support). Returns `None` if everything looks good.
pub fn system_bwrap_warning() -> Option<String> {
    if is_wsl1() {
        return Some(WSL1_BWRAP_WARNING.to_string());
    }

    let system_bwrap_path = find_system_bwrap_in_path();
    system_bwrap_warning_for_path(system_bwrap_path.as_deref())
}

fn system_bwrap_warning_for_path(system_bwrap_path: Option<&Path>) -> Option<String> {
    if is_wsl1() {
        return Some(WSL1_BWRAP_WARNING.to_string());
    }

    let Some(system_bwrap_path) = system_bwrap_path else {
        return Some(MISSING_BWRAP_WARNING.to_string());
    };

    if !system_bwrap_has_user_namespace_access(system_bwrap_path, SYSTEM_BWRAP_PROBE_TIMEOUT) {
        return Some(USER_NAMESPACE_WARNING.to_string());
    }

    None
}

fn system_bwrap_has_user_namespace_access(system_bwrap_path: &Path, timeout: Duration) -> bool {
    let Ok(mut child) = Command::new(system_bwrap_path)
        .args([
            "--unshare-user",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "/bin/true",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return true;
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = child.stderr.take().map_or_else(Vec::new, |stderr| {
                    let fd = stderr.as_raw_fd();
                    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                    if flags < 0
                        || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                    {
                        return Vec::new();
                    }

                    let mut bytes = Vec::new();
                    let mut stderr = stderr.take(SYSTEM_BWRAP_PROBE_STDERR_LIMIT_BYTES);
                    if let Err(err) = stderr.read_to_end(&mut bytes) {
                        if err.kind() != ErrorKind::WouldBlock {
                            return bytes;
                        }
                    }
                    bytes
                });
                let output = Output {
                    status,
                    stdout: Vec::new(),
                    stderr,
                };
                return output.status.success() || !is_user_namespace_failure(&output);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return true;
                }
                thread::sleep(SYSTEM_BWRAP_PROBE_POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

pub(crate) fn is_wsl1() -> bool {
    std::fs::read_to_string("/proc/version")
        .is_ok_and(|proc_version| proc_version_indicates_wsl1(&proc_version))
}

fn proc_version_indicates_wsl1(proc_version: &str) -> bool {
    let proc_version = proc_version.to_ascii_lowercase();
    let mut remaining = proc_version.as_str();
    while let Some(marker) = remaining.find("wsl") {
        let version_start = marker + "wsl".len();
        let version_digits: String = remaining[version_start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(version) = version_digits.parse::<u32>() {
            return version == 1;
        }
        remaining = &remaining[version_start..];
    }

    proc_version.contains("microsoft") && !proc_version.contains("microsoft-standard")
}

fn is_user_namespace_failure(output: &Output) -> bool {
    let stderr = String::from_utf8_lossy(&output.stderr);
    USER_NAMESPACE_FAILURES
        .iter()
        .any(|failure| stderr.contains(failure))
}

/// Search PATH for `bwrap`, skipping any found under the current directory
/// to defend against PATH hijacking.
pub fn find_system_bwrap_in_path() -> Option<PathBuf> {
    let search_path = std::env::var_os("PATH")?;
    let cwd = std::env::current_dir().ok()?;
    find_system_bwrap_in_search_paths(std::env::split_paths(&search_path), &cwd)
}

fn find_system_bwrap_in_search_paths(
    search_paths: impl IntoIterator<Item = PathBuf>,
    cwd: &Path,
) -> Option<PathBuf> {
    let search_path = std::env::join_paths(search_paths).ok()?;
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let cwd_is_root = cwd.parent().is_none();
    let candidates: Vec<PathBuf> = which::which_in_all(SYSTEM_BWRAP_PROGRAM, Some(search_path), &cwd)
        .ok()?
        .collect();
    candidates.into_iter().find_map(|path| {
        let path = std::fs::canonicalize(path).ok()?;
        if !cwd_is_root && path.starts_with(&cwd) {
            None
        } else {
            Some(path)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl1_detected_from_proc_version() {
        assert!(proc_version_indicates_wsl1(
            "Linux version 4.4.0-19041-Microsoft (Microsoft@Microsoft.com) (gcc version 5.4.0) #1-Microsoft Fri Dec 06 14:06:00 PST 2019"
        ));
    }

    #[test]
    fn wsl2_not_detected_as_wsl1() {
        assert!(!proc_version_indicates_wsl1(
            "Linux version 5.15.90.1-microsoft-standard-WSL2 (root@65c757a075e2) (gcc (GCC) 11.2.0)"
        ));
    }

    #[test]
    fn regular_linux_not_detected_as_wsl1() {
        assert!(!proc_version_indicates_wsl1(
            "Linux version 6.8.0-116-generic (buildd@bos03-amd64-006) (x86_64-linux-gnu-gcc-13)"
        ));
    }

    #[test]
    fn explicit_wsl1_version_string() {
        assert!(proc_version_indicates_wsl1("some-kernel WSL1 build"));
    }

    #[test]
    fn user_namespace_failure_detected() {
        for failure in USER_NAMESPACE_FAILURES {
            let output = Output {
                status: std::process::ExitStatus::default(),
                stdout: Vec::new(),
                stderr: failure.as_bytes().to_vec(),
            };
            assert!(is_user_namespace_failure(&output), "should detect: {failure}");
        }
    }

    #[test]
    fn unrelated_stderr_not_treated_as_namespace_failure() {
        let output = Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: b"bwrap: Unknown option --argv0".to_vec(),
        };
        assert!(!is_user_namespace_failure(&output));
    }

    #[test]
    fn missing_bwrap_produces_warning() {
        let warning = system_bwrap_warning_for_path(None);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("bubblewrap"));
    }

    #[test]
    fn find_bwrap_skips_cwd() {
        let tmpdir = tempfile::tempdir().unwrap();
        let fake_bwrap = tmpdir.path().join("bwrap");
        std::fs::write(&fake_bwrap, "#!/bin/sh\ntrue").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_bwrap, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let result = find_system_bwrap_in_search_paths(
            vec![tmpdir.path().to_path_buf()],
            tmpdir.path(),
        );
        assert!(result.is_none(), "should skip bwrap under cwd");
    }
}
