use std::collections::HashMap;

use crate::{SandboxType, SandboxedCommand};

/// Passthrough transform: no sandbox applied. Used when the platform doesn't
/// support any sandbox backend or when sandboxing is disabled.
pub fn transform(command: &str, shell: &str) -> SandboxedCommand {
    SandboxedCommand {
        program: shell.to_string(),
        args: vec!["-c".to_string(), command.to_string()],
        working_dir: None,
        env: HashMap::new(),
        env_remove: Vec::new(),
        sandbox_type: SandboxType::Noop,
        linux_sandbox: None,
    }
}
