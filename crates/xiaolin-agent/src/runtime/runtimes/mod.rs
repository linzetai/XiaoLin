pub mod file;
pub mod shell;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool_runtime::{
    Approvable, ExecApprovalRequirement, SandboxAttempt,
    SandboxPreference, Sandboxable, ToolExecContext, ToolRuntime, ToolRuntimeError,
    ToolRunOutput,
};
use xiaolin_protocol::approval::PendingAction;

use self::file::{FileEditRuntime, FileWriteRuntime};
use self::shell::ShellRuntime;

/// Type-erased wrapper enabling heterogeneous runtime storage.
#[async_trait]
pub trait ErasedToolRuntime: Send + Sync {
    fn name(&self) -> &str;
    fn approval_keys(&self, args: &serde_json::Value) -> Vec<String>;
    fn exec_requirement(&self, args: &serde_json::Value, cwd: &Path) -> ExecApprovalRequirement;
    fn to_pending_action(&self, args: &serde_json::Value, cwd: &Path) -> PendingAction;
    fn sandbox_preference(&self) -> SandboxPreference;
    fn escalate_on_sandbox_failure(&self) -> bool;
    fn bypass_approval_on_escalation(&self) -> bool;
    async fn run(
        &self,
        args: &serde_json::Value,
        sandbox: &SandboxAttempt,
        ctx: &ToolExecContext,
    ) -> Result<ToolRunOutput, ToolRuntimeError>;
}

#[async_trait]
impl<T: ToolRuntime + 'static> ErasedToolRuntime for T {
    fn name(&self) -> &str {
        ToolRuntime::name(self)
    }
    fn approval_keys(&self, args: &serde_json::Value) -> Vec<String> {
        Approvable::approval_keys(self, args)
    }
    fn exec_requirement(&self, args: &serde_json::Value, cwd: &Path) -> ExecApprovalRequirement {
        Approvable::exec_requirement(self, args, cwd)
    }
    fn to_pending_action(&self, args: &serde_json::Value, cwd: &Path) -> PendingAction {
        Approvable::to_pending_action(self, args, cwd)
    }
    fn sandbox_preference(&self) -> SandboxPreference {
        Sandboxable::sandbox_preference(self)
    }
    fn escalate_on_sandbox_failure(&self) -> bool {
        Sandboxable::escalate_on_sandbox_failure(self)
    }
    fn bypass_approval_on_escalation(&self) -> bool {
        Sandboxable::bypass_approval_on_escalation(self)
    }
    async fn run(
        &self,
        args: &serde_json::Value,
        sandbox: &SandboxAttempt,
        ctx: &ToolExecContext,
    ) -> Result<ToolRunOutput, ToolRuntimeError> {
        ToolRuntime::run(self, args, sandbox, ctx).await
    }
}

/// Registry mapping tool names to their `ToolRuntime` implementations.
///
/// Tools registered here go through the orchestrator's 5-phase pipeline.
/// Tools NOT in this registry are treated as "safe" and executed directly.
pub struct RuntimeRegistry {
    runtimes: HashMap<String, Arc<dyn ErasedToolRuntime>>,
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            runtimes: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: impl Into<String>, runtime: Arc<dyn ErasedToolRuntime>) {
        self.runtimes.insert(name.into(), runtime);
    }

    pub fn get(&self, tool_name: &str) -> Option<&Arc<dyn ErasedToolRuntime>> {
        self.runtimes.get(tool_name)
    }

    pub fn has(&self, tool_name: &str) -> bool {
        self.runtimes.contains_key(tool_name)
    }

    pub fn registered_names(&self) -> Vec<&str> {
        self.runtimes.keys().map(|s| s.as_str()).collect()
    }
}

/// Build and return the default `RuntimeRegistry` with all guarded runtimes.
pub fn register_default_runtimes() -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();

    let shell: Arc<dyn ErasedToolRuntime> = Arc::new(ShellRuntime);
    registry.register("shell_exec", shell.clone());
    registry.register("sandboxed_shell_exec", shell);

    let file_write: Arc<dyn ErasedToolRuntime> = Arc::new(FileWriteRuntime);
    registry.register("write_file", file_write.clone());
    registry.register("create_file", file_write);

    let file_edit: Arc<dyn ErasedToolRuntime> = Arc::new(FileEditRuntime);
    registry.register("edit_file", file_edit.clone());
    registry.register("multi_edit", file_edit);

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_registers_and_retrieves() {
        let registry = register_default_runtimes();
        assert!(registry.has("shell_exec"));
        assert!(registry.has("write_file"));
        assert!(registry.has("edit_file"));
        assert!(!registry.has("read_file"));
    }

    #[test]
    fn registered_names_includes_all() {
        let registry = register_default_runtimes();
        let names = registry.registered_names();
        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"sandboxed_shell_exec"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"create_file"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"multi_edit"));
    }
}
