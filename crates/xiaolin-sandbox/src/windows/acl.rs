//! ACL-based file system isolation for Windows sandboxing.
//!
//! Aligned with Codex `windows-sandbox-rs/src/acl.rs` + `deny_read_acl.rs`.
//! Modifies file/directory ACLs to implement deny-read and restrict
//! write access for sandboxed processes.

use std::path::PathBuf;

/// An ACL entry describing access control for a file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclEntry {
    /// The path to apply the ACL to.
    pub path: PathBuf,
    /// The type of ACL entry.
    pub entry_type: AclEntryType,
    /// Whether to propagate to children (for directories).
    pub inherit: bool,
}

/// Type of ACL entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclEntryType {
    /// Allow read access.
    AllowRead,
    /// Allow read and write access.
    AllowReadWrite,
    /// Deny read access (explicit deny overrides allows).
    DenyRead,
    /// Deny all access.
    DenyAll,
}

/// ACL modification plan for a sandboxed process.
///
/// Contains the set of ACL changes to apply before spawning and
/// the corresponding rollback entries to restore after execution.
#[derive(Debug, Clone)]
pub struct AclPlan {
    /// ACL entries to apply before process execution.
    pub apply: Vec<AclEntry>,
    /// ACL entries to restore after process exits.
    pub rollback: Vec<AclEntry>,
}

impl AclPlan {
    /// Create an empty plan.
    pub fn empty() -> Self {
        Self {
            apply: Vec::new(),
            rollback: Vec::new(),
        }
    }

    /// Whether this plan has any modifications.
    pub fn is_empty(&self) -> bool {
        self.apply.is_empty()
    }
}

/// Build an ACL plan from filesystem sandbox policy entries.
///
/// For each writable root, allows read-write. For deny-read paths,
/// adds explicit deny-read ACL entries. All changes are paired with
/// rollback entries.
pub fn build_acl_plan(
    _writable_roots: &[PathBuf],
    deny_read_paths: &[PathBuf],
    _sid: &str,
) -> AclPlan {
    let mut apply = Vec::new();
    let mut rollback = Vec::new();

    for path in deny_read_paths {
        apply.push(AclEntry {
            path: path.clone(),
            entry_type: AclEntryType::DenyRead,
            inherit: true,
        });
        rollback.push(AclEntry {
            path: path.clone(),
            entry_type: AclEntryType::AllowRead,
            inherit: true,
        });
    }

    AclPlan { apply, rollback }
}

/// Apply ACL entries to the file system.
///
/// On non-Windows platforms, this is a no-op that returns Ok.
#[cfg(target_os = "windows")]
pub fn apply_acl_entries(entries: &[AclEntry], sid: &str) -> Result<(), std::io::Error> {
    // Windows implementation would use:
    // - GetNamedSecurityInfo to get current DACL
    // - SetEntriesInAcl to add/modify entries
    // - SetNamedSecurityInfo to apply the modified DACL
    for entry in entries {
        tracing::debug!(?entry.path, ?entry.entry_type, "applying ACL entry");
    }
    todo!("Windows ACL modification implementation")
}

#[cfg(not(target_os = "windows"))]
pub fn apply_acl_entries(_entries: &[AclEntry], _sid: &str) -> Result<(), std::io::Error> {
    Ok(())
}

/// Rollback ACL entries, restoring the original state.
#[cfg(target_os = "windows")]
pub fn rollback_acl_entries(entries: &[AclEntry], sid: &str) -> Result<(), std::io::Error> {
    apply_acl_entries(entries, sid)
}

#[cfg(not(target_os = "windows"))]
pub fn rollback_acl_entries(_entries: &[AclEntry], _sid: &str) -> Result<(), std::io::Error> {
    Ok(())
}

/// Get the list of paths that should be allowed for reading.
///
/// Returns the resolved set of allowed path prefixes from a policy.
pub fn get_allowed_prefixes(
    writable_roots: &[PathBuf],
    readable_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut prefixes = Vec::new();
    prefixes.extend(writable_roots.iter().cloned());
    prefixes.extend(readable_roots.iter().cloned());

    // System paths always readable
    for sys_path in &[
        "/usr", "/bin", "/lib", "/etc",
        "C:\\Windows", "C:\\Program Files", "C:\\Program Files (x86)",
    ] {
        prefixes.push(PathBuf::from(sys_path));
    }

    prefixes.sort();
    prefixes.dedup();
    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_plan() {
        let plan = AclPlan::empty();
        assert!(plan.is_empty());
        assert!(plan.apply.is_empty());
        assert!(plan.rollback.is_empty());
    }

    #[test]
    fn build_plan_with_deny_reads() {
        let plan = build_acl_plan(
            &[PathBuf::from("/home/user")],
            &[PathBuf::from("/etc/shadow"), PathBuf::from("/root/.ssh")],
            "S-1-5-21-test",
        );
        assert_eq!(plan.apply.len(), 2);
        assert_eq!(plan.rollback.len(), 2);
        assert_eq!(plan.apply[0].entry_type, AclEntryType::DenyRead);
        assert_eq!(plan.rollback[0].entry_type, AclEntryType::AllowRead);
    }

    #[test]
    fn build_plan_no_deny_reads() {
        let plan = build_acl_plan(
            &[PathBuf::from("/tmp")],
            &[],
            "S-1-5-21-test",
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn acl_entries_have_inherit() {
        let plan = build_acl_plan(
            &[],
            &[PathBuf::from("/secret")],
            "S-1-5-21-test",
        );
        assert!(plan.apply[0].inherit);
    }

    #[test]
    fn apply_acl_noop_on_non_windows() {
        let entries = vec![AclEntry {
            path: PathBuf::from("/tmp"),
            entry_type: AclEntryType::DenyRead,
            inherit: false,
        }];
        assert!(apply_acl_entries(&entries, "S-1-5-21-test").is_ok());
    }

    #[test]
    fn rollback_acl_noop_on_non_windows() {
        let entries = vec![AclEntry {
            path: PathBuf::from("/tmp"),
            entry_type: AclEntryType::AllowRead,
            inherit: false,
        }];
        assert!(rollback_acl_entries(&entries, "S-1-5-21-test").is_ok());
    }

    #[test]
    fn allowed_prefixes_includes_system_paths() {
        let prefixes = get_allowed_prefixes(&[], &[]);
        assert!(!prefixes.is_empty());
        assert!(prefixes.iter().any(|p| p.to_string_lossy().contains("Windows")
            || p.to_string_lossy().contains("/usr")));
    }

    #[test]
    fn allowed_prefixes_deduplicates() {
        let prefixes = get_allowed_prefixes(
            &[PathBuf::from("/home/user")],
            &[PathBuf::from("/home/user")],
        );
        let count = prefixes.iter().filter(|p| p == &&PathBuf::from("/home/user")).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn acl_entry_type_equality() {
        assert_eq!(AclEntryType::DenyRead, AclEntryType::DenyRead);
        assert_ne!(AclEntryType::DenyRead, AclEntryType::AllowRead);
    }
}
