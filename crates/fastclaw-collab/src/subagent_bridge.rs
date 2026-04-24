use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;

use fastclaw_agent::SubAgentManager;
use fastclaw_core::agent_config::SubAgentPolicy;
use fastclaw_core::error::{FastClawError, FastClawResult};
use fastclaw_core::tool::ToolRegistry;
use fastclaw_core::types::{StreamEvent, SubAgentType};

use crate::delegation::{DelegationRequest, DelegationResult};

/// Bridges the old `DelegationRequest/DelegationResult` interface to the new
/// `SubAgentManager` system, providing streaming, lifecycle management, and
/// typed sub-agent execution while retaining the simple request/reply API.
pub struct SubAgentDelegation {
    manager: Arc<SubAgentManager>,
    tool_registry: Arc<ToolRegistry>,
    policy: SubAgentPolicy,
}

impl SubAgentDelegation {
    pub fn new(
        manager: Arc<SubAgentManager>,
        tool_registry: Arc<ToolRegistry>,
        policy: SubAgentPolicy,
    ) -> Self {
        Self {
            manager,
            tool_registry,
            policy,
        }
    }

    /// Execute a delegation request through the sub-agent system.
    ///
    /// This provides the same `DelegationRequest -> DelegationResult` contract
    /// as the bus-based `delegate_task`, but with full sub-agent lifecycle
    /// management (streaming, cancellation, typed registries).
    pub async fn delegate(
        &self,
        req: DelegationRequest,
        timeout: Duration,
    ) -> FastClawResult<DelegationResult> {
        self.delegate_with_stream(req, timeout, None).await
    }

    /// Like [`delegate`](Self::delegate), but forwards streaming events to the
    /// given sender for real-time visibility.
    pub async fn delegate_with_stream(
        &self,
        req: DelegationRequest,
        timeout: Duration,
        event_tx: Option<mpsc::Sender<StreamEvent>>,
    ) -> FastClawResult<DelegationResult> {
        let agent_config = self.manager.resolve_agent(&req.to_agent).ok_or_else(|| {
            FastClawError::Agent(format!("agent '{}' not found in SubAgentManager", req.to_agent))
        })?;

        let subagent_type = extract_subagent_type(&req.context);

        let child_registry = Arc::new(
            fastclaw_agent::subagent::build_child_registry(&self.tool_registry, &subagent_type),
        );

        let context_str = if req.context.is_null() {
            None
        } else {
            Some(serde_json::to_string(&req.context).unwrap_or_default())
        };

        let parent_tx = event_tx.unwrap_or_else(|| {
            let (tx, _rx) = mpsc::channel(16);
            tx
        });

        let mut policy_with_timeout = self.policy.clone();
        policy_with_timeout.timeout_seconds = timeout.as_secs();

        let run_id = self
            .manager
            .spawn(
                agent_config,
                subagent_type,
                req.task,
                context_str,
                String::new(),
                req.from_agent.to_string(),
                0,
                &policy_with_timeout,
                child_registry,
                parent_tx,
                None,
            )
            .await
            .map_err(|e| FastClawError::Agent(format!("failed to spawn sub-agent: {e}")))?;

        let poll_interval = Duration::from_millis(200);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            tokio::time::sleep(poll_interval).await;

            if tokio::time::Instant::now() > deadline {
                self.manager.cancel(&run_id);
                return Err(FastClawError::Agent(
                    "sub-agent delegation timed out".into(),
                ));
            }

            if let Some(run) = self.manager.get_run(&run_id) {
                if run.status.is_terminal() {
                    let success = run.status == fastclaw_core::types::SubAgentStatus::Completed;
                    let output = run
                        .result
                        .map(|s| Value::String(s))
                        .unwrap_or(Value::Null);
                    return Ok(DelegationResult { success, output });
                }
            }
        }
    }
}

fn extract_subagent_type(context: &Value) -> SubAgentType {
    context
        .get("subagent_type")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "explore" => SubAgentType::Explore,
            "shell" => SubAgentType::Shell,
            "browser" => SubAgentType::Browser,
            "general" => SubAgentType::General,
            other => SubAgentType::Custom(other.to_string()),
        })
        .unwrap_or(SubAgentType::General)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_type_from_context() {
        let ctx = serde_json::json!({ "subagent_type": "explore", "data": 42 });
        assert_eq!(extract_subagent_type(&ctx), SubAgentType::Explore);

        let empty = serde_json::json!({});
        assert_eq!(extract_subagent_type(&empty), SubAgentType::General);
    }
}
