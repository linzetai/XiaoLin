use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};

/// Wraps a tool with a different OpenAI function name (for per-agent scoped tools).
pub(crate) struct RenamedTool {
    name: String,
    description: String,
    inner: Arc<dyn Tool + Send + Sync>,
}

impl RenamedTool {
    pub(crate) fn new(
        name: String,
        description: String,
        inner: Arc<dyn Tool + Send + Sync>,
    ) -> Self {
        Self {
            name,
            description,
            inner,
        }
    }
}

#[async_trait]
impl Tool for RenamedTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        self.inner.parameters_schema()
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        self.inner.execute(arguments).await
    }
}
