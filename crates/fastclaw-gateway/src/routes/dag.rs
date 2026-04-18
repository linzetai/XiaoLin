use async_trait::async_trait;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;

pub(super) async fn dag_list_workflows() -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(Json(json!({
        "count": 0,
        "workflows": [],
        "notes": "FastClaw currently executes ad-hoc DAG definitions via POST /api/v1/dag/execute (alias: /api/v1/dag/run).",
    })))
}

pub(super) async fn dag_validate(
    AppJson(body): AppJson<serde_json::Value>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let json_str = serde_json::to_string(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid JSON: {e}")))?;
    let def = fastclaw_dag::DagDefinition::from_json(&json_str)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let graph =
        fastclaw_dag::DagGraph::build(&def).map_err(|e| AppError::BadRequest(e.to_string()))?;
    let levels = graph
        .execution_levels()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    Ok(Json(json!({
        "valid": true,
        "nodes": graph.node_count(),
        "edges": graph.edge_count(),
        "levels": levels.len(),
        "level_detail": levels.iter().enumerate().map(|(i, l)| json!({
            "level": i,
            "nodes": l,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub(super) struct DagExecuteBody {
    pub dag: serde_json::Value,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
}

pub(super) async fn dag_execute(
    axum::extract::State(state): axum::extract::State<AppState>,
    AppJson(body): AppJson<DagExecuteBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let dag_json = serde_json::to_string(&body.dag)
        .map_err(|e| AppError::BadRequest(format!("invalid dag JSON: {e}")))?;
    let def = fastclaw_dag::DagDefinition::from_json(&dag_json)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let graph =
        fastclaw_dag::DagGraph::build(&def).map_err(|e| AppError::BadRequest(e.to_string()))?;

    let handler = std::sync::Arc::new(DagNodeHandler {
        tool_registry: state.tool_registry.clone(),
        runtime: state.runtime.clone(),
        router: state.router.clone(),
    });
    let dag_run_id = uuid::Uuid::new_v4().to_string();
    let executor = fastclaw_dag::DagExecutor::with_checkpoint_store(
        graph,
        handler,
        state.dag_checkpoint_store.clone(),
        dag_run_id,
    );

    let ctx = if let Some(input) = body.input {
        fastclaw_dag::ExecutionContext::with_input(input)
    } else {
        fastclaw_dag::ExecutionContext::new()
    };

    let result_ctx = executor.run(ctx).await?;
    let snapshot = result_ctx.snapshot().await;

    Ok(Json(json!({
        "completed": true,
        "outputs": snapshot,
    })))
}

pub(crate) struct DagNodeHandler {
    pub(crate) tool_registry: std::sync::Arc<fastclaw_core::tool::ToolRegistry>,
    pub(crate) runtime: std::sync::Arc<fastclaw_agent::AgentRuntime>,
    pub(crate) router: crate::state::SharedRouter,
}

pub(crate) type CronDagHandler = DagNodeHandler;

#[async_trait]
impl fastclaw_dag::NodeHandler for DagNodeHandler {
    async fn execute_node(
        &self,
        node: &fastclaw_dag::NodeDef,
        ctx: &fastclaw_dag::ExecutionContext,
    ) -> anyhow::Result<serde_json::Value> {
        match node.kind {
            fastclaw_dag::NodeKind::ToolCall => {
                let tool_name = node
                    .config
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("tool_call node '{}' missing 'tool_name' config", node.id)
                    })?;
                let args = node
                    .config
                    .get("arguments")
                    .map(|v| serde_json::to_string(v).unwrap_or_default())
                    .unwrap_or_else(|| "{}".to_string());
                let tool = self
                    .tool_registry
                    .get(tool_name)
                    .ok_or_else(|| anyhow::anyhow!("tool not found: {tool_name}"))?;
                let result = tool.execute(&args).await;
                Ok(json!({
                    "tool": tool_name,
                    "success": result.success,
                    "output": result.output,
                }))
            }
            fastclaw_dag::NodeKind::Condition => {
                let condition = node
                    .config
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .unwrap_or("true");
                let snapshot = ctx.snapshot().await;
                let context_value = serde_json::to_value(&snapshot)?;
                let branch = fastclaw_dag::evaluate_condition(condition, &context_value)?;
                Ok(serde_json::Value::String(branch))
            }
            fastclaw_dag::NodeKind::LlmCall => {
                let agent_id = node
                    .config
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main");
                let prompt = node
                    .config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Process the input.");

                let input_val = ctx.get("input").await.unwrap_or(serde_json::Value::Null);
                let user_msg = if input_val.is_null() {
                    prompt.to_string()
                } else {
                    format!("{prompt}\n\nInput: {input_val}")
                };

                let request = fastclaw_core::types::ChatRequest {
                    messages: vec![fastclaw_core::types::ChatMessage {
                        role: fastclaw_core::types::Role::User,
                        content: Some(serde_json::Value::String(user_msg)),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    }],
                    stream: false,
                    model: node
                        .config
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    temperature: node
                        .config
                        .get("temperature")
                        .and_then(|v| v.as_f64())
                        .map(|f| f as f32),
                    max_tokens: node
                        .config
                        .get("max_tokens")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32),
                    agent_id: Some(agent_id.to_string()),
                    session_id: None,
                    tools: None,
                    slash_intent: None,
                    work_dir: None,
                };

                let agent_config = self
                    .router
                    .read()
                    .await
                    .resolve(&request)
                    .map(|c| c.clone())
                    .map_err(|e| anyhow::anyhow!("agent resolve failed: {e}"))?;

                let exec_result = self
                    .runtime
                    .execute(&agent_config, &request, &self.tool_registry, None)
                    .await?;
                let content = exec_result
                    .response
                    .choices
                    .first()
                    .and_then(|c| c.message.text_content())
                    .unwrap_or_default();

                Ok(json!({
                    "node_id": node.id,
                    "agent_id": agent_id,
                    "content": content,
                    "tool_calls_made": exec_result.tool_calls_made,
                    "iterations": exec_result.iterations,
                }))
            }
            _ => Ok(json!({
                "node_id": node.id,
                "kind": format!("{:?}", node.kind),
                "status": "executed",
            })),
        }
    }
}
