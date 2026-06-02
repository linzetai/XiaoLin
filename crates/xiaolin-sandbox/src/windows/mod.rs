pub mod acl;
pub mod token;
pub mod wfp;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use xiaolin_security::{FileSystemSandboxPolicy, NetworkSandboxPolicy};

use crate::{SandboxType, SandboxedCommand};

/// Windows-specific sandbox isolation level.
///
/// Controls the depth of Windows security token restriction applied
/// to child processes. Aligned with Codex `config_types.rs:254-259`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowsSandboxLevel {
    /// No sandbox restriction (tokens unchanged).
    Off,
    /// Restricted token with deny-only SIDs and limited privileges.
    #[default]
    Standard,
    /// Low integrity level + restricted token + job object containment.
    Strict,
}

/// Transform a shell command for Windows RestrictedToken sandbox using the
/// default `WindowsSandboxLevel::Standard`.
pub fn transform(
    command: &str,
    shell: &str,
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
) -> SandboxedCommand {
    transform_with_level(command, shell, fs_policy, net_policy, WindowsSandboxLevel::default())
}

/// Transform a shell command for Windows RestrictedToken sandbox with an
/// explicit sandbox level.
///
/// Environment markers signal the chosen level to the execution runtime:
/// - `XIAOLIN_SANDBOXED`: "1" (always set)
/// - `XIAOLIN_SANDBOX_LEVEL`: "off" | "standard" | "strict"
/// - `XIAOLIN_SANDBOX_FS_POLICY`: JSON-serialized filesystem sandbox policy
/// - `XIAOLIN_SANDBOX_NET_POLICY`: JSON-serialized network sandbox policy
pub fn transform_with_level(
    command: &str,
    shell: &str,
    fs_policy: &FileSystemSandboxPolicy,
    net_policy: NetworkSandboxPolicy,
    level: WindowsSandboxLevel,
) -> SandboxedCommand {
    let mut env = HashMap::new();
    env.insert("XIAOLIN_SANDBOXED".to_string(), "1".to_string());

    let level_str = match level {
        WindowsSandboxLevel::Off => "off",
        WindowsSandboxLevel::Standard => "standard",
        WindowsSandboxLevel::Strict => "strict",
    };
    env.insert("XIAOLIN_SANDBOX_LEVEL".to_string(), level_str.to_string());

    if let Ok(fs_json) = serde_json::to_string(fs_policy) {
        env.insert("XIAOLIN_SANDBOX_FS_POLICY".to_string(), fs_json);
    }
    if let Ok(net_json) = serde_json::to_string(&net_policy) {
        env.insert("XIAOLIN_SANDBOX_NET_POLICY".to_string(), net_json);
    }

    let (program, args) = if shell == "powershell" || shell == "pwsh" {
        (
            shell.to_string(),
            vec!["-Command".to_string(), command.to_string()],
        )
    } else {
        (
            "cmd.exe".to_string(),
            vec!["/C".to_string(), command.to_string()],
        )
    };

    SandboxedCommand {
        program,
        args,
        working_dir: None,
        env,
        env_remove: Vec::new(),
        sandbox_type: SandboxType::RestrictedToken,
        linux_sandbox: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unrestricted_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::unrestricted()
    }

    fn locked_down_fs() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::restricted(vec![])
    }

    #[test]
    fn windows_transform_uses_cmd() {
        let cmd = transform("echo hello", "cmd", &unrestricted_fs(), NetworkSandboxPolicy::Enabled);
        assert_eq!(cmd.program, "cmd.exe");
        assert_eq!(cmd.sandbox_type, SandboxType::RestrictedToken);
        assert!(cmd.args.contains(&"/C".to_string()));
    }

    #[test]
    fn windows_transform_powershell() {
        let cmd = transform(
            "echo hello",
            "powershell",
            &unrestricted_fs(),
            NetworkSandboxPolicy::Enabled,
        );
        assert_eq!(cmd.program, "powershell");
        assert!(cmd.args.contains(&"-Command".to_string()));
    }

    #[test]
    fn windows_transform_sets_env_markers() {
        let cmd = transform("dir", "cmd", &locked_down_fs(), NetworkSandboxPolicy::Restricted);
        assert_eq!(cmd.env.get("XIAOLIN_SANDBOXED").unwrap(), "1");
        assert!(cmd.env.contains_key("XIAOLIN_SANDBOX_FS_POLICY"));
        assert!(cmd.env.contains_key("XIAOLIN_SANDBOX_NET_POLICY"));
    }

    #[test]
    fn windows_sandbox_level_default_is_standard() {
        assert_eq!(WindowsSandboxLevel::default(), WindowsSandboxLevel::Standard);
    }

    #[test]
    fn windows_sandbox_level_json_roundtrip() {
        for level in [
            WindowsSandboxLevel::Off,
            WindowsSandboxLevel::Standard,
            WindowsSandboxLevel::Strict,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let deserialized: WindowsSandboxLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, deserialized);
        }
    }
}
