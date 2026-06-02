#![allow(unsafe_code)]

/// Apply process hardening before normal initialization begins.
///
/// This should be called as early as possible in `main()`, before any
/// security-sensitive work. It performs platform-specific steps:
///
/// - **Linux**: `PR_SET_DUMPABLE=0`, `RLIMIT_CORE=0`, remove `LD_*` env vars
/// - **macOS**: `PT_DENY_ATTACH`, `RLIMIT_CORE=0`, remove `DYLD_*` env vars
/// - **Windows**: (reserved for future use)
pub fn pre_main_hardening() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    harden_linux();

    #[cfg(target_os = "macos")]
    harden_macos();

    #[cfg(any(target_os = "freebsd", target_os = "openbsd"))]
    harden_bsd();

    #[cfg(windows)]
    harden_windows();
}

/// Mark the current Linux process non-dumpable so same-user processes
/// cannot attach with ptrace.
#[cfg(target_os = "linux")]
pub fn disable_process_dumping() -> std::io::Result<()> {
    let ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "linux", target_os = "android"))]
fn harden_linux() {
    let ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if ret != 0 {
        eprintln!(
            "ERROR: prctl(PR_SET_DUMPABLE, 0) failed: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(5);
    }

    set_core_limit_zero();
    remove_env_vars_with_prefix(b"LD_");
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn harden_macos() {
    let ret = unsafe { libc::ptrace(libc::PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0) };
    if ret == -1 {
        eprintln!(
            "ERROR: ptrace(PT_DENY_ATTACH) failed: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(6);
    }

    set_core_limit_zero();
    remove_env_vars_with_prefix(b"DYLD_");
    remove_env_vars_with_prefix(b"MallocStackLogging");
    remove_env_vars_with_prefix(b"MallocLogFile");
}

// ---------------------------------------------------------------------------
// BSD
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "freebsd", target_os = "openbsd"))]
fn harden_bsd() {
    set_core_limit_zero();
    remove_env_vars_with_prefix(b"LD_");
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn harden_windows() {
    // Reserved for future Windows hardening (SetProcessMitigationPolicy etc.)
}

// ---------------------------------------------------------------------------
// Shared Unix helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn set_core_limit_zero() {
    let rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &raw const rlim) };
    if ret != 0 {
        eprintln!(
            "ERROR: setrlimit(RLIMIT_CORE) failed: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(7);
    }
}

#[cfg(unix)]
fn remove_env_vars_with_prefix(prefix: &[u8]) {
    use std::os::unix::ffi::OsStrExt;

    let keys: Vec<_> = std::env::vars_os()
        .filter_map(|(key, _)| {
            key.as_os_str()
                .as_bytes()
                .starts_with(prefix)
                .then_some(key)
        })
        .collect();

    for key in keys {
        unsafe { std::env::remove_var(key) };
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn pre_main_hardening_does_not_panic() {
        super::pre_main_hardening();
    }

    #[test]
    fn remove_env_vars_with_prefix_filters_correctly() {
        let test_var = "LD_XIAOLIN_TEST_VAR";
        unsafe { std::env::set_var(test_var, "1") };
        assert!(std::env::var(test_var).is_ok());

        super::remove_env_vars_with_prefix(b"LD_XIAOLIN_TEST");
        assert!(std::env::var(test_var).is_err());
    }

    #[test]
    fn remove_env_vars_ignores_non_matching() {
        let safe_var = "XIAOLIN_SAFE_TEST_VAR";
        unsafe { std::env::set_var(safe_var, "ok") };

        super::remove_env_vars_with_prefix(b"LD_");
        assert_eq!(std::env::var(safe_var).unwrap(), "ok");
        unsafe { std::env::remove_var(safe_var) };
    }

    #[test]
    fn prefix_match_is_byte_level() {
        let key = OsStr::from_bytes(b"LD_PRELOAD");
        assert!(key.as_bytes().starts_with(b"LD_"));
    }

    #[test]
    fn remove_env_vars_handles_non_utf8_keys() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let non_utf8_key = OsString::from_vec(vec![b'L', b'D', b'_', 0xF0]);
        assert!(non_utf8_key.clone().into_string().is_err());

        unsafe { std::env::set_var(&non_utf8_key, "1") };
        assert!(std::env::var_os(&non_utf8_key).is_some());

        super::remove_env_vars_with_prefix(b"LD_");
        assert!(
            std::env::var_os(&non_utf8_key).is_none(),
            "non-UTF-8 LD_ key should be removed"
        );
    }

    #[test]
    fn non_matching_non_utf8_key_is_preserved() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let key = OsString::from_vec(vec![b'R', 0xD6, b'D', b'B', b'U', b'R', b'K']);
        assert!(key.clone().into_string().is_err());

        unsafe { std::env::set_var(&key, "val") };
        super::remove_env_vars_with_prefix(b"LD_");
        assert!(
            std::env::var_os(&key).is_some(),
            "non-LD_ non-UTF-8 key should survive"
        );
        unsafe { std::env::remove_var(&key) };
    }
}
