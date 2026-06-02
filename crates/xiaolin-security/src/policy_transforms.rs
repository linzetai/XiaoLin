use std::path::{Component, Path, PathBuf};

use crate::{
    FileSystemAccessMode, FileSystemPath, FileSystemSandboxEntry, FileSystemSandboxKind,
    FileSystemSandboxPolicy, NetworkSandboxPolicy,
};

/// Canonicalize a path by resolving `.` and `..` components without following
/// symlinks. Returns `None` if the result would be empty (e.g. `..` past root).
pub fn canonicalize_preserving_symlinks(path: &Path) -> Option<PathBuf> {
    let mut components: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if let Some(last) = components.last() {
                    if !matches!(last, Component::RootDir) {
                        components.pop();
                    }
                }
            }
            Component::CurDir => {}
            other => components.push(other),
        }
    }
    let result: PathBuf = components.iter().collect();
    if result.as_os_str().is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Merge two optional max-depth values. `None` means unbounded, so
/// `None` wins. When both are `Some`, take the larger (more permissive).
fn merge_max_depth(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (None, _) | (_, None) => None,
        (Some(a), Some(b)) => Some(a.max(b)),
    }
}

// ---------------------------------------------------------------------------
// Effective sandbox policy functions
// ---------------------------------------------------------------------------

/// Compute the effective filesystem sandbox policy by merging a base
/// `FileSystemSandboxPolicy` with optional additional entries.
///
/// When both base and additional are restricted, the additional entries are
/// appended to the base (union of allowed paths).
pub fn effective_file_system_sandbox_policy(
    base: Option<&FileSystemSandboxPolicy>,
    additional: Option<&FileSystemSandboxPolicy>,
) -> FileSystemSandboxPolicy {
    let base = match base {
        Some(b) => b,
        None => return additional.cloned().unwrap_or_default(),
    };
    let extra = match additional {
        Some(a) => a,
        None => return base.clone(),
    };

    if base.kind == FileSystemSandboxKind::Restricted
        && extra.kind == FileSystemSandboxKind::Restricted
    {
        let mut entries = base.entries.clone();
        entries.extend(extra.entries.clone());
        FileSystemSandboxPolicy {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: merge_max_depth(
                base.glob_scan_max_depth,
                extra.glob_scan_max_depth,
            ),
            entries,
        }
    } else {
        base.clone()
    }
}

/// Compute the effective network sandbox policy by merging base and additional.
///
/// If either allows network, the result is `Enabled`.
pub fn effective_network_sandbox_policy(
    base: Option<NetworkSandboxPolicy>,
    additional: Option<NetworkSandboxPolicy>,
) -> NetworkSandboxPolicy {
    let base = base.unwrap_or(NetworkSandboxPolicy::Restricted);
    match additional {
        Some(extra) => merge_network_sandbox(base, extra),
        None => base,
    }
}

fn merge_network_sandbox(a: NetworkSandboxPolicy, b: NetworkSandboxPolicy) -> NetworkSandboxPolicy {
    if a.is_enabled() || b.is_enabled() {
        NetworkSandboxPolicy::Enabled
    } else {
        NetworkSandboxPolicy::Restricted
    }
}

// ---------------------------------------------------------------------------
// Normalize / Merge / Intersect
// ---------------------------------------------------------------------------

/// Normalize filesystem sandbox entries: canonicalize paths (preserving
/// symlinks), deduplicate, and reject glob patterns used for non-deny access.
pub fn normalize_fs_entries(
    entries: Vec<FileSystemSandboxEntry>,
) -> Result<Vec<FileSystemSandboxEntry>, String> {
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        if matches!(&entry.path, FileSystemPath::GlobPattern { .. })
            && entry.access != FileSystemAccessMode::None
        {
            return Err(
                "glob file system permissions only support deny-read entries".to_string(),
            );
        }
        let path = match entry.path {
            FileSystemPath::Path { path } => {
                let canonical = canonicalize_preserving_symlinks(path.as_path())
                    .and_then(|p| xiaolin_path::AbsolutePathBuf::from_absolute_path(p).ok())
                    .unwrap_or(path);
                FileSystemPath::Path { path: canonical }
            }
            other => other,
        };
        let normalized_entry = FileSystemSandboxEntry {
            path,
            access: entry.access,
        };
        if !normalized.contains(&normalized_entry) {
            normalized.push(normalized_entry);
        }
    }
    Ok(normalized)
}

/// Merge two sets of filesystem entries into a union. Entries are appended
/// and deduplicated.
pub fn merge_fs_entries(
    base: &[FileSystemSandboxEntry],
    additional: &[FileSystemSandboxEntry],
) -> Vec<FileSystemSandboxEntry> {
    let mut merged = base.to_vec();
    for entry in additional {
        if !merged.contains(entry) {
            merged.push(entry.clone());
        }
    }
    merged
}

/// Merge two `FileSystemSandboxPolicy` instances. The result preserves the
/// base kind and merges entries (union). If either is Unrestricted or
/// ExternalSandbox, the base takes precedence.
pub fn merge_file_system_policies(
    base: &FileSystemSandboxPolicy,
    additional: &FileSystemSandboxPolicy,
) -> FileSystemSandboxPolicy {
    if !matches!(base.kind, FileSystemSandboxKind::Restricted)
        || !matches!(additional.kind, FileSystemSandboxKind::Restricted)
    {
        return base.clone();
    }

    FileSystemSandboxPolicy {
        kind: FileSystemSandboxKind::Restricted,
        glob_scan_max_depth: merge_max_depth(
            base.glob_scan_max_depth,
            additional.glob_scan_max_depth,
        ),
        entries: merge_fs_entries(&base.entries, &additional.entries),
    }
}

/// Intersect a "requested" policy with a "granted" policy.
///
/// Returns a policy that only contains entries from `granted` that are
/// within the scope of `requested`. Deny entries from both sides are
/// preserved to maintain safety constraints.
///
/// This is used in approval flows: the user requests permissions, the
/// approver grants a subset, and the intersection ensures only the
/// approved subset is active.
pub fn intersect_fs_sandbox_policies(
    requested: &FileSystemSandboxPolicy,
    granted: &FileSystemSandboxPolicy,
    _cwd: &Path,
) -> FileSystemSandboxPolicy {
    if !matches!(requested.kind, FileSystemSandboxKind::Restricted)
        || !matches!(granted.kind, FileSystemSandboxKind::Restricted)
    {
        return granted.clone();
    }

    let mut entries: Vec<FileSystemSandboxEntry> = Vec::new();

    for granted_entry in &granted.entries {
        if granted_entry.access == FileSystemAccessMode::None {
            if !entries.contains(granted_entry) {
                entries.push(granted_entry.clone());
            }
            continue;
        }

        let is_within_request = requested.entries.iter().any(|req| {
            req.access.can_read() && entries_share_scope(&req.path, &granted_entry.path)
        });

        if is_within_request && !entries.contains(granted_entry) {
            entries.push(granted_entry.clone());
        }
    }

    for requested_entry in &requested.entries {
        if requested_entry.access == FileSystemAccessMode::None
            && !entries.contains(requested_entry)
        {
            entries.push(requested_entry.clone());
        }
    }

    FileSystemSandboxPolicy {
        kind: FileSystemSandboxKind::Restricted,
        glob_scan_max_depth: merge_max_depth(
            requested.glob_scan_max_depth,
            granted.glob_scan_max_depth,
        ),
        entries,
    }
}

/// Conservative scope check: two paths share scope when they are equal
/// or one is a prefix of the other.
fn entries_share_scope(a: &FileSystemPath, b: &FileSystemPath) -> bool {
    match (a, b) {
        (FileSystemPath::Path { path: pa }, FileSystemPath::Path { path: pb }) => {
            pa.as_path().starts_with(pb.as_path()) || pb.as_path().starts_with(pa.as_path())
        }
        (FileSystemPath::Special { value: va }, FileSystemPath::Special { value: vb }) => {
            va == vb
        }
        _ => false,
    }
}

/// Intersect network policies. Both must be Enabled for the result to
/// be Enabled; otherwise Restricted.
pub fn intersect_network_sandbox(
    requested: NetworkSandboxPolicy,
    granted: NetworkSandboxPolicy,
) -> NetworkSandboxPolicy {
    if requested.is_enabled() && granted.is_enabled() {
        NetworkSandboxPolicy::Enabled
    } else {
        NetworkSandboxPolicy::Restricted
    }
}

// ---------------------------------------------------------------------------
// AdditionalPermissionProfile transforms
// ---------------------------------------------------------------------------

/// Normalize an `AdditionalPermissionProfile`: canonicalize paths,
/// deduplicate entries, and reject glob patterns for non-deny access.
pub fn normalize_additional_permissions(
    mut profile: crate::AdditionalPermissionProfile,
) -> Result<crate::AdditionalPermissionProfile, String> {
    if let Some(ref mut fs) = profile.file_system {
        fs.entries = normalize_fs_entries(std::mem::take(&mut fs.entries))?;
    }
    Ok(profile)
}

/// Merge two optional `AdditionalPermissionProfile`s (union semantics).
///
/// - network: either `enabled=true` → result `enabled=true`
/// - file_system: entries are merged and deduplicated, `glob_scan_max_depth` takes max
pub fn merge_permission_profiles(
    a: Option<&crate::AdditionalPermissionProfile>,
    b: Option<&crate::AdditionalPermissionProfile>,
) -> Option<crate::AdditionalPermissionProfile> {
    match (a, b) {
        (None, None) => None,
        (Some(a), None) => Some(a.clone()),
        (None, Some(b)) => Some(b.clone()),
        (Some(a), Some(b)) => {
            let network = match (&a.network, &b.network) {
                (None, None) => None,
                (Some(n), None) | (None, Some(n)) => Some(n.clone()),
                (Some(na), Some(nb)) => Some(crate::NetworkPermissions {
                    enabled: match (na.enabled, nb.enabled) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => na.enabled.or(nb.enabled),
                    },
                }),
            };

            let file_system = match (&a.file_system, &b.file_system) {
                (None, None) => None,
                (Some(f), None) | (None, Some(f)) => Some(f.clone()),
                (Some(fa), Some(fb)) => {
                    let entries = merge_fs_entries(&fa.entries, &fb.entries);
                    let depth = match (
                        fa.glob_scan_max_depth.map(|d| d.get()),
                        fb.glob_scan_max_depth.map(|d| d.get()),
                    ) {
                        (None, _) | (_, None) => None,
                        (Some(a), Some(b)) => std::num::NonZeroUsize::new(a.max(b)),
                    };
                    Some(crate::FileSystemPermissions {
                        entries,
                        glob_scan_max_depth: depth,
                    })
                }
            };

            let result = crate::AdditionalPermissionProfile {
                network,
                file_system,
            };
            if result.is_empty() { None } else { Some(result) }
        }
    }
}

/// Intersect a "requested" `AdditionalPermissionProfile` with a "granted" one.
///
/// Approval flow: requested A, approved B, effective = A ∩ B.
/// - For file_system: only granted entries within the requested scope survive.
/// - For network: both must be `enabled=true` for the result to be enabled.
pub fn intersect_permission_profiles(
    requested: Option<&crate::AdditionalPermissionProfile>,
    granted: Option<&crate::AdditionalPermissionProfile>,
    _cwd: &Path,
) -> Option<crate::AdditionalPermissionProfile> {
    match (requested, granted) {
        (None, _) | (_, None) => None,
        (Some(req), Some(grt)) => {
            let network = match (&req.network, &grt.network) {
                (Some(rn), Some(gn)) => {
                    let enabled = match (rn.enabled, gn.enabled) {
                        (Some(true), Some(true)) => Some(true),
                        _ => Some(false),
                    };
                    Some(crate::NetworkPermissions { enabled })
                }
                _ => None,
            };

            let file_system = match (&req.file_system, &grt.file_system) {
                (Some(rf), Some(gf)) => {
                    let mut entries: Vec<FileSystemSandboxEntry> = Vec::new();

                    for granted_entry in &gf.entries {
                        if granted_entry.access == FileSystemAccessMode::None {
                            if !entries.contains(granted_entry) {
                                entries.push(granted_entry.clone());
                            }
                            continue;
                        }

                        let is_within = rf.entries.iter().any(|re| {
                            re.access.can_read()
                                && entries_share_scope(&re.path, &granted_entry.path)
                        });
                        if is_within && !entries.contains(granted_entry) {
                            entries.push(granted_entry.clone());
                        }
                    }

                    // Preserve deny entries from requested
                    for req_entry in &rf.entries {
                        if req_entry.access == FileSystemAccessMode::None
                            && !entries.contains(req_entry)
                        {
                            entries.push(req_entry.clone());
                        }
                    }

                    let depth = match (
                        rf.glob_scan_max_depth.map(|d| d.get()),
                        gf.glob_scan_max_depth.map(|d| d.get()),
                    ) {
                        (None, _) | (_, None) => None,
                        (Some(a), Some(b)) => std::num::NonZeroUsize::new(a.min(b)),
                    };

                    if entries.is_empty() {
                        None
                    } else {
                        Some(crate::FileSystemPermissions {
                            entries,
                            glob_scan_max_depth: depth,
                        })
                    }
                }
                _ => None,
            };

            let result = crate::AdditionalPermissionProfile {
                network,
                file_system,
            };
            if result.is_empty() { None } else { Some(result) }
        }
    }
}

/// Higher-level function: apply `AdditionalPermissionProfile` to a base
/// `PermissionProfile`, producing a new profile with the additional
/// permissions merged in.
pub fn effective_permission_profile(
    base: &crate::PermissionProfile,
    additional: Option<&crate::AdditionalPermissionProfile>,
) -> crate::PermissionProfile {
    let additional = match additional {
        Some(a) if !a.is_empty() => a,
        _ => return base.clone(),
    };

    let (mut fs_policy, net_policy) = base.to_runtime_permissions();

    // Merge file system
    if let Some(ref fs_perms) = additional.file_system {
        if matches!(fs_policy.kind, FileSystemSandboxKind::Restricted) && !fs_perms.entries.is_empty() {
            let mut entries = fs_policy.entries;
            for entry in &fs_perms.entries {
                if !entries.contains(entry) {
                    entries.push(entry.clone());
                }
            }
            let depth = merge_max_depth(
                fs_policy.glob_scan_max_depth,
                fs_perms.glob_scan_max_depth.map(|d| d.get()),
            );
            fs_policy = FileSystemSandboxPolicy {
                kind: FileSystemSandboxKind::Restricted,
                glob_scan_max_depth: depth,
                entries,
            };
        }
    }

    // Merge network
    let net = match &additional.network {
        Some(np) if np.enabled == Some(true) => NetworkSandboxPolicy::Enabled,
        _ => net_policy,
    };

    crate::PermissionProfile::from_runtime_permissions_with_enforcement(
        base.enforcement(),
        &fs_policy,
        net,
    )
}

/// Determine whether the platform-level sandbox should be activated.
/// Returns `false` if the filesystem policy is unrestricted/external or grants
/// full disk write with no deny restrictions, since sandboxing would add no value.
pub fn should_require_platform_sandbox(
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
) -> bool {
    match fs_policy.kind {
        FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => false,
        FileSystemSandboxKind::Restricted => {
            if !net_policy.is_enabled() || fs_policy.has_denied_read_restrictions() {
                return true;
            }

            !fs_policy.has_full_disk_write_access()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileSystemSpecialPath;
    use xiaolin_path::test_support::{test_path_buf, PathBufExt};

    #[test]
    fn canonicalize_resolves_dotdot() {
        let result = canonicalize_preserving_symlinks(Path::new("/home/user/../other"));
        assert_eq!(result, Some(PathBuf::from("/home/other")));
    }

    #[test]
    fn canonicalize_resolves_dot() {
        let result = canonicalize_preserving_symlinks(Path::new("/home/./user"));
        assert_eq!(result, Some(PathBuf::from("/home/user")));
    }

    #[test]
    fn canonicalize_preserves_normal_paths() {
        let result = canonicalize_preserving_symlinks(Path::new("/home/user/project"));
        assert_eq!(result, Some(PathBuf::from("/home/user/project")));
    }

    #[test]
    fn canonicalize_dotdot_at_root_stays_at_root() {
        let result = canonicalize_preserving_symlinks(Path::new("/../foo"));
        assert_eq!(result, Some(PathBuf::from("/foo")));
    }

    // --- effective policy functions tests ---

    #[test]
    fn effective_fs_policy_merges_additional_entries() {
        let base = FileSystemSandboxPolicy::default();
        let additional = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/home/user/project").abs(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let effective =
            effective_file_system_sandbox_policy(Some(&base), Some(&additional));
        assert_eq!(effective.kind, FileSystemSandboxKind::Restricted);
        assert!(effective.entries.len() > base.entries.len());
    }

    #[test]
    fn effective_fs_policy_no_additional() {
        let base = FileSystemSandboxPolicy::default();
        let effective = effective_file_system_sandbox_policy(Some(&base), None);
        assert_eq!(effective, base);
    }

    #[test]
    fn effective_fs_policy_unrestricted_base_ignores_additional() {
        let base = FileSystemSandboxPolicy::unrestricted();
        let additional = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/tmp").abs(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let effective =
            effective_file_system_sandbox_policy(Some(&base), Some(&additional));
        assert_eq!(effective.kind, FileSystemSandboxKind::Unrestricted);
    }

    #[test]
    fn effective_net_policy_merges_enabled() {
        let effective = effective_network_sandbox_policy(
            Some(NetworkSandboxPolicy::Restricted),
            Some(NetworkSandboxPolicy::Enabled),
        );
        assert!(effective.is_enabled());
    }

    #[test]
    fn effective_net_policy_both_restricted() {
        let effective = effective_network_sandbox_policy(
            Some(NetworkSandboxPolicy::Restricted),
            Some(NetworkSandboxPolicy::Restricted),
        );
        assert!(!effective.is_enabled());
    }

    #[test]
    fn should_require_sandbox_unrestricted_returns_false() {
        let fs = FileSystemSandboxPolicy::unrestricted();
        assert!(!should_require_platform_sandbox(&fs, NetworkSandboxPolicy::Restricted));
    }

    #[test]
    fn should_require_sandbox_external_returns_false() {
        let fs = FileSystemSandboxPolicy::external_sandbox();
        assert!(!should_require_platform_sandbox(&fs, NetworkSandboxPolicy::Enabled));
    }

    #[test]
    fn should_require_sandbox_restricted_no_write_root() {
        let fs = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        }]);
        assert!(should_require_platform_sandbox(&fs, NetworkSandboxPolicy::Enabled));
    }

    #[test]
    fn should_require_sandbox_network_disabled() {
        let fs = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Write,
        }]);
        assert!(should_require_platform_sandbox(&fs, NetworkSandboxPolicy::Restricted));
    }

    #[test]
    fn should_require_sandbox_full_write_no_deny_network_enabled() {
        let fs = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Write,
        }]);
        assert!(!should_require_platform_sandbox(&fs, NetworkSandboxPolicy::Enabled));
    }

    #[test]
    fn should_require_sandbox_full_write_with_deny_entries() {
        let fs = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
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
        assert!(should_require_platform_sandbox(&fs, NetworkSandboxPolicy::Enabled));
    }

    // --- normalize / merge / intersect tests ---

    #[test]
    fn normalize_rejects_glob_with_write_access() {
        let entries = vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "**/*.txt".into(),
            },
            access: FileSystemAccessMode::Write,
        }];
        assert!(normalize_fs_entries(entries).is_err());
    }

    #[test]
    fn normalize_allows_glob_with_none_access() {
        let entries = vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: "**/.env".into(),
            },
            access: FileSystemAccessMode::None,
        }];
        let result = normalize_fs_entries(entries);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn normalize_deduplicates_entries() {
        let entry = FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/tmp").abs(),
            },
            access: FileSystemAccessMode::Write,
        };
        let entries = vec![entry.clone(), entry];
        let result = normalize_fs_entries(entries).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn merge_fs_entries_union() {
        let base = vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/a").abs(),
            },
            access: FileSystemAccessMode::Write,
        }];
        let additional = vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/b").abs(),
            },
            access: FileSystemAccessMode::Write,
        }];
        let merged = merge_fs_entries(&base, &additional);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_fs_entries_deduplicates() {
        let entry = FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/a").abs(),
            },
            access: FileSystemAccessMode::Write,
        };
        let merged = merge_fs_entries(&[entry.clone()], &[entry]);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn merge_policies_restricted() {
        let base = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/a").abs(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let additional = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/b").abs(),
            },
            access: FileSystemAccessMode::Read,
        }]);
        let merged = merge_file_system_policies(&base, &additional);
        assert_eq!(merged.entries.len(), 2);
    }

    #[test]
    fn intersect_preserves_deny_entries() {
        let requested = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: test_path_buf("/home").abs(),
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
        let granted = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/home").abs(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let result = intersect_fs_sandbox_policies(&requested, &granted, Path::new("/"));
        assert!(result.entries.iter().any(|e| e.access == FileSystemAccessMode::None));
        assert!(result.entries.iter().any(|e| e.access == FileSystemAccessMode::Write));
    }

    #[test]
    fn intersect_filters_non_requested() {
        let requested = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: test_path_buf("/home").abs(),
            },
            access: FileSystemAccessMode::Write,
        }]);
        let granted = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: test_path_buf("/home").abs(),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: test_path_buf("/etc").abs(),
                },
                access: FileSystemAccessMode::Write,
            },
        ]);
        let result = intersect_fs_sandbox_policies(&requested, &granted, Path::new("/"));
        assert!(result.entries.iter().any(|e| matches!(&e.path,
            FileSystemPath::Path { path } if path.as_path() == Path::new("/home")
        )));
        assert!(!result.entries.iter().any(|e| matches!(&e.path,
            FileSystemPath::Path { path } if path.as_path() == Path::new("/etc")
        )));
    }

    #[test]
    fn intersect_network_both_enabled() {
        assert_eq!(
            intersect_network_sandbox(NetworkSandboxPolicy::Enabled, NetworkSandboxPolicy::Enabled),
            NetworkSandboxPolicy::Enabled,
        );
    }

    #[test]
    fn intersect_network_one_restricted() {
        assert_eq!(
            intersect_network_sandbox(
                NetworkSandboxPolicy::Enabled,
                NetworkSandboxPolicy::Restricted
            ),
            NetworkSandboxPolicy::Restricted,
        );
    }
}
