use dirs::home_dir;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Display, Path, PathBuf};

mod absolutize;

/// A path that is guaranteed to be absolute and normalized (though it is not
/// guaranteed to be canonicalized or exist on the filesystem).
///
/// When deserializing, a base path must be set using [`AbsolutePathBufGuard::new`].
/// If no base path is set, deserialization will fail unless the path is already absolute.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct AbsolutePathBuf(PathBuf);

impl AbsolutePathBuf {
    fn maybe_expand_home_directory(path: &Path) -> PathBuf {
        if let Some(path_str) = path.to_str() {
            if let Some(home) = home_dir() {
                if let Some(rest) = path_str.strip_prefix('~') {
                    if rest.is_empty() {
                        return home;
                    } else if let Some(rest) = rest.strip_prefix('/') {
                        return home.join(rest.trim_start_matches('/'));
                    }
                }
            }
        }
        path.to_path_buf()
    }

    pub fn resolve_path_against_base<P: AsRef<Path>, B: AsRef<Path>>(
        path: P,
        base_path: B,
    ) -> Self {
        let expanded = Self::maybe_expand_home_directory(path.as_ref());
        let expanded = normalize_path_for_platform(&expanded);
        let base_path = normalize_path_for_platform(base_path.as_ref());
        Self(absolutize::absolutize_from(
            expanded.as_ref(),
            base_path.as_ref(),
        ))
    }

    pub fn from_absolute_path<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let expanded = Self::maybe_expand_home_directory(path.as_ref());
        let expanded = normalize_path_for_platform(&expanded);
        Ok(Self(absolutize::absolutize(expanded.as_ref())?))
    }

    pub fn from_absolute_path_checked<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let expanded = Self::maybe_expand_home_directory(path.as_ref());
        let expanded = normalize_path_for_platform(&expanded);
        if !expanded.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("path is not absolute: {}", path.as_ref().display()),
            ));
        }

        Ok(Self(absolutize::absolutize_from(
            expanded.as_ref(),
            Path::new("/"),
        )))
    }

    pub fn current_dir() -> std::io::Result<Self> {
        Self::from_absolute_path(std::env::current_dir()?)
    }

    pub fn relative_to_current_dir<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        Ok(Self::resolve_path_against_base(
            path,
            std::env::current_dir()?,
        ))
    }

    pub fn join<P: AsRef<Path>>(&self, path: P) -> Self {
        Self::resolve_path_against_base(path, &self.0)
    }

    pub fn canonicalize(&self) -> std::io::Result<Self> {
        std::fs::canonicalize(&self.0).map(Self)
    }

    pub fn parent(&self) -> Option<Self> {
        self.0.parent().map(|p| {
            debug_assert!(
                p.is_absolute(),
                "parent of AbsolutePathBuf must be absolute"
            );
            Self(p.to_path_buf())
        })
    }

    pub fn ancestors(&self) -> impl Iterator<Item = Self> + '_ {
        self.0.ancestors().map(|p| {
            debug_assert!(
                p.is_absolute(),
                "ancestor of AbsolutePathBuf must be absolute"
            );
            Self(p.to_path_buf())
        })
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.0.clone()
    }

    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        self.0.to_string_lossy()
    }

    pub fn display(&self) -> Display<'_> {
        self.0.display()
    }
}

fn normalize_path_for_platform(path: &Path) -> Cow<'_, Path> {
    // On non-Windows, no normalization is needed.
    // Windows device path normalization (\\?\ prefix) is a stub for now.
    Cow::Borrowed(path)
}

/// Canonicalize a path when possible, but preserve the logical absolute path
/// whenever canonicalization would rewrite it through a nested symlink.
pub fn canonicalize_preserving_symlinks(path: &Path) -> std::io::Result<PathBuf> {
    let logical = AbsolutePathBuf::from_absolute_path(path)?.into_path_buf();
    let preserve_logical_path = should_preserve_logical_path(&logical);
    match std::fs::canonicalize(path) {
        Ok(canonical) if preserve_logical_path && canonical != logical => Ok(logical),
        Ok(canonical) => Ok(canonical),
        Err(_) => Ok(logical),
    }
}

/// Canonicalize an existing path while preserving the logical absolute path
/// whenever canonicalization would rewrite it through a nested symlink.
pub fn canonicalize_existing_preserving_symlinks(path: &Path) -> std::io::Result<PathBuf> {
    let logical = AbsolutePathBuf::from_absolute_path(path)?.into_path_buf();
    let canonical = std::fs::canonicalize(path)?;
    if should_preserve_logical_path(&logical) && canonical != logical {
        Ok(logical)
    } else {
        Ok(canonical)
    }
}

fn should_preserve_logical_path(logical: &Path) -> bool {
    logical.ancestors().any(|ancestor| {
        let Ok(metadata) = std::fs::symlink_metadata(ancestor) else {
            return false;
        };
        metadata.file_type().is_symlink() && ancestor.parent().and_then(Path::parent).is_some()
    })
}

impl AsRef<Path> for AbsolutePathBuf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for AbsolutePathBuf {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<AbsolutePathBuf> for PathBuf {
    fn from(path: AbsolutePathBuf) -> Self {
        path.into_path_buf()
    }
}

impl TryFrom<&Path> for AbsolutePathBuf {
    type Error = std::io::Error;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        Self::from_absolute_path(value)
    }
}

impl TryFrom<PathBuf> for AbsolutePathBuf {
    type Error = std::io::Error;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        Self::from_absolute_path(value)
    }
}

impl TryFrom<&str> for AbsolutePathBuf {
    type Error = std::io::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_absolute_path(value)
    }
}

impl TryFrom<String> for AbsolutePathBuf {
    type Error = std::io::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_absolute_path(value)
    }
}

thread_local! {
    static ABSOLUTE_PATH_BASE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Hold this guard while deserializing `AbsolutePathBuf` values to provide a
/// base path for resolving relative paths. Uses thread-local storage, so
/// deserialization must happen on the same thread.
pub struct AbsolutePathBufGuard;

impl AbsolutePathBufGuard {
    pub fn new(base_path: &Path) -> Self {
        ABSOLUTE_PATH_BASE.with(|cell| {
            *cell.borrow_mut() = Some(base_path.to_path_buf());
        });
        Self
    }
}

impl Drop for AbsolutePathBufGuard {
    fn drop(&mut self) {
        ABSOLUTE_PATH_BASE.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

impl<'de> Deserialize<'de> for AbsolutePathBuf {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = PathBuf::deserialize(deserializer)?;
        ABSOLUTE_PATH_BASE.with(|cell| match cell.borrow().as_deref() {
            Some(base) => Ok(Self::resolve_path_against_base(path, base)),
            None if path.is_absolute() => {
                Self::from_absolute_path(path).map_err(serde::de::Error::custom)
            }
            None => Err(serde::de::Error::custom(
                "AbsolutePathBuf deserialized without a base path",
            )),
        })
    }
}

/// Helpers for constructing absolute paths in tests.
pub mod test_support {
    use super::AbsolutePathBuf;
    use std::path::{Path, PathBuf};

    /// Creates a platform-absolute [`PathBuf`] from a Unix-style absolute test path.
    /// On Windows, `/tmp/example` maps to `C:\tmp\example`.
    pub fn test_path_buf(unix_path: &str) -> PathBuf {
        if cfg!(windows) {
            let mut path = PathBuf::from(r"C:\");
            path.extend(
                unix_path
                    .trim_start_matches('/')
                    .split('/')
                    .filter(|segment| !segment.is_empty()),
            );
            path
        } else {
            PathBuf::from(unix_path)
        }
    }

    pub trait PathExt {
        fn abs(&self) -> AbsolutePathBuf;
    }

    impl PathExt for Path {
        fn abs(&self) -> AbsolutePathBuf {
            AbsolutePathBuf::from_absolute_path_checked(self)
                .expect("path should already be absolute")
        }
    }

    pub trait PathBufExt {
        fn abs(&self) -> AbsolutePathBuf;
    }

    impl PathBufExt for PathBuf {
        fn abs(&self) -> AbsolutePathBuf {
            self.as_path().abs()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_path_buf;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn create_with_absolute_path_ignores_base_path() {
        let base_dir = tempdir().expect("base dir");
        let absolute_dir = tempdir().expect("absolute dir");
        let base_path = base_dir.path();
        let absolute_path = absolute_dir.path().join("file.txt");
        let abs_path_buf =
            AbsolutePathBuf::resolve_path_against_base(absolute_path.clone(), base_path);
        assert_eq!(abs_path_buf.as_path(), absolute_path.as_path());
    }

    #[test]
    fn from_absolute_path_checked_rejects_relative_path() {
        let err = AbsolutePathBuf::from_absolute_path_checked("relative/path")
            .expect_err("relative path should fail");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn relative_path_is_resolved_against_base_path() {
        let temp_dir = tempdir().expect("base dir");
        let base_dir = temp_dir.path();
        let abs_path_buf = AbsolutePathBuf::resolve_path_against_base("file.txt", base_dir);
        assert_eq!(abs_path_buf.as_path(), base_dir.join("file.txt").as_path());
    }

    #[test]
    fn relative_path_dots_are_normalized_against_base_path() {
        let temp_dir = tempdir().expect("base dir");
        let base_dir = temp_dir.path();
        let abs_path_buf =
            AbsolutePathBuf::resolve_path_against_base("./nested/../file.txt", base_dir);
        assert_eq!(abs_path_buf.as_path(), base_dir.join("file.txt").as_path());
    }

    #[test]
    fn canonicalize_returns_absolute_path_buf() {
        let temp_dir = tempdir().expect("base dir");
        fs::create_dir(temp_dir.path().join("one")).expect("create one dir");
        fs::create_dir(temp_dir.path().join("two")).expect("create two dir");
        fs::write(temp_dir.path().join("two").join("file.txt"), "").expect("write file");
        let abs_path_buf =
            AbsolutePathBuf::from_absolute_path(temp_dir.path().join("one/../two/./file.txt"))
                .expect("absolute path");
        assert!(abs_path_buf.canonicalize().is_ok());
    }

    #[test]
    fn canonicalize_returns_error_for_missing_path() {
        let temp_dir = tempdir().expect("base dir");
        let abs_path_buf = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("missing.txt"))
            .expect("absolute path");
        assert!(abs_path_buf.canonicalize().is_err());
    }

    #[test]
    fn ancestors_returns_absolute_path_bufs() {
        let abs_path_buf =
            AbsolutePathBuf::from_absolute_path_checked(test_path_buf("/tmp/one/two"))
                .expect("absolute path");
        let ancestors: Vec<_> = abs_path_buf
            .ancestors()
            .map(|p| p.to_path_buf())
            .collect();
        let expected = vec![
            test_path_buf("/tmp/one/two"),
            test_path_buf("/tmp/one"),
            test_path_buf("/tmp"),
            test_path_buf("/"),
        ];
        assert_eq!(ancestors, expected);
    }

    #[test]
    fn relative_to_current_dir_resolves_relative_path() {
        let current_dir = std::env::current_dir().unwrap();
        let abs_path_buf = AbsolutePathBuf::relative_to_current_dir("file.txt").unwrap();
        assert_eq!(
            abs_path_buf.as_path(),
            current_dir.join("file.txt").as_path()
        );
    }

    #[test]
    fn guard_used_in_deserialization() {
        let temp_dir = tempdir().expect("base dir");
        let base_dir = temp_dir.path();
        let relative_path = "subdir/file.txt";
        let abs_path_buf = {
            let _guard = AbsolutePathBufGuard::new(base_dir);
            serde_json::from_str::<AbsolutePathBuf>(&format!(r#""{relative_path}""#))
                .expect("failed to deserialize")
        };
        assert_eq!(
            abs_path_buf.as_path(),
            base_dir.join(relative_path).as_path()
        );
    }

    #[test]
    fn home_directory_root_is_expanded_in_deserialization() {
        let Some(home) = home_dir() else { return };
        let temp_dir = tempdir().expect("base dir");
        let abs_path_buf = {
            let _guard = AbsolutePathBufGuard::new(temp_dir.path());
            serde_json::from_str::<AbsolutePathBuf>("\"~\"").expect("failed to deserialize")
        };
        assert_eq!(abs_path_buf.as_path(), home.as_path());
    }

    #[test]
    fn home_directory_subpath_is_expanded_in_deserialization() {
        let Some(home) = home_dir() else { return };
        let temp_dir = tempdir().expect("base dir");
        let abs_path_buf = {
            let _guard = AbsolutePathBufGuard::new(temp_dir.path());
            serde_json::from_str::<AbsolutePathBuf>("\"~/code\"").expect("failed to deserialize")
        };
        assert_eq!(abs_path_buf.as_path(), home.join("code").as_path());
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_preserving_symlinks_keeps_logical_symlink_path() {
        let temp_dir = tempdir().expect("temp dir");
        let real = temp_dir.path().join("real");
        let link = temp_dir.path().join("link");
        fs::create_dir_all(&real).expect("create real dir");
        std::os::unix::fs::symlink(&real, &link).expect("create symlink");
        let canonicalized =
            canonicalize_preserving_symlinks(&link).expect("canonicalize preserving symlinks");
        assert_eq!(canonicalized, link);
    }

    #[test]
    fn canonicalize_existing_preserving_symlinks_errors_for_missing_path() {
        let temp_dir = tempdir().expect("temp dir");
        let missing = temp_dir.path().join("missing");
        let err = canonicalize_existing_preserving_symlinks(&missing)
            .expect_err("missing path should fail canonicalization");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn join_resolves_relative() {
        let base = AbsolutePathBuf::from_absolute_path_checked(test_path_buf("/home/user"))
            .expect("abs path");
        let joined = base.join("project/file.txt");
        assert_eq!(
            joined.as_path(),
            test_path_buf("/home/user/project/file.txt")
        );
    }

    #[test]
    fn parent_returns_parent() {
        let p = AbsolutePathBuf::from_absolute_path_checked(test_path_buf("/home/user"))
            .expect("abs path");
        let parent = p.parent().expect("has parent");
        assert_eq!(parent.as_path(), test_path_buf("/home"));
    }

    #[test]
    fn try_from_impls() {
        let from_str: AbsolutePathBuf = "/tmp".try_into().expect("from &str");
        assert_eq!(from_str.as_path(), Path::new("/tmp"));

        let from_string: AbsolutePathBuf = "/tmp".to_string().try_into().expect("from String");
        assert_eq!(from_string.as_path(), Path::new("/tmp"));

        let from_path: AbsolutePathBuf = Path::new("/tmp").try_into().expect("from &Path");
        assert_eq!(from_path.as_path(), Path::new("/tmp"));

        let from_pathbuf: AbsolutePathBuf =
            PathBuf::from("/tmp").try_into().expect("from PathBuf");
        assert_eq!(from_pathbuf.as_path(), Path::new("/tmp"));
    }

    #[test]
    fn into_pathbuf_conversion() {
        let abs = AbsolutePathBuf::from_absolute_path_checked(test_path_buf("/tmp"))
            .expect("abs path");
        let pb: PathBuf = abs.into();
        assert_eq!(pb, test_path_buf("/tmp"));
    }

    #[test]
    fn display_and_to_string_lossy() {
        let abs = AbsolutePathBuf::from_absolute_path_checked(test_path_buf("/tmp/file"))
            .expect("abs path");
        let display = format!("{}", abs.display());
        assert!(display.contains("tmp"));
        assert!(abs.to_string_lossy().contains("tmp"));
    }
}
