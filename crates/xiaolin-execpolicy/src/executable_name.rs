use std::path::Path;

#[cfg(windows)]
const WINDOWS_EXECUTABLE_SUFFIXES: [&str; 4] = [".exe", ".cmd", ".bat", ".com"];

/// Convert a raw executable name to a canonical lookup key.
///
/// On Windows, strips known executable suffixes (`.exe`, `.cmd`, `.bat`,
/// `.com`) so that rules written for `git` also match `git.exe`.
pub fn executable_lookup_key(raw: &str) -> String {
    #[cfg(windows)]
    {
        let raw = raw.to_ascii_lowercase();
        for suffix in WINDOWS_EXECUTABLE_SUFFIXES {
            if raw.ends_with(suffix) {
                let stripped_len = raw.len() - suffix.len();
                return raw[..stripped_len].to_string();
            }
        }
        raw
    }

    #[cfg(not(windows))]
    {
        raw.to_string()
    }
}

/// Extract the basename from an absolute path and convert it to a lookup key.
///
/// Returns `None` if the path has no file name component or is not valid UTF-8.
pub fn executable_path_lookup_key(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(executable_lookup_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn lookup_key_preserves_name() {
        assert_eq!(executable_lookup_key("git"), "git");
        assert_eq!(executable_lookup_key("bash"), "bash");
    }

    #[test]
    fn path_lookup_key_extracts_basename() {
        assert_eq!(
            executable_path_lookup_key(&PathBuf::from("/usr/bin/git")),
            Some("git".to_string())
        );
        assert_eq!(
            executable_path_lookup_key(&PathBuf::from("/usr/local/bin/python3")),
            Some("python3".to_string())
        );
    }

    #[test]
    fn path_lookup_key_returns_none_for_root() {
        assert_eq!(executable_path_lookup_key(&PathBuf::from("/")), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_strips_exe_suffix() {
        assert_eq!(executable_lookup_key("git.exe"), "git");
        assert_eq!(executable_lookup_key("npm.cmd"), "npm");
        assert_eq!(executable_lookup_key("run.bat"), "run");
        assert_eq!(executable_lookup_key("tool.com"), "tool");
    }
}
