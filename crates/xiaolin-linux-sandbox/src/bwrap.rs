//! Bubblewrap-based filesystem sandboxing for Linux.
//!
//! Mirrors Codex's semantics:
//! - the filesystem is read-only by default,
//! - explicit writable roots are layered on top, and
//! - sensitive subpaths such as `.git`, `.agents`, and `.xiaolin` remain
//!   read-only even when their parent root is writable.
//!
//! The overall Linux sandbox is composed of:
//! - seccomp + `PR_SET_NO_NEW_PRIVS` applied in-process, and
//! - bubblewrap used to construct the filesystem view before exec.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};
use xiaolin_core::path::AbsolutePathBuf;
use xiaolin_security::permission_profile::{
    FileSystemAccessMode, FileSystemPath, FileSystemSandboxPolicy, FileSystemSpecialPath,
    WritableRoot, is_protected_metadata_name,
};

const LINUX_PLATFORM_DEFAULT_READ_ROOTS: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/lib",
    "/lib64",
    "/nix/store",
    "/run/current-system/sw",
];

const MAX_UNREADABLE_GLOB_MATCHES: usize = 8192;

/// Options that control how bubblewrap is invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BwrapOptions {
    pub mount_proc: bool,
    pub network_mode: BwrapNetworkMode,
    pub glob_scan_max_depth: Option<usize>,
}

impl Default for BwrapOptions {
    fn default() -> Self {
        Self {
            mount_proc: true,
            network_mode: BwrapNetworkMode::FullAccess,
            glob_scan_max_depth: None,
        }
    }
}

/// Network policy modes for bubblewrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BwrapNetworkMode {
    #[default]
    FullAccess,
    Isolated,
    ProxyOnly,
}

impl BwrapNetworkMode {
    fn should_unshare_network(self) -> bool {
        !matches!(self, Self::FullAccess)
    }
}

/// The result of building bwrap command arguments.
#[derive(Debug)]
pub struct BwrapArgs {
    pub args: Vec<String>,
    pub preserved_files: Vec<File>,
    pub synthetic_mount_targets: Vec<SyntheticMountTarget>,
    pub protected_create_targets: Vec<ProtectedCreateTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntheticMountTargetKind {
    EmptyFile,
    EmptyDirectory,
}

/// Tracks a synthetic path created as a bwrap mount target that should
/// be cleaned up after the sandboxed process exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntheticMountTarget {
    path: PathBuf,
    kind: SyntheticMountTargetKind,
    pre_existing_path: Option<FileIdentity>,
}

impl SyntheticMountTarget {
    pub fn missing(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: SyntheticMountTargetKind::EmptyFile,
            pre_existing_path: None,
        }
    }

    pub fn missing_empty_directory(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: SyntheticMountTargetKind::EmptyDirectory,
            pre_existing_path: None,
        }
    }

    pub fn existing_empty_file(path: &Path, metadata: &Metadata) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: SyntheticMountTargetKind::EmptyFile,
            pre_existing_path: Some(FileIdentity::from_metadata(metadata)),
        }
    }

    fn existing_empty_directory(path: &Path, metadata: &Metadata) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: SyntheticMountTargetKind::EmptyDirectory,
            pre_existing_path: Some(FileIdentity::from_metadata(metadata)),
        }
    }

    pub fn preserves_pre_existing_path(&self) -> bool {
        self.pre_existing_path.is_some()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> SyntheticMountTargetKind {
        self.kind
    }

    pub fn should_remove_after_bwrap(&self, metadata: &Metadata) -> bool {
        match self.kind {
            SyntheticMountTargetKind::EmptyFile => {
                if !metadata.file_type().is_file() || metadata.len() != 0 {
                    return false;
                }
            }
            SyntheticMountTargetKind::EmptyDirectory => {
                if !metadata.file_type().is_dir() {
                    return false;
                }
            }
        }

        match self.pre_existing_path {
            Some(pre_existing_path) => pre_existing_path != FileIdentity::from_metadata(metadata),
            None => true,
        }
    }
}

/// A protected path that must not be created by the sandbox. When the path
/// is missing, the bwrap setup ensures it stays absent by masking the first
/// missing ancestor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedCreateTarget {
    path: PathBuf,
}

impl ProtectedCreateTarget {
    pub fn missing(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub const WSL1_BWRAP_WARNING: &str =
    "bubblewrap is not supported on WSL1; please upgrade to WSL2";

/// Check if we're running on WSL1.
pub fn is_wsl1() -> bool {
    if let Ok(v) = std::fs::read_to_string("/proc/version") {
        let lower = v.to_ascii_lowercase();
        if lower.contains("microsoft") || lower.contains("wsl") {
            if let Ok(info) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
                return info.to_ascii_lowercase().contains("microsoft")
                    && !Path::new("/run/WSL").exists();
            }
        }
    }
    false
}

/// Build bwrap command arguments from a `FileSystemSandboxPolicy`.
///
/// When the policy grants full disk write access and full network access,
/// returns `command` unchanged so we avoid unnecessary sandboxing overhead.
pub fn create_bwrap_command_args(
    command: Vec<String>,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    sandbox_policy_cwd: &Path,
    command_cwd: &Path,
    options: BwrapOptions,
) -> Result<BwrapArgs> {
    let unreadable_globs =
        file_system_sandbox_policy.get_unreadable_globs_with_cwd(sandbox_policy_cwd);

    if file_system_sandbox_policy.has_full_disk_write_access() && unreadable_globs.is_empty() {
        return if options.network_mode == BwrapNetworkMode::FullAccess {
            Ok(BwrapArgs {
                args: command,
                preserved_files: Vec::new(),
                synthetic_mount_targets: Vec::new(),
                protected_create_targets: Vec::new(),
            })
        } else {
            Ok(create_bwrap_flags_full_filesystem(command, options))
        };
    }

    create_bwrap_flags(
        command,
        file_system_sandbox_policy,
        sandbox_policy_cwd,
        command_cwd,
        options,
    )
}

fn create_bwrap_flags_full_filesystem(command: Vec<String>, options: BwrapOptions) -> BwrapArgs {
    let mut args = vec![
        "--new-session".to_string(),
        "--die-with-parent".to_string(),
        "--bind".to_string(),
        "/".to_string(),
        "/".to_string(),
        "--unshare-user".to_string(),
        "--unshare-pid".to_string(),
    ];
    if options.network_mode.should_unshare_network() {
        args.push("--unshare-net".to_string());
    }
    if options.mount_proc {
        args.push("--proc".to_string());
        args.push("/proc".to_string());
    }
    args.push("--".to_string());
    args.extend(command);
    BwrapArgs {
        args,
        preserved_files: Vec::new(),
        synthetic_mount_targets: Vec::new(),
        protected_create_targets: Vec::new(),
    }
}

fn create_bwrap_flags(
    command: Vec<String>,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    sandbox_policy_cwd: &Path,
    command_cwd: &Path,
    options: BwrapOptions,
) -> Result<BwrapArgs> {
    let BwrapArgs {
        args: filesystem_args,
        preserved_files,
        synthetic_mount_targets,
        protected_create_targets,
    } = create_filesystem_args(file_system_sandbox_policy, sandbox_policy_cwd, options)?;

    let normalized_command_cwd = normalize_command_cwd_for_bwrap(command_cwd);
    let mut args = Vec::new();
    args.push("--new-session".to_string());
    args.push("--die-with-parent".to_string());
    args.extend(filesystem_args);
    args.push("--unshare-user".to_string());
    args.push("--unshare-pid".to_string());
    if options.network_mode.should_unshare_network() {
        args.push("--unshare-net".to_string());
    }
    if options.mount_proc {
        args.push("--proc".to_string());
        args.push("/proc".to_string());
    }
    if normalized_command_cwd.as_path() != command_cwd {
        args.push("--chdir".to_string());
        args.push(path_to_string(normalized_command_cwd.as_path()));
    }
    args.push("--".to_string());
    args.extend(command);
    Ok(BwrapArgs {
        args,
        preserved_files,
        synthetic_mount_targets,
        protected_create_targets,
    })
}

/// Build the bubblewrap filesystem mounts for a given filesystem policy.
///
/// Mount order:
/// 1. Full-read uses `--ro-bind / /`; restricted starts from `--tmpfs /`.
/// 2. `--dev /dev` mounts minimal writable device nodes.
/// 3. Unreadable ancestors of writable roots are masked.
/// 4. `--bind <root> <root>` re-enables writes for allowed roots.
/// 5. `--ro-bind <subpath>` re-applies read-only protections.
/// 6. Remaining unreadable carveouts are masked.
fn create_filesystem_args(
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &Path,
    options: BwrapOptions,
) -> Result<BwrapArgs> {
    let unreadable_globs = file_system_sandbox_policy.get_unreadable_globs_with_cwd(cwd);

    let mut writable_roots = file_system_sandbox_policy
        .get_rich_writable_roots_with_cwd(cwd)
        .into_iter()
        .filter(|wr| wr.root.as_path().exists())
        .collect::<Vec<_>>();

    if writable_roots.is_empty()
        && file_system_sandbox_policy.has_full_disk_write_access()
        && !unreadable_globs.is_empty()
    {
        if let Ok(root_path) = AbsolutePathBuf::from_absolute_path("/") {
            writable_roots.push(WritableRoot::new(root_path));
        }
    }

    let missing_auto_metadata_read_only_project_root_subpaths: HashSet<PathBuf> =
        file_system_sandbox_policy
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::Read)
            .filter_map(|entry| {
                let FileSystemPath::Special {
                    value:
                        FileSystemSpecialPath::ProjectRoots {
                            subpath: Some(subpath),
                        },
                } = &entry.path
                else {
                    return None;
                };
                let project_subpath = subpath.as_path();
                if project_subpath != Path::new(".git")
                    && project_subpath != Path::new(".agents")
                    && project_subpath != Path::new(".xiaolin")
                {
                    return None;
                }
                let resolved = AbsolutePathBuf::resolve_path_against_base(subpath, cwd);
                (!resolved.as_path().exists()).then(|| resolved.into_path_buf())
            })
            .collect();

    let mut unreadable_roots = file_system_sandbox_policy
        .get_unreadable_roots_with_cwd(cwd)
        .into_iter()
        .map(AbsolutePathBuf::into_path_buf)
        .collect::<Vec<_>>();

    unreadable_roots.extend(
        expand_unreadable_globs_with_ripgrep(
            &unreadable_globs,
            cwd,
            options
                .glob_scan_max_depth
                .or(file_system_sandbox_policy.glob_scan_max_depth),
        )?
        .into_iter()
        .map(AbsolutePathBuf::into_path_buf),
    );
    unreadable_roots.sort();
    unreadable_roots.dedup();

    let args = if file_system_sandbox_policy.has_full_disk_read_access() {
        vec![
            "--ro-bind".to_string(),
            "/".to_string(),
            "/".to_string(),
            "--dev".to_string(),
            "/dev".to_string(),
        ]
    } else {
        let mut args = vec![
            "--tmpfs".to_string(),
            "/".to_string(),
            "--dev".to_string(),
            "/dev".to_string(),
        ];

        let mut readable_roots: BTreeSet<PathBuf> = file_system_sandbox_policy
            .get_readable_roots_with_cwd(cwd)
            .into_iter()
            .map(PathBuf::from)
            .collect();
        if file_system_sandbox_policy.include_platform_defaults() {
            readable_roots.extend(
                LINUX_PLATFORM_DEFAULT_READ_ROOTS
                    .iter()
                    .map(|path| PathBuf::from(*path))
                    .filter(|path| path.exists()),
            );
        }

        if readable_roots.iter().any(|root| root == Path::new("/")) {
            args = vec![
                "--ro-bind".to_string(),
                "/".to_string(),
                "/".to_string(),
                "--dev".to_string(),
                "/dev".to_string(),
            ];
        } else {
            for root in readable_roots {
                if !root.exists() {
                    continue;
                }
                let mount_root = if writable_roots
                    .iter()
                    .any(|wr| root.starts_with(wr.root.as_path()))
                {
                    canonical_target_if_symlinked_path(&root).unwrap_or(root)
                } else {
                    root
                };
                args.push("--ro-bind".to_string());
                args.push(path_to_string(&mount_root));
                args.push(path_to_string(&mount_root));
            }
        }

        args
    };

    let mut bwrap_args = BwrapArgs {
        args,
        preserved_files: Vec::new(),
        synthetic_mount_targets: Vec::new(),
        protected_create_targets: Vec::new(),
    };

    let mut allowed_write_paths = Vec::with_capacity(writable_roots.len());
    for writable_root in &writable_roots {
        let root = writable_root.root.as_path();
        allowed_write_paths.push(root.to_path_buf());
        if let Some(target) = canonical_target_if_symlinked_path(root) {
            allowed_write_paths.push(target);
        }
    }

    let unreadable_paths: HashSet<PathBuf> = unreadable_roots.iter().cloned().collect();
    let mut sorted_writable_roots = writable_roots;
    sorted_writable_roots.sort_by_key(|wr| path_depth(wr.root.as_path()));

    // Mask unreadable ancestors that sit outside every writable root.
    let mut unreadable_ancestors_of_writable_roots: Vec<PathBuf> = unreadable_roots
        .iter()
        .filter(|path| {
            let unreadable_root = path.as_path();
            !allowed_write_paths
                .iter()
                .any(|root| unreadable_root.starts_with(root))
                && allowed_write_paths
                    .iter()
                    .any(|root| root.starts_with(unreadable_root))
        })
        .cloned()
        .collect();
    unreadable_ancestors_of_writable_roots.sort_by_key(|path| path_depth(path));

    for unreadable_root in &unreadable_ancestors_of_writable_roots {
        append_unreadable_root_args(&mut bwrap_args, unreadable_root, &allowed_write_paths)?;
    }

    for writable_root in &sorted_writable_roots {
        let root = writable_root.root.as_path();
        let symlink_target = canonical_target_if_symlinked_path(root);

        if let Some(masking_root) = unreadable_roots
            .iter()
            .map(PathBuf::as_path)
            .filter(|unreadable_root| root.starts_with(unreadable_root))
            .max_by_key(|unreadable_root| path_depth(unreadable_root))
        {
            append_mount_target_parent_dir_args(&mut bwrap_args.args, root, masking_root);
        }

        let mount_root = symlink_target.as_deref().unwrap_or(root);
        bwrap_args.args.push("--bind".to_string());
        bwrap_args.args.push(path_to_string(mount_root));
        bwrap_args.args.push(path_to_string(mount_root));

        let mut read_only_subpaths: Vec<PathBuf> = writable_root
            .read_only_subpaths
            .iter()
            .map(|path| path.as_path().to_path_buf())
            .filter(|path| !unreadable_paths.contains(path))
            .filter(|path| !missing_auto_metadata_read_only_project_root_subpaths.contains(path))
            .collect();
        let protected_metadata_names = writable_root.protected_metadata_names.clone();

        append_metadata_path_masks_for_writable_root(
            &mut read_only_subpaths,
            root,
            mount_root,
            &protected_metadata_names,
        );
        if let Some(target) = &symlink_target {
            read_only_subpaths = remap_paths_for_symlink_target(read_only_subpaths, root, target);
        }
        append_protected_create_targets_for_writable_root(
            &mut bwrap_args,
            &protected_metadata_names,
            root,
            symlink_target.as_deref(),
            &read_only_subpaths,
        );
        read_only_subpaths.sort_by_key(|path| path_depth(path));
        for subpath in read_only_subpaths {
            append_read_only_subpath_args(&mut bwrap_args, &subpath, &allowed_write_paths)?;
        }

        let mut nested_unreadable_roots: Vec<PathBuf> = unreadable_roots
            .iter()
            .filter(|path| path.starts_with(root))
            .cloned()
            .collect();
        if let Some(target) = &symlink_target {
            nested_unreadable_roots =
                remap_paths_for_symlink_target(nested_unreadable_roots, root, target);
        }
        nested_unreadable_roots.sort_by_key(|path| path_depth(path));
        for unreadable_root in nested_unreadable_roots {
            append_unreadable_root_args(&mut bwrap_args, &unreadable_root, &allowed_write_paths)?;
        }
    }

    let mut rootless_unreadable_roots: Vec<PathBuf> = unreadable_roots
        .iter()
        .filter(|path| {
            let unreadable_root = path.as_path();
            !allowed_write_paths
                .iter()
                .any(|root| unreadable_root.starts_with(root) || root.starts_with(unreadable_root))
        })
        .cloned()
        .collect();
    rootless_unreadable_roots.sort_by_key(|path| path_depth(path));
    for unreadable_root in rootless_unreadable_roots {
        append_unreadable_root_args(&mut bwrap_args, &unreadable_root, &allowed_write_paths)?;
    }

    Ok(bwrap_args)
}

// ── Protected metadata helpers ──────────────────────────────────────────────

fn append_protected_create_targets_for_writable_root(
    bwrap_args: &mut BwrapArgs,
    protected_metadata_names: &[String],
    root: &Path,
    symlink_target: Option<&Path>,
    read_only_subpaths: &[PathBuf],
) {
    for name in protected_metadata_names {
        let mut path = root.join(name);
        if let Some(target) = symlink_target {
            if let Ok(relative_path) = path.strip_prefix(root) {
                path = target.join(relative_path);
            }
        }
        if read_only_subpaths.iter().any(|subpath| subpath == &path) || path.exists() {
            continue;
        }
        bwrap_args
            .protected_create_targets
            .push(ProtectedCreateTarget::missing(&path));
    }
}

fn append_metadata_path_masks_for_writable_root(
    read_only_subpaths: &mut Vec<PathBuf>,
    root: &Path,
    mount_root: &Path,
    protected_metadata_names: &[String],
) {
    for name in protected_metadata_names {
        let path = root.join(name);
        if should_leave_missing_git_for_parent_repo_discovery(mount_root, name) {
            continue;
        }
        if !read_only_subpaths.iter().any(|subpath| subpath == &path) {
            read_only_subpaths.push(path);
        }
    }
}

fn should_leave_missing_git_for_parent_repo_discovery(mount_root: &Path, name: &str) -> bool {
    let path = mount_root.join(name);
    name == ".git"
        && matches!(
            path.symlink_metadata(),
            Err(err) if err.kind() == io::ErrorKind::NotFound
        )
        && mount_root
            .ancestors()
            .skip(1)
            .any(ancestor_has_git_metadata)
}

fn ancestor_has_git_metadata(ancestor: &Path) -> bool {
    let git_path = ancestor.join(".git");
    let Ok(metadata) = git_path.symlink_metadata() else {
        return false;
    };
    if metadata.is_dir() {
        return git_path.join("HEAD").symlink_metadata().is_ok();
    }
    if metadata.is_file() {
        return fs::read_to_string(git_path)
            .is_ok_and(|contents| contents.trim_start().starts_with("gitdir:"));
    }
    false
}

// ── Glob expansion ──────────────────────────────────────────────────────────

fn expand_unreadable_globs_with_ripgrep(
    patterns: &[String],
    cwd: &Path,
    max_depth: Option<usize>,
) -> Result<Vec<AbsolutePathBuf>> {
    if patterns.is_empty() || max_depth == Some(0) {
        return Ok(Vec::new());
    }

    let mut patterns_by_search_root: BTreeMap<AbsolutePathBuf, Vec<String>> = BTreeMap::new();
    for pattern in patterns {
        if let Some((search_root, glob)) = split_pattern_for_ripgrep(pattern, cwd) {
            if search_root.as_path().is_dir() {
                patterns_by_search_root
                    .entry(search_root)
                    .or_default()
                    .push(glob);
            }
        }
    }

    let mut expanded_paths = BTreeSet::new();
    for (search_root, globs) in patterns_by_search_root {
        for path in ripgrep_files(search_root.as_path(), &globs, max_depth)? {
            if let Some(target) = canonical_target_if_symlinked_path(path.as_path()) {
                if let Ok(abs) = AbsolutePathBuf::from_absolute_path_checked(target) {
                    expanded_paths.insert(abs);
                }
            }
            expanded_paths.insert(path);
            if expanded_paths.len() > MAX_UNREADABLE_GLOB_MATCHES {
                bail!(
                    "unreadable glob expansion for {} matched more than {MAX_UNREADABLE_GLOB_MATCHES} paths",
                    search_root.display()
                );
            }
        }
    }

    Ok(expanded_paths.into_iter().collect())
}

fn split_pattern_for_ripgrep(pattern: &str, cwd: &Path) -> Option<(AbsolutePathBuf, String)> {
    let absolute_pattern = AbsolutePathBuf::resolve_path_against_base(pattern, cwd);
    let pattern = absolute_pattern.to_string_lossy();
    let first_glob_index = pattern
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '*' | '?' | '[' | ']').then_some(index))?;
    let static_prefix = &pattern[..first_glob_index];
    if static_prefix.is_empty() || static_prefix == "/" {
        return None;
    }
    let search_root_end = if static_prefix.ends_with('/') {
        static_prefix.len() - 1
    } else {
        static_prefix.rfind('/').unwrap_or(0)
    };
    let search_root = if search_root_end == 0 {
        PathBuf::from("/")
    } else {
        PathBuf::from(&pattern[..search_root_end])
    };
    let search_root = AbsolutePathBuf::from_absolute_path_checked(search_root).ok()?;
    let glob = escape_unclosed_glob_classes(&pattern[search_root_end + 1..]);
    (!glob.is_empty()).then_some((search_root, glob))
}

fn escape_unclosed_glob_classes(glob: &str) -> String {
    let mut escaped = String::with_capacity(glob.len());
    let mut chars = glob.chars();

    while let Some(ch) = chars.next() {
        if ch != '[' {
            escaped.push(ch);
            continue;
        }

        let mut class = String::new();
        let mut closed = false;
        for class_ch in chars.by_ref() {
            if class_ch == ']' {
                closed = true;
                break;
            }
            class.push(class_ch);
        }

        if closed {
            escaped.push('[');
            escaped.push_str(&class);
            escaped.push(']');
        } else {
            escaped.push_str(r"\[");
            escaped.push_str(&class);
        }
    }

    escaped
}

fn ripgrep_files(
    search_root: &Path,
    globs: &[String],
    max_depth: Option<usize>,
) -> Result<Vec<AbsolutePathBuf>> {
    let mut command = Command::new("rg");
    command
        .arg("--files")
        .arg("--hidden")
        .arg("--no-ignore")
        .arg("--null");
    if let Some(max_depth) = max_depth {
        command.arg("--max-depth").arg(max_depth.to_string());
    }
    for glob in globs {
        command.arg("--glob").arg(glob);
    }
    command.arg("--").arg(search_root);

    let output = match command.output() {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return glob_files(search_root, globs, max_depth);
        }
        Err(err) => return Err(err.into()),
    };
    if !output.status.success() {
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return Ok(Vec::new());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ripgrep unreadable glob scan failed for {}: {stderr}",
            search_root.display()
        );
    }

    let paths = output
        .stdout
        .split(|byte| *byte == b'\0')
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = PathBuf::from(OsString::from_vec(path.to_vec()));
            if path.is_absolute() {
                path
            } else {
                search_root.join(path)
            }
        })
        .map(AbsolutePathBuf::from_absolute_path_checked)
        .collect::<io::Result<Vec<_>>>()?;
    Ok(paths)
}

fn glob_files(
    search_root: &Path,
    globs: &[String],
    max_depth: Option<usize>,
) -> Result<Vec<AbsolutePathBuf>> {
    use globset::{GlobBuilder, GlobSetBuilder};

    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        let g = GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .map_err(|err| {
                anyhow::anyhow!(
                    "unreadable glob pattern is invalid for {}: {err}",
                    search_root.display()
                )
            })?;
        builder.add(g);
    }
    let glob_set = builder.build().map_err(|err| {
        anyhow::anyhow!(
            "unreadable glob matcher failed for {}: {err}",
            search_root.display()
        )
    })?;

    let mut paths = Vec::new();
    collect_glob_files(search_root, search_root, &glob_set, max_depth, &mut paths)?;
    Ok(paths)
}

fn collect_glob_files(
    search_root: &Path,
    dir: &Path,
    glob_set: &globset::GlobSet,
    remaining_depth: Option<usize>,
    paths: &mut Vec<AbsolutePathBuf>,
) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let relative = path.strip_prefix(search_root).unwrap_or(path.as_path());

        if (file_type.is_file() || file_type.is_symlink()) && glob_set.is_match(relative) {
            paths.push(AbsolutePathBuf::from_absolute_path_checked(&path)?);
        }

        if !file_type.is_dir() {
            continue;
        }
        let remaining_depth = match remaining_depth {
            Some(0 | 1) => continue,
            Some(depth) => Some(depth - 1),
            None => None,
        };
        collect_glob_files(search_root, &path, glob_set, remaining_depth, paths)?;
    }
    Ok(())
}

// ── Symlink normalization ───────────────────────────────────────────────────

fn canonical_target_if_symlinked_path(path: &Path) -> Option<PathBuf> {
    let mut current = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::RootDir => {
                current.push(Path::new("/"));
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                current.pop();
                continue;
            }
            Component::Normal(part) => current.push(part),
            Component::Prefix(_) => continue,
        }

        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(_) => return None,
        };
        if metadata.file_type().is_symlink() {
            let target = fs::canonicalize(path).ok()?;
            if target.as_path() == path {
                return None;
            }
            return Some(target);
        }
    }
    None
}

fn remap_paths_for_symlink_target(paths: Vec<PathBuf>, root: &Path, target: &Path) -> Vec<PathBuf> {
    paths
        .into_iter()
        .map(|path| {
            if let Ok(relative) = path.strip_prefix(root) {
                target.join(relative)
            } else {
                path
            }
        })
        .collect()
}

// ── Path utilities ──────────────────────────────────────────────────────────

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn normalize_command_cwd_for_bwrap(command_cwd: &Path) -> PathBuf {
    command_cwd
        .canonicalize()
        .unwrap_or_else(|_| command_cwd.to_path_buf())
}

fn append_mount_target_parent_dir_args(args: &mut Vec<String>, mount_target: &Path, anchor: &Path) {
    let mount_target_dir = if mount_target.is_dir() {
        mount_target
    } else if let Some(parent) = mount_target.parent() {
        parent
    } else {
        return;
    };
    let mut mount_target_dirs: Vec<PathBuf> = mount_target_dir
        .ancestors()
        .take_while(|path| *path != anchor)
        .map(Path::to_path_buf)
        .collect();
    mount_target_dirs.reverse();
    for dir in mount_target_dirs {
        args.push("--dir".to_string());
        args.push(path_to_string(&dir));
    }
}

// ── Read-only subpath handling ──────────────────────────────────────────────

fn append_read_only_subpath_args(
    bwrap_args: &mut BwrapArgs,
    subpath: &Path,
    allowed_write_paths: &[PathBuf],
) -> Result<()> {
    if let Some(symlink) = first_writable_symlink_component_in_path(subpath, allowed_write_paths) {
        bail!(
            "cannot enforce sandbox read-only path {} because it crosses writable symlink {}",
            subpath.display(),
            symlink.display()
        );
    }

    if let Some(metadata) = transient_empty_metadata_path(subpath) {
        if is_within_allowed_write_paths(subpath, allowed_write_paths) {
            match metadata {
                EmptyProtectedMetadataPath::File(metadata) => {
                    append_existing_empty_file_bind_data_args(bwrap_args, subpath, &metadata)?;
                }
                EmptyProtectedMetadataPath::Directory(metadata) => {
                    append_existing_empty_directory_args(bwrap_args, subpath, &metadata);
                }
            }
            return Ok(());
        }
    }

    if !subpath.exists() {
        if let Some(first_missing_component) = find_first_non_existent_component(subpath) {
            if is_within_allowed_write_paths(&first_missing_component, allowed_write_paths) {
                append_missing_read_only_subpath_args(bwrap_args, &first_missing_component)?;
            }
        }
        return Ok(());
    }

    if is_within_allowed_write_paths(subpath, allowed_write_paths) {
        bwrap_args.args.push("--ro-bind".to_string());
        bwrap_args.args.push(path_to_string(subpath));
        bwrap_args.args.push(path_to_string(subpath));
    }
    Ok(())
}

fn append_empty_file_bind_data_args(bwrap_args: &mut BwrapArgs, path: &Path) -> Result<()> {
    if bwrap_args.preserved_files.is_empty() {
        bwrap_args.preserved_files.push(File::open("/dev/null")?);
    }
    let null_fd = bwrap_args.preserved_files[0].as_raw_fd().to_string();
    bwrap_args.args.push("--ro-bind-data".to_string());
    bwrap_args.args.push(null_fd);
    bwrap_args.args.push(path_to_string(path));
    Ok(())
}

fn append_empty_directory_args(bwrap_args: &mut BwrapArgs, path: &Path) {
    bwrap_args.args.push("--perms".to_string());
    bwrap_args.args.push("555".to_string());
    bwrap_args.args.push("--tmpfs".to_string());
    bwrap_args.args.push(path_to_string(path));
    bwrap_args.args.push("--remount-ro".to_string());
    bwrap_args.args.push(path_to_string(path));
}

fn append_missing_read_only_subpath_args(bwrap_args: &mut BwrapArgs, path: &Path) -> Result<()> {
    if path.file_name().is_some_and(is_protected_metadata_name) {
        append_empty_directory_args(bwrap_args, path);
        bwrap_args
            .synthetic_mount_targets
            .push(SyntheticMountTarget::missing_empty_directory(path));
        return Ok(());
    }

    append_missing_empty_file_bind_data_args(bwrap_args, path)
}

fn append_missing_empty_file_bind_data_args(bwrap_args: &mut BwrapArgs, path: &Path) -> Result<()> {
    append_empty_file_bind_data_args(bwrap_args, path)?;
    bwrap_args
        .synthetic_mount_targets
        .push(SyntheticMountTarget::missing(path));
    Ok(())
}

fn append_existing_empty_file_bind_data_args(
    bwrap_args: &mut BwrapArgs,
    path: &Path,
    metadata: &Metadata,
) -> Result<()> {
    append_empty_file_bind_data_args(bwrap_args, path)?;
    bwrap_args
        .synthetic_mount_targets
        .push(SyntheticMountTarget::existing_empty_file(path, metadata));
    Ok(())
}

fn append_existing_empty_directory_args(
    bwrap_args: &mut BwrapArgs,
    path: &Path,
    metadata: &Metadata,
) {
    append_empty_directory_args(bwrap_args, path);
    bwrap_args
        .synthetic_mount_targets
        .push(SyntheticMountTarget::existing_empty_directory(
            path, metadata,
        ));
}

// ── Unreadable root handling ────────────────────────────────────────────────

fn append_unreadable_root_args(
    bwrap_args: &mut BwrapArgs,
    unreadable_root: &Path,
    allowed_write_paths: &[PathBuf],
) -> Result<()> {
    if let Some(symlink) =
        first_writable_symlink_component_in_path(unreadable_root, allowed_write_paths)
    {
        bail!(
            "cannot enforce sandbox deny-read path {} because it crosses writable symlink {}",
            unreadable_root.display(),
            symlink.display()
        );
    }

    if !unreadable_root.exists() {
        if let Some(first_missing_component) = find_first_non_existent_component(unreadable_root) {
            if is_within_allowed_write_paths(&first_missing_component, allowed_write_paths) {
                append_missing_empty_file_bind_data_args(bwrap_args, &first_missing_component)?;
            }
        }
        return Ok(());
    }

    append_existing_unreadable_path_args(bwrap_args, unreadable_root, allowed_write_paths)
}

fn append_existing_unreadable_path_args(
    bwrap_args: &mut BwrapArgs,
    unreadable_root: &Path,
    allowed_write_paths: &[PathBuf],
) -> Result<()> {
    if unreadable_root.is_dir() {
        let mut writable_descendants: Vec<&Path> = allowed_write_paths
            .iter()
            .map(PathBuf::as_path)
            .filter(|path| *path != unreadable_root && path.starts_with(unreadable_root))
            .collect();
        bwrap_args.args.push("--perms".to_string());
        bwrap_args.args.push(if writable_descendants.is_empty() {
            "000".to_string()
        } else {
            "111".to_string()
        });
        bwrap_args.args.push("--tmpfs".to_string());
        bwrap_args.args.push(path_to_string(unreadable_root));
        writable_descendants.sort_by_key(|path| path_depth(path));
        for writable_descendant in writable_descendants {
            append_mount_target_parent_dir_args(
                &mut bwrap_args.args,
                writable_descendant,
                unreadable_root,
            );
        }
        bwrap_args.args.push("--remount-ro".to_string());
        bwrap_args.args.push(path_to_string(unreadable_root));
        return Ok(());
    }

    bwrap_args.args.push("--perms".to_string());
    bwrap_args.args.push("000".to_string());
    append_empty_file_bind_data_args(bwrap_args, unreadable_root)
}

// ── Safety helpers ──────────────────────────────────────────────────────────

fn is_within_allowed_write_paths(path: &Path, allowed_write_paths: &[PathBuf]) -> bool {
    allowed_write_paths
        .iter()
        .any(|root| path.starts_with(root))
}

enum EmptyProtectedMetadataPath {
    File(Metadata),
    Directory(Metadata),
}

fn transient_empty_metadata_path(path: &Path) -> Option<EmptyProtectedMetadataPath> {
    if !path.file_name().is_some_and(is_protected_metadata_name) {
        return None;
    }

    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_file() && metadata.len() == 0 {
        return Some(EmptyProtectedMetadataPath::File(metadata));
    }

    if metadata.file_type().is_dir() && directory_is_empty(path) {
        return Some(EmptyProtectedMetadataPath::Directory(metadata));
    }

    None
}

fn directory_is_empty(path: &Path) -> bool {
    let Ok(mut entries) = fs::read_dir(path) else {
        return false;
    };
    entries.next().is_none()
}

fn first_writable_symlink_component_in_path(
    target_path: &Path,
    allowed_write_paths: &[PathBuf],
) -> Option<PathBuf> {
    let mut current = PathBuf::new();

    for component in target_path.components() {
        use std::path::Component;
        match component {
            Component::RootDir => {
                current.push(Path::new("/"));
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                current.pop();
                continue;
            }
            Component::Normal(part) => current.push(part),
            Component::Prefix(_) => continue,
        }

        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(_) => break,
        };

        if metadata.file_type().is_symlink()
            && is_within_allowed_write_paths(&current, allowed_write_paths)
        {
            return Some(current);
        }
    }

    None
}

fn find_first_non_existent_component(target_path: &Path) -> Option<PathBuf> {
    let mut current = PathBuf::new();

    for component in target_path.components() {
        use std::path::Component;
        match component {
            Component::RootDir => {
                current.push(Path::new("/"));
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                current.pop();
                continue;
            }
            Component::Normal(part) => current.push(part),
            Component::Prefix(_) => continue,
        }

        if !current.exists() {
            return Some(current);
        }
    }

    None
}

/// Execute the child command within a bubblewrap sandbox.
/// This replaces the current process via exec.
pub fn exec_with_bwrap(
    bwrap_args: BwrapArgs,
    bwrap_path: Option<&str>,
) -> Result<()> {
    let bwrap_exe = match bwrap_path {
        Some(p) => p.to_string(),
        None => find_bwrap()?,
    };

    tracing::info!(
        "exec bwrap: {} {}",
        bwrap_exe,
        bwrap_args.args.join(" ")
    );

    let program = std::ffi::CString::new(bwrap_exe.as_bytes())
        .map_err(|e| anyhow::anyhow!("invalid bwrap path: {e}"))?;
    let mut all_args = vec![program.clone()];
    for a in &bwrap_args.args {
        all_args.push(
            std::ffi::CString::new(a.as_bytes())
                .map_err(|e| anyhow::anyhow!("invalid argument: {e}"))?,
        );
    }

    // Keep preserved files alive across exec
    let _preserved = bwrap_args.preserved_files;

    nix::unistd::execvp(&program, &all_args)?;
    unreachable!()
}

fn find_bwrap() -> Result<String> {
    let candidates = ["/usr/bin/bwrap", "/usr/local/bin/bwrap"];
    for path in &candidates {
        if Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }

    if let Ok(found) = which::which("bwrap") {
        return Ok(found.to_string_lossy().to_string());
    }

    bail!("bubblewrap (bwrap) not found; install it or use landlock-only mode")
}

/// Clean up synthetic mount targets created for bwrap after the sandbox exits.
pub fn cleanup_synthetic_mount_targets(targets: &[SyntheticMountTarget]) {
    for target in targets.iter().rev() {
        let path = target.path();
        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !target.should_remove_after_bwrap(&metadata) {
            continue;
        }
        match target.kind() {
            SyntheticMountTargetKind::EmptyFile => {
                let _ = fs::remove_file(path);
            }
            SyntheticMountTargetKind::EmptyDirectory => {
                let _ = fs::remove_dir(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_security::permission_profile::FileSystemSandboxPolicy;

    fn test_cwd() -> PathBuf {
        PathBuf::from("/tmp/test")
    }

    fn unrestricted_policy() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::unrestricted()
    }

    #[test]
    fn full_disk_write_full_network_passes_through() {
        let policy = unrestricted_policy();
        let result = create_bwrap_command_args(
            vec!["echo".into(), "hello".into()],
            &policy,
            &test_cwd(),
            &test_cwd(),
            BwrapOptions::default(),
        )
        .unwrap();
        assert_eq!(result.args, vec!["echo", "hello"]);
        assert!(result.synthetic_mount_targets.is_empty());
    }

    #[test]
    fn full_disk_write_isolated_network_wraps() {
        let policy = unrestricted_policy();
        let result = create_bwrap_command_args(
            vec!["echo".into()],
            &policy,
            &test_cwd(),
            &test_cwd(),
            BwrapOptions {
                network_mode: BwrapNetworkMode::Isolated,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result.args.contains(&"--unshare-net".to_string()));
        assert!(result.args.contains(&"--bind".to_string()));
    }

    #[test]
    fn restricted_policy_has_ro_bind_or_tmpfs() {
        let policy = FileSystemSandboxPolicy::read_only();
        let result = create_bwrap_command_args(
            vec!["ls".into()],
            &policy,
            Path::new("/"),
            Path::new("/"),
            BwrapOptions::default(),
        )
        .unwrap();
        let has_ro = result.args.contains(&"--ro-bind".to_string());
        let has_tmpfs = result.args.contains(&"--tmpfs".to_string());
        assert!(has_ro || has_tmpfs);
    }

    #[test]
    fn bwrap_options_default() {
        let opts = BwrapOptions::default();
        assert!(opts.mount_proc);
        assert_eq!(opts.network_mode, BwrapNetworkMode::FullAccess);
        assert!(opts.glob_scan_max_depth.is_none());
    }

    #[test]
    fn network_mode_should_unshare() {
        assert!(!BwrapNetworkMode::FullAccess.should_unshare_network());
        assert!(BwrapNetworkMode::Isolated.should_unshare_network());
        assert!(BwrapNetworkMode::ProxyOnly.should_unshare_network());
    }

    #[test]
    fn escape_unclosed_glob_classes_balanced() {
        assert_eq!(escape_unclosed_glob_classes("[abc]"), "[abc]");
        assert_eq!(escape_unclosed_glob_classes("foo[bar]baz"), "foo[bar]baz");
    }

    #[test]
    fn escape_unclosed_glob_classes_unclosed() {
        assert_eq!(escape_unclosed_glob_classes("[abc"), r"\[abc");
        assert_eq!(escape_unclosed_glob_classes("foo[bar"), r"foo\[bar");
    }

    #[test]
    fn path_depth_counts_components() {
        assert_eq!(path_depth(Path::new("/")), 1);
        assert_eq!(path_depth(Path::new("/home")), 2);
        assert_eq!(path_depth(Path::new("/home/user/project")), 4);
    }

    #[test]
    fn synthetic_mount_target_missing_should_remove() {
        let target = SyntheticMountTarget::missing(Path::new("/tmp/nonexistent"));
        assert!(!target.preserves_pre_existing_path());
    }

    #[test]
    fn protected_create_target_path() {
        let target = ProtectedCreateTarget::missing(Path::new("/workspace/.git"));
        assert_eq!(target.path(), Path::new("/workspace/.git"));
    }

    #[test]
    fn find_first_non_existent_in_existing_path() {
        assert!(find_first_non_existent_component(Path::new("/")).is_none());
        assert!(find_first_non_existent_component(Path::new("/tmp")).is_none());
    }

    #[test]
    fn find_first_non_existent_partial() {
        let result =
            find_first_non_existent_component(Path::new("/tmp/definitely_does_not_exist_12345"));
        assert!(result.is_some());
    }

    #[test]
    fn cleanup_synthetic_mount_targets_does_not_panic() {
        cleanup_synthetic_mount_targets(&[]);
    }
}
