use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use xiaolin_core::path::AbsolutePathBuf;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Protected metadata constants (canonical list lives in xiaolin-core)
// ---------------------------------------------------------------------------

/// Re-export of [`xiaolin_core::PROTECTED_METADATA_PATH_NAMES`].
pub use xiaolin_core::PROTECTED_METADATA_PATH_NAMES;

const PROJECT_ROOTS_GLOB_PATTERN_PREFIX: &str = "xiaolin-project-roots://";

/// Returns true when a path basename is one of the protected workspace metadata names.
pub fn is_protected_metadata_name(name: &OsStr) -> bool {
    PROTECTED_METADATA_PATH_NAMES
        .iter()
        .any(|metadata_name| name == OsStr::new(metadata_name))
}

/// Build a glob pattern string that uses the project-roots prefix convention.
pub fn project_roots_glob_pattern(subpath: &Path) -> String {
    format!("{PROJECT_ROOTS_GLOB_PATTERN_PREFIX}{}", subpath.display())
}

/// Returns the protected workspace metadata name when an agent write to `path`
/// should be blocked before execution.
pub fn forbidden_agent_metadata_write(
    path: &Path,
    cwd: &Path,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> Option<&'static str> {
    if !matches!(
        file_system_sandbox_policy.kind,
        FileSystemSandboxKind::Restricted
    ) {
        return None;
    }

    let target = resolve_candidate_path(path, cwd)?;
    let (_, metadata_name) =
        metadata_child_of_writable_root(file_system_sandbox_policy, target.as_path(), cwd)?;

    if !file_system_sandbox_policy.can_write_path_with_cwd(target.as_path(), cwd) {
        return Some(metadata_name);
    }

    None
}

// ---------------------------------------------------------------------------
// Network sandbox policy — aligned with Codex protocol/src/permissions.rs:83-93
// ---------------------------------------------------------------------------

/// Whether network access is restricted or enabled at the sandbox level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkSandboxPolicy {
    #[default]
    Restricted,
    Enabled,
}

impl NetworkSandboxPolicy {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

// ---------------------------------------------------------------------------
// SandboxEnforcement & PermissionProfile — runtime semantic wrappers
// ---------------------------------------------------------------------------

/// How filesystem sandbox enforcement is handled.
///
/// This maps to Codex's `SandboxEnforcement`: distinguishing whether
/// XiaoLin manages the sandbox itself, an external caller does, or
/// sandboxing is disabled entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEnforcement {
    /// XiaoLin owns sandbox construction (Landlock, Seatbelt, etc.).
    #[default]
    Managed,
    /// No outer filesystem sandbox should be applied.
    Disabled,
    /// Filesystem isolation is enforced by an external caller.
    External,
}

impl SandboxEnforcement {
    pub fn from_fs_sandbox_kind(kind: FileSystemSandboxKind) -> Self {
        match kind {
            FileSystemSandboxKind::Restricted | FileSystemSandboxKind::Unrestricted => {
                Self::Managed
            }
            FileSystemSandboxKind::ExternalSandbox => Self::External,
        }
    }
}

/// Filesystem permissions for profiles where XiaoLin manages the sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManagedFileSystemPermissions {
    Restricted {
        entries: Vec<FileSystemSandboxEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        glob_scan_max_depth: Option<usize>,
    },
    Unrestricted,
}

impl ManagedFileSystemPermissions {
    pub fn from_fs_sandbox_policy(policy: &FileSystemSandboxPolicy) -> Self {
        match policy.kind {
            FileSystemSandboxKind::Restricted => Self::Restricted {
                entries: policy.entries.clone(),
                glob_scan_max_depth: policy.glob_scan_max_depth,
            },
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                Self::Unrestricted
            }
        }
    }

    pub fn to_fs_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        match self {
            Self::Restricted {
                entries,
                glob_scan_max_depth,
            } => FileSystemSandboxPolicy {
                kind: FileSystemSandboxKind::Restricted,
                glob_scan_max_depth: *glob_scan_max_depth,
                entries: entries.clone(),
            },
            Self::Unrestricted => FileSystemSandboxPolicy::unrestricted(),
        }
    }
}

/// High-level runtime permission profile combining enforcement mode,
/// filesystem rules, and network policy.
///
/// Maps to Codex's `PermissionProfile` enum (Managed/Disabled/External).
/// Use this for runtime decision-making about what level of sandboxing
/// to apply, and convert to/from `(FileSystemSandboxPolicy, NetworkSandboxPolicy)`
/// for the actual enforcement layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionProfile {
    /// XiaoLin owns sandbox construction for this profile.
    Managed {
        file_system: ManagedFileSystemPermissions,
        network: NetworkSandboxPolicy,
    },
    /// Do not apply an outer sandbox.
    Disabled,
    /// Filesystem isolation is enforced by an external caller.
    External {
        network: NetworkSandboxPolicy,
    },
}

impl PermissionProfile {
    /// Extract the `SandboxEnforcement` mode from this profile.
    pub fn enforcement(&self) -> SandboxEnforcement {
        match self {
            Self::Managed { .. } => SandboxEnforcement::Managed,
            Self::Disabled => SandboxEnforcement::Disabled,
            Self::External { .. } => SandboxEnforcement::External,
        }
    }

    /// Extract the `NetworkSandboxPolicy` from this profile.
    pub fn network_sandbox_policy(&self) -> NetworkSandboxPolicy {
        match self {
            Self::Managed { network, .. } => *network,
            Self::Disabled => NetworkSandboxPolicy::Enabled,
            Self::External { network } => *network,
        }
    }

    /// Extract the `FileSystemSandboxPolicy` from this profile.
    pub fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        match self {
            Self::Managed { file_system, .. } => file_system.to_fs_sandbox_policy(),
            Self::Disabled => FileSystemSandboxPolicy::unrestricted(),
            Self::External { .. } => FileSystemSandboxPolicy::external_sandbox(),
        }
    }

    /// Convert to the runtime permission pair.
    pub fn to_runtime_permissions(&self) -> (FileSystemSandboxPolicy, NetworkSandboxPolicy) {
        (
            self.file_system_sandbox_policy(),
            self.network_sandbox_policy(),
        )
    }

    /// Construct a `PermissionProfile` from runtime permission types.
    pub fn from_runtime_permissions(
        fs_policy: &FileSystemSandboxPolicy,
        net_policy: NetworkSandboxPolicy,
    ) -> Self {
        Self::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::from_fs_sandbox_kind(fs_policy.kind),
            fs_policy,
            net_policy,
        )
    }

    /// Construct with an explicit enforcement mode override.
    pub fn from_runtime_permissions_with_enforcement(
        enforcement: SandboxEnforcement,
        fs_policy: &FileSystemSandboxPolicy,
        net_policy: NetworkSandboxPolicy,
    ) -> Self {
        match fs_policy.kind {
            FileSystemSandboxKind::ExternalSandbox => Self::External {
                network: net_policy,
            },
            FileSystemSandboxKind::Unrestricted
                if enforcement == SandboxEnforcement::Disabled =>
            {
                Self::Disabled
            }
            FileSystemSandboxKind::Restricted | FileSystemSandboxKind::Unrestricted => {
                Self::Managed {
                    file_system: ManagedFileSystemPermissions::from_fs_sandbox_policy(fs_policy),
                    network: net_policy,
                }
            }
        }
    }

    /// Read-only preset: Root(Read) + Restricted network.
    pub fn read_only() -> Self {
        Self::Managed {
            file_system: ManagedFileSystemPermissions::Restricted {
                entries: vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::Root,
                    },
                    access: FileSystemAccessMode::Read,
                }],
                glob_scan_max_depth: None,
            },
            network: NetworkSandboxPolicy::Restricted,
        }
    }

    /// Workspace-write preset: ProjectRoots(Write) + Tmpdir(Write) +
    /// SlashTmp(Write) + Root(Read) + Restricted network.
    pub fn workspace_write() -> Self {
        Self::workspace_write_with(Vec::new(), NetworkSandboxPolicy::Restricted, false, false)
    }

    /// Workspace-write preset with explicit parameters.
    pub fn workspace_write_with(
        extra_writable_roots: Vec<FileSystemSandboxEntry>,
        network: NetworkSandboxPolicy,
        exclude_tmpdir: bool,
        exclude_slash_tmp: bool,
    ) -> Self {
        let mut entries = vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::ProjectRoots { subpath: None },
            },
            access: FileSystemAccessMode::Write,
        }];
        if !exclude_tmpdir {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Tmpdir,
                },
                access: FileSystemAccessMode::Write,
            });
        }
        if !exclude_slash_tmp {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::SlashTmp,
                },
                access: FileSystemAccessMode::Write,
            });
        }
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        });
        entries.extend(extra_writable_roots);

        Self::Managed {
            file_system: ManagedFileSystemPermissions::Restricted {
                entries,
                glob_scan_max_depth: None,
            },
            network,
        }
    }

    /// Construct from a legacy `SandboxPolicy` string label.
    pub fn from_legacy_sandbox_policy(label: &str) -> Option<Self> {
        match label {
            "read-only" => Some(Self::read_only()),
            "workspace-write" => Some(Self::workspace_write()),
            "full-access" | "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    /// Convert back to a legacy sandbox policy label.
    pub fn to_legacy_sandbox_policy(&self) -> &'static str {
        match self {
            Self::Managed {
                file_system: ManagedFileSystemPermissions::Restricted { entries, .. },
                network: NetworkSandboxPolicy::Restricted,
            } => {
                let has_project_roots_write = entries.iter().any(|e| {
                    matches!(
                        &e.path,
                        FileSystemPath::Special {
                            value: FileSystemSpecialPath::ProjectRoots { .. }
                        }
                    ) && e.access == FileSystemAccessMode::Write
                });
                if has_project_roots_write {
                    "workspace-write"
                } else {
                    "read-only"
                }
            }
            Self::Managed {
                file_system: ManagedFileSystemPermissions::Restricted { .. },
                ..
            } => "read-only",
            Self::Managed {
                file_system: ManagedFileSystemPermissions::Unrestricted,
                ..
            } => "full-access",
            Self::Disabled => "disabled",
            Self::External { .. } => "read-only",
        }
    }

    /// Returns true if platform sandboxing is actually needed.
    pub fn needs_direct_runtime_enforcement(&self) -> bool {
        match self {
            Self::Managed { .. } => true,
            Self::Disabled => false,
            Self::External { .. } => false,
        }
    }
}

// ---------------------------------------------------------------------------
// WritableRoot
// ---------------------------------------------------------------------------

/// A writable root directory resolved from a permission profile.
///
/// Supports fine-grained control: read-only subpaths within the writable root
/// and protected metadata directories (e.g. `.git/hooks`) that must not be
/// overwritten unless the policy explicitly grants write access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableRoot {
    pub root: AbsolutePathBuf,

    /// Subpaths under `root` that are read-only despite the root being writable.
    pub read_only_subpaths: Vec<AbsolutePathBuf>,

    /// Workspace metadata path names (e.g. `.git`, `.xiaolin`) that must not
    /// be created or replaced under `root` unless the policy grants an explicit
    /// write rule for that metadata path.
    pub protected_metadata_names: Vec<String>,
}

impl WritableRoot {
    pub fn new(root: AbsolutePathBuf) -> Self {
        Self {
            root,
            read_only_subpaths: Vec::new(),
            protected_metadata_names: PROTECTED_METADATA_PATH_NAMES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    pub fn with_read_only_subpaths(
        root: AbsolutePathBuf,
        read_only_subpaths: Vec<AbsolutePathBuf>,
    ) -> Self {
        Self {
            root,
            read_only_subpaths,
            protected_metadata_names: PROTECTED_METADATA_PATH_NAMES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    /// Check whether a given path is writable under this root.
    pub fn is_path_writable(&self, path: &Path) -> bool {
        if !path.starts_with(&self.root) {
            return false;
        }

        for subpath in &self.read_only_subpaths {
            if path.starts_with(subpath.as_path()) {
                return false;
            }
        }

        if self.path_contains_protected_metadata_name(path) {
            return false;
        }

        true
    }

    fn path_contains_protected_metadata_name(&self, path: &Path) -> bool {
        let Ok(relative_path) = path.strip_prefix(&self.root) else {
            return false;
        };

        let Some(first_component) = relative_path.components().next() else {
            return false;
        };

        self.protected_metadata_names
            .iter()
            .any(|name| first_component.as_os_str() == OsStr::new(name))
    }
}

/// Active permission profile identity, tracking which named profile is in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivePermissionProfile {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
}

impl ActivePermissionProfile {
    pub fn read_only() -> Self {
        Self {
            id: "read-only".to_string(),
            extends: None,
        }
    }

    pub fn workspace_write() -> Self {
        Self {
            id: "workspace-write".to_string(),
            extends: None,
        }
    }

    pub fn custom(id: impl Into<String>, extends: Option<String>) -> Self {
        Self {
            id: id.into(),
            extends,
        }
    }
}

// ---------------------------------------------------------------------------
// AdditionalPermissionProfile — for approval flows
// ---------------------------------------------------------------------------

/// Additional filesystem permissions that may be granted on top of a
/// base `PermissionProfile` (e.g. via a tool-use approval).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FileSystemPermissions {
    #[serde(default)]
    pub entries: Vec<FileSystemSandboxEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_scan_max_depth: Option<std::num::NonZeroUsize>,
}

impl FileSystemPermissions {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn from_read_write_roots(
        read_roots: &[AbsolutePathBuf],
        write_roots: &[AbsolutePathBuf],
    ) -> Self {
        let mut entries: Vec<FileSystemSandboxEntry> = read_roots
            .iter()
            .map(|p| FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: p.clone() },
                access: FileSystemAccessMode::Read,
            })
            .collect();
        entries.extend(write_roots.iter().map(|p| FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: p.clone() },
            access: FileSystemAccessMode::Write,
        }));
        Self {
            entries,
            glob_scan_max_depth: None,
        }
    }

    pub fn explicit_path_entries(&self) -> impl Iterator<Item = &FileSystemSandboxEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(&e.path, FileSystemPath::Path { .. }))
    }
}

/// Additional network permissions that may be granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NetworkPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl NetworkPermissions {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
    }
}

/// A bundle of additional permissions that can be applied on top of a
/// base `PermissionProfile`. Used by approval/tool-use flows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AdditionalPermissionProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_system: Option<FileSystemPermissions>,
}

impl AdditionalPermissionProfile {
    pub fn is_empty(&self) -> bool {
        self.network.as_ref().is_none_or(|n| n.is_empty())
            && self.file_system.as_ref().is_none_or(|f| f.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Rich type system — aligned with Codex protocol/src/permissions.rs
// ---------------------------------------------------------------------------

/// Access mode for a filesystem entry. Conflict precedence: None > Write > Read.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileSystemAccessMode {
    Read,
    Write,
    None,
}

impl FileSystemAccessMode {
    pub fn can_read(self) -> bool {
        !matches!(self, Self::None)
    }

    pub fn can_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// Symbolic filesystem path tokens resolved at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileSystemSpecialPath {
    Root,
    Minimal,
    #[serde(alias = "current_working_directory")]
    ProjectRoots {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subpath: Option<PathBuf>,
    },
    Tmpdir,
    SlashTmp,
    Unknown {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subpath: Option<PathBuf>,
    },
}

impl FileSystemSpecialPath {
    pub fn project_roots(subpath: Option<PathBuf>) -> Self {
        Self::ProjectRoots { subpath }
    }

    pub fn unknown(path: impl Into<String>, subpath: Option<PathBuf>) -> Self {
        Self::Unknown {
            path: path.into(),
            subpath,
        }
    }
}

/// A filesystem path specification: concrete path, glob, or special token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileSystemPath {
    Path { path: AbsolutePathBuf },
    GlobPattern { pattern: String },
    Special { value: FileSystemSpecialPath },
}

/// A single sandbox entry combining a path specification with an access mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileSystemSandboxEntry {
    pub path: FileSystemPath,
    pub access: FileSystemAccessMode,
}

/// Whether the sandbox restricts, is unrestricted, or defers to an external sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FileSystemSandboxKind {
    #[default]
    Restricted,
    Unrestricted,
    ExternalSandbox,
}

/// Rich filesystem sandbox policy with entry-level granularity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSystemSandboxPolicy {
    pub kind: FileSystemSandboxKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_scan_max_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<FileSystemSandboxEntry>,
}

impl Default for FileSystemSandboxPolicy {
    fn default() -> Self {
        Self {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: None,
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            }],
        }
    }
}

impl FileSystemSandboxPolicy {
    pub fn unrestricted() -> Self {
        Self {
            kind: FileSystemSandboxKind::Unrestricted,
            glob_scan_max_depth: None,
            entries: Vec::new(),
        }
    }

    pub fn external_sandbox() -> Self {
        Self {
            kind: FileSystemSandboxKind::ExternalSandbox,
            glob_scan_max_depth: None,
            entries: Vec::new(),
        }
    }

    pub fn restricted(entries: Vec<FileSystemSandboxEntry>) -> Self {
        Self {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: None,
            entries,
        }
    }

    pub fn has_full_disk_read_access(&self) -> bool {
        match self.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => true,
            FileSystemSandboxKind::Restricted => {
                self.has_root_access(FileSystemAccessMode::can_read)
                    && !self.has_denied_read_restrictions()
            }
        }
    }

    pub fn has_full_disk_write_access(&self) -> bool {
        match self.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => true,
            FileSystemSandboxKind::Restricted => {
                self.has_root_access(FileSystemAccessMode::can_write)
                    && !self.has_write_narrowing_entries()
            }
        }
    }

    pub fn has_denied_read_restrictions(&self) -> bool {
        matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self
                .entries
                .iter()
                .any(|entry| entry.access == FileSystemAccessMode::None)
    }

    fn has_root_access(&self, predicate: impl Fn(FileSystemAccessMode) -> bool) -> bool {
        matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self.entries.iter().any(|entry| {
                matches!(
                    &entry.path,
                    FileSystemPath::Special { value }
                        if matches!(value, FileSystemSpecialPath::Root) && predicate(entry.access)
                )
            })
    }
}

// ---------------------------------------------------------------------------
// Internal helper type for resolved entries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFileSystemEntry {
    path: AbsolutePathBuf,
    access: FileSystemAccessMode,
}

// ---------------------------------------------------------------------------
// CWD-aware runtime methods — aligned with Codex protocol/src/permissions.rs
// ---------------------------------------------------------------------------

impl FileSystemSandboxPolicy {
    /// Filesystem policy matching `WorkspaceWrite` semantics without requiring
    /// callers to construct a legacy `SandboxPolicy` first.
    pub fn workspace_write(
        writable_roots: &[AbsolutePathBuf],
        exclude_tmpdir_env_var: bool,
        exclude_slash_tmp: bool,
    ) -> Self {
        let mut entries = vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        }];
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::ProjectRoots { subpath: None },
            },
            access: FileSystemAccessMode::Write,
        });
        if !exclude_tmpdir_env_var {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Tmpdir,
                },
                access: FileSystemAccessMode::Write,
            });
        }
        if !exclude_slash_tmp {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::SlashTmp,
                },
                access: FileSystemAccessMode::Write,
            });
        }
        entries.extend(writable_roots.iter().map(|path| FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: path.clone(),
            },
            access: FileSystemAccessMode::Write,
        }));

        for writable_root in writable_roots {
            for protected_path in default_read_only_subpaths_for_writable_root(writable_root) {
                append_default_read_only_path_if_no_explicit_rule(
                    &mut entries,
                    protected_path,
                );
            }
        }

        FileSystemSandboxPolicy::restricted(entries)
    }

    /// Returns true when platform-default readable roots should be included.
    pub fn include_platform_defaults(&self) -> bool {
        !self.has_full_disk_read_access()
            && matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self.entries.iter().any(|entry| {
                matches!(
                    &entry.path,
                    FileSystemPath::Special { value }
                        if matches!(value, FileSystemSpecialPath::Minimal)
                            && entry.access.can_read()
                )
            })
    }

    /// Resolve the effective access mode for `path` given a working directory.
    pub fn resolve_access_with_cwd(&self, path: &Path, cwd: &Path) -> FileSystemAccessMode {
        match self.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                return FileSystemAccessMode::Write;
            }
            FileSystemSandboxKind::Restricted => {}
        }

        let Some(path) = resolve_candidate_path(path, cwd) else {
            return FileSystemAccessMode::None;
        };

        self.resolved_entries_with_cwd(cwd)
            .into_iter()
            .filter(|entry| path.as_path().starts_with(entry.path.as_path()))
            .max_by_key(resolved_entry_precedence)
            .map(|entry| entry.access)
            .unwrap_or(FileSystemAccessMode::None)
    }

    /// Check whether `path` is readable under this policy.
    pub fn can_read_path_with_cwd(&self, path: &Path, cwd: &Path) -> bool {
        self.resolve_access_with_cwd(path, cwd).can_read()
    }

    /// Check whether `path` is writable under this policy, including
    /// metadata protection checks.
    pub fn can_write_path_with_cwd(&self, path: &Path, cwd: &Path) -> bool {
        if !self.resolve_access_with_cwd(path, cwd).can_write() {
            return false;
        }
        if self.has_full_disk_write_access() {
            return true;
        }
        !self.is_metadata_write_denied(path, cwd)
    }

    fn is_metadata_write_denied(&self, path: &Path, cwd: &Path) -> bool {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return false;
        }
        let Some(target) = resolve_candidate_path(path, cwd) else {
            return true;
        };
        metadata_child_of_writable_root(self, target.as_path(), cwd).is_some()
    }

    /// Returns the explicit readable roots resolved against the provided cwd.
    pub fn get_readable_roots_with_cwd(&self, cwd: &Path) -> Vec<AbsolutePathBuf> {
        if self.has_full_disk_read_access() {
            return Vec::new();
        }

        dedup_absolute_paths(
            self.resolved_entries_with_cwd(cwd)
                .into_iter()
                .filter(|entry| entry.access.can_read())
                .filter(|entry| self.can_read_path_with_cwd(entry.path.as_path(), cwd))
                .map(|entry| entry.path)
                .collect(),
        )
    }

    /// Returns the writable roots resolved against the provided cwd.
    pub fn get_writable_roots_with_cwd(&self, cwd: &Path) -> Vec<AbsolutePathBuf> {
        if self.has_full_disk_write_access() {
            return Vec::new();
        }

        dedup_absolute_paths(
            self.resolved_entries_with_cwd(cwd)
                .into_iter()
                .filter(|entry| entry.access.can_write())
                .filter(|entry| self.can_write_path_with_cwd(entry.path.as_path(), cwd))
                .map(|entry| entry.path)
                .collect(),
        )
    }

    /// Returns writable roots as rich `WritableRoot` structs with read-only
    /// subpath and protected metadata information.
    pub fn get_rich_writable_roots_with_cwd(&self, cwd: &Path) -> Vec<WritableRoot> {
        self.get_writable_roots_with_cwd(cwd)
            .into_iter()
            .map(|root| {
                let read_only_subpaths = self.read_only_subpaths_for_root(&root, cwd);
                WritableRoot {
                    root,
                    read_only_subpaths,
                    protected_metadata_names: PROTECTED_METADATA_PATH_NAMES
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                }
            })
            .collect()
    }

    /// Compute read-only subpaths under a writable root by finding entries
    /// that are read-only and children of the given root.
    fn read_only_subpaths_for_root(
        &self,
        root: &AbsolutePathBuf,
        cwd: &Path,
    ) -> Vec<AbsolutePathBuf> {
        self.resolved_entries_with_cwd(cwd)
            .into_iter()
            .filter(|entry| !entry.access.can_write())
            .filter(|entry| entry.path.as_path().starts_with(root.as_path()))
            .map(|entry| entry.path)
            .collect()
    }

    /// Returns explicit unreadable roots resolved against the provided cwd.
    pub fn get_unreadable_roots_with_cwd(&self, cwd: &Path) -> Vec<AbsolutePathBuf> {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return Vec::new();
        }

        let root = AbsolutePathBuf::from_absolute_path(cwd)
            .ok()
            .and_then(|c| absolute_root_path_for_cwd(&c));

        dedup_absolute_paths(
            self.resolved_entries_with_cwd(cwd)
                .iter()
                .filter(|entry| entry.access == FileSystemAccessMode::None)
                .filter(|entry| !self.can_read_path_with_cwd(entry.path.as_path(), cwd))
                .filter(|entry| root.as_ref() != Some(&entry.path))
                .map(|entry| entry.path.clone())
                .collect(),
        )
    }

    /// Returns unreadable glob patterns resolved against the provided cwd.
    pub fn get_unreadable_globs_with_cwd(&self, cwd: &Path) -> Vec<String> {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return Vec::new();
        }

        let mut patterns: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::None)
            .filter_map(|entry| match &entry.path {
                FileSystemPath::GlobPattern { pattern } => Some(
                    AbsolutePathBuf::resolve_path_against_base(pattern, cwd)
                        .to_string_lossy()
                        .into_owned(),
                ),
                FileSystemPath::Path { .. } | FileSystemPath::Special { .. } => None,
            })
            .collect();
        patterns.sort();
        patterns.dedup();
        patterns
    }

    /// Replaces symbolic `:project_roots` entries with absolute paths
    /// resolved against `cwd`.
    pub fn materialize_project_roots_with_cwd(mut self, cwd: &Path) -> Self {
        let cwd = AbsolutePathBuf::from_absolute_path(cwd).ok();
        for entry in &mut self.entries {
            match &entry.path {
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { .. },
                } => {
                    if let Some(path) =
                        resolve_file_system_path(&entry.path, cwd.as_ref())
                    {
                        entry.path = FileSystemPath::Path { path };
                    }
                }
                FileSystemPath::GlobPattern { pattern } => {
                    if let (Some(cwd_ref), Some(subpath)) =
                        (cwd.as_ref(), parse_project_roots_glob_pattern(pattern))
                    {
                        entry.path = FileSystemPath::GlobPattern {
                            pattern: resolve_project_roots_glob_pattern(subpath, cwd_ref),
                        };
                    }
                }
                FileSystemPath::Special { .. } | FileSystemPath::Path { .. } => {}
            }
        }
        self
    }

    /// Expand symbolic `:project_roots` entries against multiple workspace roots.
    ///
    /// Each `ProjectRoots { subpath }` entry is replaced with one concrete
    /// `Path { path }` per workspace root. Non-project-roots entries are
    /// passed through unchanged.
    pub fn materialize_project_roots_with_workspace_roots(
        &self,
        workspace_roots: &[AbsolutePathBuf],
    ) -> Self {
        let mut new_entries = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            match &entry.path {
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { subpath },
                } => {
                    for root in workspace_roots {
                        let concrete = if let Some(sub) = subpath {
                            AbsolutePathBuf::resolve_path_against_base(sub, root.as_path())
                        } else {
                            root.clone()
                        };
                        new_entries.push(FileSystemSandboxEntry {
                            path: FileSystemPath::Path { path: concrete },
                            access: entry.access,
                        });
                    }
                }
                FileSystemPath::GlobPattern { pattern } => {
                    if let Some(subpath) = parse_project_roots_glob_pattern(pattern) {
                        for root in workspace_roots {
                            new_entries.push(FileSystemSandboxEntry {
                                path: FileSystemPath::GlobPattern {
                                    pattern: resolve_project_roots_glob_pattern(subpath, root),
                                },
                                access: entry.access,
                            });
                        }
                    } else {
                        new_entries.push(entry.clone());
                    }
                }
                _ => {
                    new_entries.push(entry.clone());
                }
            }
        }
        FileSystemSandboxPolicy {
            kind: self.kind,
            glob_scan_max_depth: self.glob_scan_max_depth,
            entries: new_entries,
        }
    }

    /// Append additional readable roots, skipping those already readable.
    pub fn with_additional_readable_roots(
        mut self,
        cwd: &Path,
        additional_readable_roots: &[AbsolutePathBuf],
    ) -> Self {
        if self.has_full_disk_read_access() {
            return self;
        }
        for path in additional_readable_roots {
            if self.can_read_path_with_cwd(path.as_path(), cwd) {
                continue;
            }
            self.entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: path.clone(),
                },
                access: FileSystemAccessMode::Read,
            });
        }
        self
    }

    /// Append additional writable roots, skipping those already writable.
    pub fn with_additional_writable_roots(
        mut self,
        cwd: &Path,
        additional_writable_roots: &[AbsolutePathBuf],
    ) -> Self {
        for path in additional_writable_roots {
            if self.can_write_path_with_cwd(path.as_path(), cwd) {
                continue;
            }
            self.entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: path.clone(),
                },
                access: FileSystemAccessMode::Write,
            });
        }
        self
    }

    /// Preserve explicit read-deny rules from `existing` when a caller
    /// replaces the allow side of a policy.
    pub fn preserve_deny_read_restrictions_from(&mut self, existing: &Self) {
        let has_deny_entries = existing
            .entries
            .iter()
            .any(|entry| entry.access == FileSystemAccessMode::None);

        if matches!(self.kind, FileSystemSandboxKind::Unrestricted) && has_deny_entries {
            *self = Self::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            }]);
        }

        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return;
        }

        if self.glob_scan_max_depth.is_none() {
            self.glob_scan_max_depth = existing.glob_scan_max_depth;
        }

        for deny_entry in existing
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::None)
        {
            if !self.entries.iter().any(|entry| entry == deny_entry) {
                self.entries.push(deny_entry.clone());
            }
        }
    }

    /// Returns true when two policies resolve to the same filesystem access
    /// model for `cwd`, ignoring incidental entry ordering.
    pub fn is_semantically_equivalent_to(&self, other: &Self, cwd: &Path) -> bool {
        self.semantic_signature(cwd) == other.semantic_signature(cwd)
    }

    /// Returns true when a restricted policy contains any entry that really
    /// reduces a broader `:root = write` grant.
    fn has_write_narrowing_entries(&self) -> bool {
        matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self.entries.iter().any(|entry| {
                if entry.access.can_write() {
                    return false;
                }
                match &entry.path {
                    FileSystemPath::Path { .. } => !self.has_same_target_write_override(entry),
                    FileSystemPath::GlobPattern { .. } => true,
                    FileSystemPath::Special { value } => match value {
                        FileSystemSpecialPath::Root => entry.access == FileSystemAccessMode::None,
                        FileSystemSpecialPath::Minimal | FileSystemSpecialPath::Unknown { .. } => {
                            false
                        }
                        _ => !self.has_same_target_write_override(entry),
                    },
                }
            })
    }

    fn has_same_target_write_override(&self, entry: &FileSystemSandboxEntry) -> bool {
        self.entries.iter().any(|candidate| {
            candidate.access.can_write()
                && candidate.access > entry.access
                && file_system_paths_share_target(&candidate.path, &entry.path)
        })
    }

    fn resolved_entries_with_cwd(&self, cwd: &Path) -> Vec<ResolvedFileSystemEntry> {
        let cwd_absolute = AbsolutePathBuf::from_absolute_path(cwd).ok();
        self.entries
            .iter()
            .filter_map(|entry| {
                resolve_entry_path(&entry.path, cwd_absolute.as_ref()).map(|path| {
                    ResolvedFileSystemEntry {
                        path,
                        access: entry.access,
                    }
                })
            })
            .collect()
    }

    fn semantic_signature(&self, cwd: &Path) -> FileSystemSemanticSignature {
        let mut readable_roots = self.get_readable_roots_with_cwd(cwd);
        readable_roots.sort_by(|a, b| a.as_path().cmp(b.as_path()));
        let mut writable_roots = self.get_writable_roots_with_cwd(cwd);
        writable_roots.sort_by(|a, b| a.as_path().cmp(b.as_path()));
        let mut unreadable_roots = self.get_unreadable_roots_with_cwd(cwd);
        unreadable_roots.sort_by(|a, b| a.as_path().cmp(b.as_path()));
        FileSystemSemanticSignature {
            has_full_disk_read_access: self.has_full_disk_read_access(),
            has_full_disk_write_access: self.has_full_disk_write_access(),
            include_platform_defaults: self.include_platform_defaults(),
            readable_roots,
            writable_roots,
            unreadable_roots,
            unreadable_globs: self.get_unreadable_globs_with_cwd(cwd),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FileSystemSemanticSignature {
    has_full_disk_read_access: bool,
    has_full_disk_write_access: bool,
    include_platform_defaults: bool,
    readable_roots: Vec<AbsolutePathBuf>,
    writable_roots: Vec<AbsolutePathBuf>,
    unreadable_roots: Vec<AbsolutePathBuf>,
    unreadable_globs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Module-level helper functions
// ---------------------------------------------------------------------------

fn resolve_file_system_special_path(
    value: &FileSystemSpecialPath,
    cwd: Option<&AbsolutePathBuf>,
) -> Option<AbsolutePathBuf> {
    match value {
        FileSystemSpecialPath::Root
        | FileSystemSpecialPath::Minimal
        | FileSystemSpecialPath::Unknown { .. } => None,
        FileSystemSpecialPath::ProjectRoots { subpath } => {
            let cwd = cwd?;
            match subpath.as_ref() {
                Some(subpath) => Some(AbsolutePathBuf::resolve_path_against_base(
                    subpath,
                    cwd.as_path(),
                )),
                None => Some(cwd.clone()),
            }
        }
        FileSystemSpecialPath::Tmpdir => {
            let tmpdir = std::env::var_os("TMPDIR")?;
            if tmpdir.is_empty() {
                None
            } else {
                AbsolutePathBuf::from_absolute_path(PathBuf::from(tmpdir)).ok()
            }
        }
        FileSystemSpecialPath::SlashTmp => {
            let slash_tmp = AbsolutePathBuf::from_absolute_path("/tmp").ok()?;
            if !slash_tmp.as_path().is_dir() {
                return None;
            }
            Some(slash_tmp)
        }
    }
}

fn resolve_file_system_path(
    path: &FileSystemPath,
    cwd: Option<&AbsolutePathBuf>,
) -> Option<AbsolutePathBuf> {
    match path {
        FileSystemPath::Path { path } => Some(path.clone()),
        FileSystemPath::GlobPattern { .. } => None,
        FileSystemPath::Special { value } => resolve_file_system_special_path(value, cwd),
    }
}

fn resolve_entry_path(
    path: &FileSystemPath,
    cwd: Option<&AbsolutePathBuf>,
) -> Option<AbsolutePathBuf> {
    match path {
        FileSystemPath::Special {
            value: FileSystemSpecialPath::Root,
        } => cwd.and_then(absolute_root_path_for_cwd),
        _ => resolve_file_system_path(path, cwd),
    }
}

fn resolve_candidate_path(path: &Path, cwd: &Path) -> Option<AbsolutePathBuf> {
    if path.is_absolute() {
        AbsolutePathBuf::from_absolute_path(path).ok()
    } else {
        Some(
            AbsolutePathBuf::from_absolute_path(cwd)
                .ok()?
                .join(path),
        )
    }
}

fn absolute_root_path_for_cwd(cwd: &AbsolutePathBuf) -> Option<AbsolutePathBuf> {
    let root = cwd.as_path().ancestors().last()?;
    AbsolutePathBuf::from_absolute_path(root).ok()
}

fn resolved_entry_precedence(entry: &ResolvedFileSystemEntry) -> (usize, FileSystemAccessMode) {
    let specificity = entry.path.as_path().components().count();
    (specificity, entry.access)
}

fn file_system_paths_share_target(left: &FileSystemPath, right: &FileSystemPath) -> bool {
    match (left, right) {
        (FileSystemPath::Path { path: left }, FileSystemPath::Path { path: right }) => {
            left == right
        }
        (FileSystemPath::Special { value: left }, FileSystemPath::Special { value: right }) => {
            special_paths_share_target(left, right)
        }
        (FileSystemPath::Path { path }, FileSystemPath::Special { value })
        | (FileSystemPath::Special { value }, FileSystemPath::Path { path }) => {
            special_path_matches_absolute(value, path)
        }
        (
            FileSystemPath::GlobPattern { pattern: left },
            FileSystemPath::GlobPattern { pattern: right },
        ) => left == right,
        (FileSystemPath::GlobPattern { .. }, _) | (_, FileSystemPath::GlobPattern { .. }) => false,
    }
}

fn special_paths_share_target(left: &FileSystemSpecialPath, right: &FileSystemSpecialPath) -> bool {
    match (left, right) {
        (FileSystemSpecialPath::Root, FileSystemSpecialPath::Root)
        | (FileSystemSpecialPath::Minimal, FileSystemSpecialPath::Minimal)
        | (FileSystemSpecialPath::Tmpdir, FileSystemSpecialPath::Tmpdir)
        | (FileSystemSpecialPath::SlashTmp, FileSystemSpecialPath::SlashTmp) => true,
        (
            FileSystemSpecialPath::ProjectRoots { subpath: left },
            FileSystemSpecialPath::ProjectRoots { subpath: right },
        ) => left == right,
        (
            FileSystemSpecialPath::Unknown {
                path: left,
                subpath: left_sub,
            },
            FileSystemSpecialPath::Unknown {
                path: right,
                subpath: right_sub,
            },
        ) => left == right && left_sub == right_sub,
        _ => false,
    }
}

fn special_path_matches_absolute(value: &FileSystemSpecialPath, path: &AbsolutePathBuf) -> bool {
    match value {
        FileSystemSpecialPath::Root => path.as_path().parent().is_none(),
        FileSystemSpecialPath::SlashTmp => path.as_path() == Path::new("/tmp"),
        _ => false,
    }
}

fn parse_project_roots_glob_pattern(pattern: &str) -> Option<&Path> {
    pattern
        .strip_prefix(PROJECT_ROOTS_GLOB_PATTERN_PREFIX)
        .map(Path::new)
}

fn resolve_project_roots_glob_pattern(subpath: &Path, root: &AbsolutePathBuf) -> String {
    AbsolutePathBuf::resolve_path_against_base(subpath, root.as_path())
        .to_string_lossy()
        .into_owned()
}

fn dedup_absolute_paths(paths: Vec<AbsolutePathBuf>) -> Vec<AbsolutePathBuf> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.to_path_buf()) {
            deduped.push(path);
        }
    }
    deduped
}

fn metadata_path_name(name: &OsStr) -> Option<&'static str> {
    PROTECTED_METADATA_PATH_NAMES
        .iter()
        .copied()
        .find(|n| name == OsStr::new(n))
}

fn metadata_child_of_writable_root(
    policy: &FileSystemSandboxPolicy,
    target: &Path,
    cwd: &Path,
) -> Option<(AbsolutePathBuf, &'static str)> {
    let cwd_absolute = AbsolutePathBuf::from_absolute_path(cwd).ok()?;
    let resolved = policy.resolved_entries_with_cwd(cwd);
    let writable_roots: Vec<_> = resolved
        .iter()
        .filter(|e| e.access.can_write())
        .map(|e| &e.path)
        .collect();

    for wr in writable_roots {
        if !target.starts_with(wr.as_path()) {
            continue;
        }
        let relative = target.strip_prefix(wr.as_path()).ok()?;
        let first_component = relative.components().next()?;
        if let std::path::Component::Normal(name) = first_component {
            if let Some(metadata_name) = metadata_path_name(name) {
                let protected_path = AbsolutePathBuf::resolve_path_against_base(
                    metadata_name,
                    wr.as_path(),
                );
                return Some((protected_path, metadata_name));
            }
        }
    }
    let _ = cwd_absolute;
    None
}

fn default_read_only_subpaths_for_writable_root(
    writable_root: &AbsolutePathBuf,
) -> Vec<AbsolutePathBuf> {
    let mut subpaths = Vec::new();
    let top_level_git = writable_root.join(".git");
    if top_level_git.as_path().exists() {
        subpaths.push(top_level_git);
    }
    let top_level_agents = writable_root.join(".agents");
    if top_level_agents.as_path().is_dir() {
        subpaths.push(top_level_agents);
    }
    let top_level_xiaolin = writable_root.join(".xiaolin");
    if top_level_xiaolin.as_path().is_dir() {
        subpaths.push(top_level_xiaolin);
    }
    subpaths
}

fn append_default_read_only_path_if_no_explicit_rule(
    entries: &mut Vec<FileSystemSandboxEntry>,
    path: AbsolutePathBuf,
) {
    let already_has_rule = entries.iter().any(|entry| {
        matches!(&entry.path, FileSystemPath::Path { path: p } if *p == path)
    });
    if !already_has_rule {
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Path { path },
            access: FileSystemAccessMode::Read,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::path::test_support::{test_path_buf, PathBufExt};

    fn abs(p: &str) -> AbsolutePathBuf {
        test_path_buf(p).abs()
    }

    #[test]
    fn file_system_access_mode_predicates() {
        assert!(FileSystemAccessMode::Read.can_read());
        assert!(!FileSystemAccessMode::Read.can_write());
        assert!(FileSystemAccessMode::Write.can_read());
        assert!(FileSystemAccessMode::Write.can_write());
        assert!(!FileSystemAccessMode::None.can_read());
        assert!(!FileSystemAccessMode::None.can_write());
    }

    #[test]
    fn file_system_sandbox_policy_default_has_root_read() {
        let policy = FileSystemSandboxPolicy::default();
        assert_eq!(policy.kind, FileSystemSandboxKind::Restricted);
        assert!(policy.has_full_disk_read_access());
        assert!(!policy.has_full_disk_write_access());
    }

    #[test]
    fn file_system_sandbox_policy_unrestricted() {
        let policy = FileSystemSandboxPolicy::unrestricted();
        assert_eq!(policy.kind, FileSystemSandboxKind::Unrestricted);
        assert!(policy.entries.is_empty());
        assert!(policy.has_full_disk_read_access());
        assert!(policy.has_full_disk_write_access());
    }

    #[test]
    fn file_system_sandbox_policy_external() {
        let policy = FileSystemSandboxPolicy::external_sandbox();
        assert_eq!(policy.kind, FileSystemSandboxKind::ExternalSandbox);
        assert!(policy.has_full_disk_read_access());
        assert!(policy.has_full_disk_write_access());
    }

    #[test]
    fn file_system_sandbox_policy_has_denied_read() {
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "**/.env".into(),
            },
            access: FileSystemAccessMode::None,
        }]);
        assert!(policy.has_denied_read_restrictions());
    }

    #[test]
    fn root_read_with_deny_entry_is_not_full_disk_read() {
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
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
        assert!(!policy.has_full_disk_read_access());
    }

    #[test]
    fn file_system_sandbox_policy_json_roundtrip() {
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project"),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: "**/.env".into(),
                },
                access: FileSystemAccessMode::None,
            },
        ]);
        let json = serde_json::to_string(&policy).unwrap();
        let _guard = xiaolin_core::path::AbsolutePathBufGuard::new(Path::new("/"));
        let deserialized: FileSystemSandboxPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);
    }

    #[test]
    fn file_system_special_path_factories() {
        let pr = FileSystemSpecialPath::project_roots(Some(PathBuf::from("sub")));
        assert!(matches!(pr, FileSystemSpecialPath::ProjectRoots { subpath: Some(_) }));

        let unk = FileSystemSpecialPath::unknown("future-token", None);
        assert!(matches!(unk, FileSystemSpecialPath::Unknown { path, subpath: None } if path == "future-token"));
    }

    #[test]
    fn network_sandbox_policy_enabled() {
        assert!(NetworkSandboxPolicy::Enabled.is_enabled());
    }

    #[test]
    fn network_sandbox_policy_restricted() {
        assert!(!NetworkSandboxPolicy::Restricted.is_enabled());
    }

    #[test]
    fn network_sandbox_policy_default_is_restricted() {
        assert_eq!(NetworkSandboxPolicy::default(), NetworkSandboxPolicy::Restricted);
    }

    // -----------------------------------------------------------------------
    // CWD-aware runtime method tests
    // -----------------------------------------------------------------------

    fn make_workspace_policy(cwd: &Path) -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path(cwd).unwrap(),
                },
                access: FileSystemAccessMode::Write,
            },
        ])
    }

    #[test]
    fn resolve_access_unrestricted_always_write() {
        let policy = FileSystemSandboxPolicy::unrestricted();
        let cwd = test_path_buf("/home/user");
        assert_eq!(
            policy.resolve_access_with_cwd(Path::new("/any/path"), &cwd),
            FileSystemAccessMode::Write
        );
    }

    #[test]
    fn resolve_access_restricted_readable_root() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        assert_eq!(
            policy.resolve_access_with_cwd(Path::new("/etc/hosts"), &cwd),
            FileSystemAccessMode::Read
        );
    }

    #[test]
    fn resolve_access_restricted_writable_project() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        assert_eq!(
            policy.resolve_access_with_cwd(Path::new("/home/user/project/src/main.rs"), &cwd),
            FileSystemAccessMode::Write
        );
    }

    #[test]
    fn can_read_and_write_convenience_methods() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        assert!(policy.can_read_path_with_cwd(Path::new("/etc/hosts"), &cwd));
        assert!(!policy.can_write_path_with_cwd(Path::new("/etc/hosts"), &cwd));
        assert!(policy.can_read_path_with_cwd(Path::new("/home/user/project/file.rs"), &cwd));
    }

    #[test]
    fn resolve_access_deny_entry_wins() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project/secrets"),
                },
                access: FileSystemAccessMode::None,
            },
        ]);
        assert!(!policy.can_read_path_with_cwd(
            Path::new("/home/user/project/secrets/key.pem"),
            &cwd
        ));
    }

    #[test]
    fn get_readable_roots_returns_explicit_roots() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project"),
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/usr/share"),
                },
                access: FileSystemAccessMode::Read,
            },
        ]);
        let roots = policy.get_readable_roots_with_cwd(&cwd);
        assert_eq!(roots.len(), 2);
    }

    #[test]
    fn get_readable_roots_empty_for_full_disk_read() {
        let policy = FileSystemSandboxPolicy::default();
        let cwd = test_path_buf("/home/user");
        let roots = policy.get_readable_roots_with_cwd(&cwd);
        assert!(roots.is_empty());
    }

    #[test]
    fn get_writable_roots_returns_explicit_roots() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        let roots = policy.get_writable_roots_with_cwd(&cwd);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].as_path(), cwd.as_path());
    }

    #[test]
    fn get_unreadable_roots() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project/secrets"),
                },
                access: FileSystemAccessMode::None,
            },
        ]);
        let roots = policy.get_unreadable_roots_with_cwd(&cwd);
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].as_path(),
            test_path_buf("/home/user/project/secrets").as_path()
        );
    }

    #[test]
    fn get_unreadable_globs() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: "**/.env".into(),
                },
                access: FileSystemAccessMode::None,
            },
        ]);
        let globs = policy.get_unreadable_globs_with_cwd(&cwd);
        assert_eq!(globs.len(), 1);
    }

    #[test]
    fn materialize_project_roots_with_cwd() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { subpath: None },
                },
                access: FileSystemAccessMode::Write,
            },
        ]);
        let materialized = policy.materialize_project_roots_with_cwd(&cwd);
        let cwd_abs = AbsolutePathBuf::from_absolute_path(&cwd).unwrap();
        assert!(materialized.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Path { path } if *path == cwd_abs
        )));
    }

    #[test]
    fn with_additional_readable_roots_appends() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: abs("/home/user/project"),
            },
            access: FileSystemAccessMode::Read,
        }]);
        let extra = abs("/opt/data");
        let policy = policy.with_additional_readable_roots(&cwd, &[extra.clone()]);
        assert!(policy.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Path { path } if *path == extra
        )));
    }

    #[test]
    fn with_additional_readable_roots_skips_existing() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        }]);
        let extra = abs("/opt/data");
        let result = policy.with_additional_readable_roots(&cwd, &[extra]);
        assert_eq!(result.entries.len(), 1);
    }

    #[test]
    fn with_additional_writable_roots_appends() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        let extra = abs("/tmp/build");
        let policy = policy.with_additional_writable_roots(&cwd, &[extra.clone()]);
        assert!(policy.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Path { path } if *path == extra
        ) && e.access == FileSystemAccessMode::Write));
    }

    #[test]
    fn preserve_deny_read_restrictions() {
        let deny_entry = FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "**/.env".into(),
            },
            access: FileSystemAccessMode::None,
        };
        let mut existing = FileSystemSandboxPolicy::restricted(vec![deny_entry.clone()]);
        existing.glob_scan_max_depth = Some(2);

        let mut replacement = FileSystemSandboxPolicy::unrestricted();
        replacement.preserve_deny_read_restrictions_from(&existing);

        assert!(matches!(
            replacement.kind,
            FileSystemSandboxKind::Restricted
        ));
        assert!(replacement.entries.iter().any(|e| e == &deny_entry));
        assert_eq!(replacement.glob_scan_max_depth, Some(2));
    }

    #[test]
    fn include_platform_defaults_with_minimal() {
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Minimal,
            },
            access: FileSystemAccessMode::Read,
        }]);
        assert!(policy.include_platform_defaults());
    }

    #[test]
    fn include_platform_defaults_false_for_full_read() {
        let policy = FileSystemSandboxPolicy::default();
        assert!(!policy.include_platform_defaults());
    }

    #[test]
    fn has_write_narrowing_with_read_only_carveout() {
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project/.git"),
                },
                access: FileSystemAccessMode::Read,
            },
        ]);
        assert!(!policy.has_full_disk_write_access());
    }

    #[test]
    fn has_write_narrowing_shadowed_by_write_override() {
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project"),
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs("/home/user/project"),
                },
                access: FileSystemAccessMode::Write,
            },
        ]);
        assert!(policy.has_full_disk_write_access());
    }

    #[test]
    fn semantic_equivalence_same_policy() {
        let cwd = test_path_buf("/home/user/project");
        let a = make_workspace_policy(&cwd);
        let b = make_workspace_policy(&cwd);
        assert!(a.is_semantically_equivalent_to(&b, &cwd));
    }

    #[test]
    fn semantic_equivalence_different_policy() {
        let cwd = test_path_buf("/home/user/project");
        let a = make_workspace_policy(&cwd);
        let b = FileSystemSandboxPolicy::default();
        assert!(!a.is_semantically_equivalent_to(&b, &cwd));
    }

    #[test]
    fn workspace_write_constructor() {
        let root = abs("/home/user/project");
        let policy = FileSystemSandboxPolicy::workspace_write(
            &[root.clone()],
            false,
            false,
        );
        assert!(matches!(policy.kind, FileSystemSandboxKind::Restricted));
        assert!(policy.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Path { path } if *path == root
        ) && e.access == FileSystemAccessMode::Write));
        assert!(policy.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Special { value: FileSystemSpecialPath::ProjectRoots { subpath: None } }
        ) && e.access == FileSystemAccessMode::Write));
    }

    #[test]
    fn workspace_write_excludes_tmpdir() {
        let root = abs("/home/user/project");
        let policy = FileSystemSandboxPolicy::workspace_write(&[root], true, true);
        assert!(!policy.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Special { value: FileSystemSpecialPath::Tmpdir }
        )));
        assert!(!policy.entries.iter().any(|e| matches!(
            &e.path,
            FileSystemPath::Special { value: FileSystemSpecialPath::SlashTmp }
        )));
    }

    #[test]
    fn is_protected_metadata_name_checks() {
        assert!(is_protected_metadata_name(OsStr::new(".git")));
        assert!(is_protected_metadata_name(OsStr::new(".agents")));
        assert!(is_protected_metadata_name(OsStr::new(".xiaolin")));
        assert!(!is_protected_metadata_name(OsStr::new(".env")));
        assert!(!is_protected_metadata_name(OsStr::new("src")));
    }

    #[test]
    fn project_roots_glob_pattern_roundtrip() {
        let subpath = Path::new("**/.env");
        let pattern = project_roots_glob_pattern(subpath);
        assert!(pattern.starts_with(PROJECT_ROOTS_GLOB_PATTERN_PREFIX));
        let parsed = parse_project_roots_glob_pattern(&pattern);
        assert_eq!(parsed, Some(subpath));
    }

    #[test]
    fn forbidden_metadata_write_blocks_git() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        let result = forbidden_agent_metadata_write(
            Path::new("/home/user/project/.git/config"),
            &cwd,
            &policy,
        );
        assert_eq!(result, Some(".git"));
    }

    #[test]
    fn forbidden_metadata_write_allows_normal_file() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        let result = forbidden_agent_metadata_write(
            Path::new("/home/user/project/src/main.rs"),
            &cwd,
            &policy,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn forbidden_metadata_write_unrestricted_allows_all() {
        let cwd = test_path_buf("/home/user/project");
        let policy = FileSystemSandboxPolicy::unrestricted();
        let result = forbidden_agent_metadata_write(
            Path::new("/home/user/project/.git/config"),
            &cwd,
            &policy,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_access_relative_path_resolved_against_cwd() {
        let cwd = test_path_buf("/home/user/project");
        let policy = make_workspace_policy(&cwd);
        assert_eq!(
            policy.resolve_access_with_cwd(Path::new("src/main.rs"), &cwd),
            FileSystemAccessMode::Write
        );
    }

    #[test]
    fn dedup_absolute_paths_removes_duplicates() {
        let p = AbsolutePathBuf::from_absolute_path(test_path_buf("/tmp")).unwrap();
        let deduped = dedup_absolute_paths(vec![p.clone(), p.clone(), p.clone()]);
        assert_eq!(deduped.len(), 1);
    }
}
