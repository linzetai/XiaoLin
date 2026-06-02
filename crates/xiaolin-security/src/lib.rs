pub mod auth;
pub mod dangerous_ops;
pub mod permission_profile;
pub mod policy_transforms;
pub mod prompt_guard;
pub mod rate_limit;
pub mod read_deny_matcher;
pub mod ssrf;

pub use auth::{ApiKeyAuth, AuthConfig};
pub use permission_profile::{
    forbidden_agent_metadata_write, is_protected_metadata_name, project_roots_glob_pattern,
    ActivePermissionProfile, AdditionalPermissionProfile, FileSystemAccessMode, FileSystemPath,
    FileSystemPermissions, FileSystemSandboxEntry, FileSystemSandboxKind,
    FileSystemSandboxPolicy, FileSystemSpecialPath, ManagedFileSystemPermissions,
    NetworkPermissions, NetworkSandboxPolicy, PermissionProfile, SandboxEnforcement,
    WritableRoot, PROTECTED_METADATA_PATH_NAMES,
};
pub use policy_transforms::{
    canonicalize_preserving_symlinks, effective_file_system_sandbox_policy,
    effective_network_sandbox_policy, effective_permission_profile,
    intersect_fs_sandbox_policies, intersect_network_sandbox, intersect_permission_profiles,
    merge_file_system_policies, merge_fs_entries, merge_permission_profiles,
    normalize_additional_permissions, normalize_fs_entries, should_require_platform_sandbox,
};
pub use prompt_guard::{PromptGuard, PromptGuardResult, RiskLevel};
pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use read_deny_matcher::ReadDenyMatcher;
