use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use xiaolin_core::path::AbsolutePathBuf;
use xiaolin_core::PROTECTED_METADATA_PATH_NAMES;
use xiaolin_security::{FileSystemSandboxKind, FileSystemSandboxPolicy};

/// High-level network access mode for sandbox policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkAccess {
    Restricted,
    Enabled,
}

impl NetworkAccess {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// A writable root with optional read-only subdirectories.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WritableRoot {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_only_subpaths: Vec<PathBuf>,
}

impl WritableRoot {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            read_only_subpaths: Vec::new(),
        }
    }

    pub fn with_read_only_subpaths(path: PathBuf, read_only_subpaths: Vec<PathBuf>) -> Self {
        Self {
            path,
            read_only_subpaths,
        }
    }

    /// Create from an `AbsolutePathBuf` (type-safe entry point).
    pub fn from_absolute(path: AbsolutePathBuf) -> Self {
        Self::new(path.into_path_buf())
    }

    pub fn is_path_writable(&self, target: &Path) -> bool {
        if !target.starts_with(&self.path) {
            return false;
        }

        let relative = target.strip_prefix(&self.path).unwrap_or(target);

        for protected_name in PROTECTED_METADATA_PATH_NAMES {
            if Self::path_contains_component(relative, protected_name) {
                return false;
            }
        }

        !self
            .read_only_subpaths
            .iter()
            .any(|ro| target.starts_with(self.path.join(ro)))
    }

    fn path_contains_component(path: &Path, component_name: &str) -> bool {
        path.components()
            .any(|c| matches!(c, std::path::Component::Normal(s) if s == component_name))
    }
}

/// High-level sandbox policy that describes the trust and access level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SandboxPolicy {
    DangerFullAccess,
    ReadOnly {
        network: NetworkAccess,
    },
    ExternalSandbox {
        network: NetworkAccess,
    },
    WorkspaceWrite {
        network: NetworkAccess,
        writable_roots: Vec<PathBuf>,
    },
}

impl SandboxPolicy {
    pub fn network_access(&self) -> Option<NetworkAccess> {
        match self {
            Self::DangerFullAccess => None,
            Self::ReadOnly { network }
            | Self::ExternalSandbox { network }
            | Self::WorkspaceWrite { network, .. } => Some(*network),
        }
    }
}

impl std::fmt::Display for SandboxPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DangerFullAccess => write!(f, "danger-full-access"),
            Self::ReadOnly { .. } => write!(f, "read-only"),
            Self::ExternalSandbox { .. } => write!(f, "external-sandbox"),
            Self::WorkspaceWrite { .. } => write!(f, "workspace-write"),
        }
    }
}

impl std::str::FromStr for SandboxPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "danger-full-access" | "danger_full_access" => Ok(Self::DangerFullAccess),
            "read-only" | "read_only" => Ok(Self::ReadOnly {
                network: NetworkAccess::Restricted,
            }),
            "external-sandbox" | "external_sandbox" => Ok(Self::ExternalSandbox {
                network: NetworkAccess::Restricted,
            }),
            "workspace-write" | "workspace_write" => Ok(Self::WorkspaceWrite {
                network: NetworkAccess::Restricted,
                writable_roots: Vec::new(),
            }),
            _ => Err(format!("unknown sandbox policy: {s}")),
        }
    }
}

/// Derive a `SandboxPolicy` from a `FileSystemSandboxPolicy` and network info.
pub fn compatibility_sandbox_policy_from_fs_sandbox(
    fs_policy: &FileSystemSandboxPolicy,
    network: NetworkAccess,
) -> SandboxPolicy {
    match fs_policy.kind {
        FileSystemSandboxKind::Unrestricted => SandboxPolicy::DangerFullAccess,
        FileSystemSandboxKind::ExternalSandbox => SandboxPolicy::ExternalSandbox { network },
        FileSystemSandboxKind::Restricted => {
            if fs_policy.has_full_disk_write_access() {
                SandboxPolicy::DangerFullAccess
            } else {
                let writable_roots: Vec<PathBuf> = fs_policy
                    .entries
                    .iter()
                    .filter(|e| e.access.can_write())
                    .filter_map(|e| match &e.path {
                        xiaolin_security::FileSystemPath::Path { path } => Some(path.to_path_buf()),
                        _ => None,
                    })
                    .collect();

                if writable_roots.is_empty() {
                    SandboxPolicy::ReadOnly { network }
                } else {
                    SandboxPolicy::WorkspaceWrite {
                        network,
                        writable_roots,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_root_basic() {
        let root = WritableRoot::new(PathBuf::from("/home/user/project"));
        assert!(root.is_path_writable(Path::new("/home/user/project/src/main.rs")));
        assert!(!root.is_path_writable(Path::new("/home/other/file.txt")));
    }

    #[test]
    fn writable_root_protects_git_metadata() {
        let root = WritableRoot::new(PathBuf::from("/home/user/project"));
        assert!(!root.is_path_writable(Path::new("/home/user/project/.git/config")));
        assert!(!root.is_path_writable(Path::new("/home/user/project/.hg/store")));
        assert!(!root.is_path_writable(Path::new("/home/user/project/.svn/entries")));
    }

    #[test]
    fn writable_root_read_only_subpaths() {
        let root = WritableRoot::with_read_only_subpaths(
            PathBuf::from("/home/user/project"),
            vec![PathBuf::from("vendor")],
        );
        assert!(root.is_path_writable(Path::new("/home/user/project/src/main.rs")));
        assert!(!root.is_path_writable(Path::new("/home/user/project/vendor/lib.rs")));
    }

    #[test]
    fn sandbox_policy_display_and_parse() {
        let policies = vec![
            ("danger-full-access", SandboxPolicy::DangerFullAccess),
            (
                "read-only",
                SandboxPolicy::ReadOnly {
                    network: NetworkAccess::Restricted,
                },
            ),
            (
                "external-sandbox",
                SandboxPolicy::ExternalSandbox {
                    network: NetworkAccess::Restricted,
                },
            ),
            (
                "workspace-write",
                SandboxPolicy::WorkspaceWrite {
                    network: NetworkAccess::Restricted,
                    writable_roots: Vec::new(),
                },
            ),
        ];
        for (expected_str, policy) in &policies {
            assert_eq!(policy.to_string(), *expected_str);
            let parsed: SandboxPolicy = expected_str.parse().unwrap();
            assert_eq!(&parsed, policy);
        }
    }

    #[test]
    fn sandbox_policy_parse_unknown() {
        let result: Result<SandboxPolicy, _> = "foobar".parse();
        assert!(result.is_err());
    }

    #[test]
    fn network_access_is_enabled() {
        assert!(NetworkAccess::Enabled.is_enabled());
        assert!(!NetworkAccess::Restricted.is_enabled());
    }

    #[test]
    fn compatibility_from_fs_sandbox_unrestricted() {
        let fs = FileSystemSandboxPolicy::unrestricted();
        let policy = compatibility_sandbox_policy_from_fs_sandbox(&fs, NetworkAccess::Enabled);
        assert!(matches!(policy, SandboxPolicy::DangerFullAccess));
    }

    #[test]
    fn compatibility_from_fs_sandbox_restricted_empty() {
        let fs = FileSystemSandboxPolicy::restricted(vec![]);
        let policy = compatibility_sandbox_policy_from_fs_sandbox(&fs, NetworkAccess::Restricted);
        assert!(matches!(
            policy,
            SandboxPolicy::ReadOnly {
                network: NetworkAccess::Restricted
            }
        ));
    }

    #[test]
    fn compatibility_from_fs_sandbox_with_writable() {
        use xiaolin_security::{FileSystemAccessMode, FileSystemPath, FileSystemSandboxEntry};
        let fs = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: AbsolutePathBuf::from_absolute_path("/home/user/project").unwrap(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let policy = compatibility_sandbox_policy_from_fs_sandbox(&fs, NetworkAccess::Enabled);
        match policy {
            SandboxPolicy::WorkspaceWrite {
                network,
                writable_roots,
            } => {
                assert_eq!(network, NetworkAccess::Enabled);
                assert!(writable_roots.contains(&PathBuf::from("/home/user/project")));
            }
            other => panic!("expected WorkspaceWrite, got {other}"),
        }
    }

    #[test]
    fn compatibility_from_fs_sandbox_external() {
        let fs = FileSystemSandboxPolicy::external_sandbox();
        let policy = compatibility_sandbox_policy_from_fs_sandbox(&fs, NetworkAccess::Enabled);
        assert!(matches!(
            policy,
            SandboxPolicy::ExternalSandbox {
                network: NetworkAccess::Enabled
            }
        ));
    }

    #[test]
    fn sandbox_policy_json_roundtrip() {
        let policy = SandboxPolicy::WorkspaceWrite {
            network: NetworkAccess::Enabled,
            writable_roots: vec![PathBuf::from("/tmp")],
        };
        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: SandboxPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);
    }
}
