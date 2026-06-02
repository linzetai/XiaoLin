use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub outcome: SandboxOutcome,
    pub output: String,
    pub latency_ms: u64,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxOutcome {
    Success,
    Failed,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Runs a prompt variant in an isolated sandbox for evaluation.
#[async_trait::async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Execute a prompt with given messages in a sandboxed context.
    /// The sandbox should be isolated from production state.
    async fn run_sandboxed(
        &self,
        agent_id: &str,
        system_prompt: &str,
        test_messages: &[serde_json::Value],
    ) -> anyhow::Result<SandboxResult>;
}

#[async_trait::async_trait]
pub trait SandboxBackend: Send + Sync {
    async fn execute_prompt(
        &self,
        agent_id: &str,
        system_prompt: &str,
        test_messages: &[serde_json::Value],
    ) -> anyhow::Result<SandboxResult>;
}

pub struct DirectSandboxRunner<B: SandboxBackend> {
    runtime: Arc<B>,
}

impl<B: SandboxBackend> DirectSandboxRunner<B> {
    pub fn new(runtime: Arc<B>) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &B {
        &self.runtime
    }
}

#[async_trait::async_trait]
impl<B: SandboxBackend> SandboxRunner for DirectSandboxRunner<B> {
    async fn run_sandboxed(
        &self,
        agent_id: &str,
        system_prompt: &str,
        test_messages: &[serde_json::Value],
    ) -> anyhow::Result<SandboxResult> {
        let started = Instant::now();
        let mut result = SandboxBackend::execute_prompt(
            self.runtime.as_ref(),
            agent_id,
            system_prompt,
            test_messages,
        )
        .await?;
        result.latency_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_result_serialization() {
        let result = SandboxResult {
            outcome: SandboxOutcome::Success,
            output: "test output".into(),
            latency_ms: 150,
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            }),
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: SandboxResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.outcome, SandboxOutcome::Success);
        assert_eq!(deserialized.latency_ms, 150);
    }
}
