//! Restricted token creation for Windows sandboxing.
//!
//! Aligned with Codex `windows-sandbox-rs/src/token.rs` + `identity.rs`.
//! Creates low-privilege process tokens by:
//! - Disabling all privileges except SeChangeNotifyPrivilege
//! - Adding deny-only SIDs for high-privilege groups
//! - Optionally setting integrity level to Low or Untrusted

use serde::{Deserialize, Serialize};


/// Privilege to retain in restricted tokens (traverse checking bypass).
pub const SE_CHANGE_NOTIFY_NAME: &str = "SeChangeNotifyPrivilege";

/// SIDs commonly marked as deny-only in restricted tokens.
pub const DENY_ONLY_SIDS: &[&str] = &[
    "S-1-5-32-544", // BUILTIN\\Administrators
    "S-1-5-32-545", // BUILTIN\\Users
    "S-1-5-4",      // NT AUTHORITY\\INTERACTIVE
    "S-1-5-11",     // NT AUTHORITY\\Authenticated Users
    "S-1-2-0",      // LOCAL
];

/// Windows integrity levels for process tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntegrityLevel {
    /// System integrity (highest).
    System,
    /// High integrity (admin).
    High,
    /// Medium integrity (default user).
    Medium,
    /// Low integrity (sandboxed).
    Low,
    /// Untrusted integrity (most restricted).
    Untrusted,
}

impl IntegrityLevel {
    /// Windows RID value for this integrity level.
    pub fn rid(self) -> u32 {
        match self {
            Self::System => 0x4000,
            Self::High => 0x3000,
            Self::Medium => 0x2000,
            Self::Low => 0x1000,
            Self::Untrusted => 0x0000,
        }
    }

    /// SID string representation.
    pub fn sid_string(self) -> String {
        format!("S-1-16-{}", self.rid())
    }
}

/// Configuration for creating a restricted process token.
#[derive(Debug, Clone)]
pub struct RestrictedTokenConfig {
    /// SIDs to mark as deny-only (prevents elevation).
    pub deny_only_sids: Vec<String>,
    /// Privileges to delete (all except retained ones).
    pub privileges_to_delete: Vec<String>,
    /// Privileges to retain.
    pub privileges_to_retain: Vec<String>,
    /// Restricting SIDs to add.
    pub restricting_sids: Vec<String>,
    /// Target integrity level.
    pub integrity_level: IntegrityLevel,
    /// Use a job object for additional containment.
    pub use_job_object: bool,
    /// Limit the desktop to the process.
    pub use_alternate_desktop: bool,
}

impl Default for RestrictedTokenConfig {
    fn default() -> Self {
        Self {
            deny_only_sids: DENY_ONLY_SIDS.iter().map(|s| s.to_string()).collect(),
            privileges_to_delete: Vec::new(),
            privileges_to_retain: vec![SE_CHANGE_NOTIFY_NAME.to_string()],
            restricting_sids: Vec::new(),
            integrity_level: IntegrityLevel::Low,
            use_job_object: true,
            use_alternate_desktop: false,
        }
    }
}

impl RestrictedTokenConfig {
    /// Configuration for standard sandbox level.
    pub fn standard() -> Self {
        Self::default()
    }

    /// Configuration for strict sandbox level.
    pub fn strict() -> Self {
        Self {
            integrity_level: IntegrityLevel::Untrusted,
            use_job_object: true,
            use_alternate_desktop: true,
            ..Self::default()
        }
    }
}

/// Represents a created restricted token handle (Windows-only actual handle).
#[derive(Debug)]
pub struct RestrictedToken {
    pub config: RestrictedTokenConfig,
    #[cfg(target_os = "windows")]
    pub handle: std::os::windows::io::RawHandle,
}

/// Create a restricted process token on Windows.
///
/// On non-Windows platforms, returns an error.
#[cfg(target_os = "windows")]
pub fn create_restricted_token(
    config: &RestrictedTokenConfig,
) -> Result<RestrictedToken, std::io::Error> {
    // Windows implementation would use:
    // - OpenProcessToken
    // - CreateRestrictedToken with deny_only_sids and restricting_sids
    // - SetTokenInformation for integrity level
    // - DeletePrivilegeFromToken for privilege removal
    todo!("Windows CreateRestrictedToken implementation")
}

#[cfg(not(target_os = "windows"))]
pub fn create_restricted_token(
    _config: &RestrictedTokenConfig,
) -> Result<RestrictedToken, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "restricted tokens are only available on Windows",
    ))
}

/// Job object configuration for process containment.
#[derive(Debug, Clone)]
pub struct JobObjectConfig {
    /// Kill all processes in the job when the job handle is closed.
    pub kill_on_close: bool,
    /// Prevent child processes from escaping the job.
    pub no_breakaway: bool,
    /// Limit active processes within the job.
    pub active_process_limit: Option<u32>,
    /// CPU rate limit (0-100, percentage).
    pub cpu_rate_limit: Option<u32>,
    /// Memory limit in bytes.
    pub memory_limit: Option<u64>,
}

impl Default for JobObjectConfig {
    fn default() -> Self {
        Self {
            kill_on_close: true,
            no_breakaway: true,
            active_process_limit: None,
            cpu_rate_limit: None,
            memory_limit: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_level_rids() {
        assert_eq!(IntegrityLevel::System.rid(), 0x4000);
        assert_eq!(IntegrityLevel::High.rid(), 0x3000);
        assert_eq!(IntegrityLevel::Medium.rid(), 0x2000);
        assert_eq!(IntegrityLevel::Low.rid(), 0x1000);
        assert_eq!(IntegrityLevel::Untrusted.rid(), 0x0000);
    }

    #[test]
    fn integrity_level_sid_strings() {
        assert_eq!(IntegrityLevel::Low.sid_string(), "S-1-16-4096");
        assert_eq!(IntegrityLevel::Untrusted.sid_string(), "S-1-16-0");
        assert_eq!(IntegrityLevel::Medium.sid_string(), "S-1-16-8192");
    }

    #[test]
    fn default_config_has_deny_only_sids() {
        let config = RestrictedTokenConfig::default();
        assert!(!config.deny_only_sids.is_empty());
        assert!(config.deny_only_sids.contains(&"S-1-5-32-544".to_string()));
    }

    #[test]
    fn standard_config() {
        let config = RestrictedTokenConfig::standard();
        assert_eq!(config.integrity_level, IntegrityLevel::Low);
        assert!(config.use_job_object);
        assert!(!config.use_alternate_desktop);
    }

    #[test]
    fn strict_config() {
        let config = RestrictedTokenConfig::strict();
        assert_eq!(config.integrity_level, IntegrityLevel::Untrusted);
        assert!(config.use_alternate_desktop);
    }

    #[test]
    fn create_restricted_token_unsupported_on_non_windows() {
        let config = RestrictedTokenConfig::default();
        let result = create_restricted_token(&config);
        assert!(result.is_err());
    }

    #[test]
    fn job_object_config_defaults() {
        let config = JobObjectConfig::default();
        assert!(config.kill_on_close);
        assert!(config.no_breakaway);
        assert!(config.active_process_limit.is_none());
    }

    #[test]
    fn deny_only_sids_are_well_known() {
        for sid in DENY_ONLY_SIDS {
            assert!(sid.starts_with("S-1-"));
        }
    }
}
