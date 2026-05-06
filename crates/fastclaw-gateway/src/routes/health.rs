use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::state::AppState;

pub(super) async fn serve_ui() -> impl IntoResponse {
    Json(json!({
        "name": "FastClaw",
        "description": "AI Agent Orchestration Engine",
        "docs": "/health"
    }))
}

pub(super) async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

pub(super) async fn readiness(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let agent_count = state.rt.router.read().await.list_agents().len();

    let db_ok = {
        let pool = state.store.session_store.pool();
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&pool)
            .await
            .map(|v| v == 1)
            .unwrap_or(false)
    };

    let all_ok = agent_count > 0 && db_ok;
    let status = if all_ok { "ready" } else { "not_ready" };
    let code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": status,
            "agents": agent_count,
            "checks": {
                "database": db_ok,
                "agents_configured": agent_count > 0,
            }
        })),
    )
}

pub(super) async fn auth_status(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let auth_required = !state.cfg.config.security.api_keys.is_empty();
    Json(json!({ "authRequired": auth_required }))
}

pub(super) async fn metrics_endpoint() -> impl IntoResponse {
    let body = fastclaw_observe::render_metrics();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.5; charset=utf-8",
        )],
        body,
    )
}

/// In-memory structured metrics (Prometheus text); see [`fastclaw_observe::MetricsCollector`].
pub(super) async fn structured_metrics_v1() -> impl IntoResponse {
    let body = fastclaw_observe::render_structured_metrics_prometheus();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.5; charset=utf-8",
        )],
        body,
    )
}

pub(super) async fn openapi_spec() -> impl IntoResponse {
    let spec = fastclaw_openapi_spec();
    Json(spec)
}

fn ep(summary: &str, op_id: &str, tag: &str) -> serde_json::Value {
    json!({ "summary": summary, "operationId": op_id, "tags": [tag], "responses": { "200": { "description": "Success" } } })
}

fn ep_body(summary: &str, op_id: &str, tag: &str, req_schema: &str) -> serde_json::Value {
    json!({
        "summary": summary, "operationId": op_id, "tags": [tag],
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{req_schema}") } } } },
        "responses": { "200": { "description": "Success" } }
    })
}

fn ep_param(summary: &str, op_id: &str, tag: &str, param: &str) -> serde_json::Value {
    json!({
        "summary": summary, "operationId": op_id, "tags": [tag],
        "parameters": [{ "name": param, "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Success" } }
    })
}

fn fastclaw_openapi_spec() -> serde_json::Value {
    use serde_json::Map;

    let info = json!({
        "title": "FastClaw API",
        "description": "AI Agent Orchestration / Harness Engine",
        "version": env!("CARGO_PKG_VERSION")
    });

    let mut paths = Map::new();

    let routes: &[(&str, serde_json::Value)] = &[
        (
            "/health",
            json!({ "get": ep("Health check", "health", "Health") }),
        ),
        (
            "/ready",
            json!({ "get": ep("Readiness probe", "readiness", "Health") }),
        ),
        (
            "/metrics",
            json!({ "get": ep("Prometheus metrics", "metrics", "Observability") }),
        ),
        (
            "/api/v1/metrics",
            json!({ "get": ep("Structured metrics", "structuredMetrics", "Observability") }),
        ),
        (
            "/api/v1/auth/status",
            json!({ "get": ep("Auth status", "authStatus", "Auth") }),
        ),
        (
            "/api/v1/chat",
            json!({ "post": ep_body("Chat completion", "chatCompletions", "Chat", "ChatRequest") }),
        ),
        (
            "/api/v1/chat/completions",
            json!({ "post": ep_body("Chat completion (alias)", "chatCompletionsAlias", "Chat", "ChatRequest") }),
        ),
        (
            "/api/v1/agents",
            json!({ "get": ep("List agents", "listAgents", "Agents"), "post": ep_body("Create agent", "postAgent", "Agents", "AgentConfig") }),
        ),
        (
            "/api/v1/agents/{agent_id}",
            json!({ "get": ep_param("Get agent", "getAgent", "Agents", "agent_id"), "put": ep_param("Update agent", "putAgent", "Agents", "agent_id"), "delete": ep_param("Delete agent", "deleteAgent", "Agents", "agent_id") }),
        ),
        (
            "/api/v1/agents/{agent_id}/tools",
            json!({ "get": ep_param("List agent tools", "listAgentTools", "Agents", "agent_id"), "put": ep_param("Set agent tools", "putAgentTools", "Agents", "agent_id") }),
        ),
        (
            "/api/v1/skills",
            json!({ "get": ep("List skills", "listSkills", "Agents") }),
        ),
        (
            "/api/v1/tools",
            json!({ "get": ep("List tools", "listTools", "Tools") }),
        ),
        (
            "/api/v1/sessions",
            json!({ "get": ep("List sessions", "listSessions", "Sessions") }),
        ),
        (
            "/api/v1/sessions/{session_id}",
            json!({ "get": ep_param("Get session", "getSession", "Sessions", "session_id"), "delete": ep_param("Delete session", "deleteSession", "Sessions", "session_id") }),
        ),
        (
            "/api/v1/sessions/{session_id}/messages",
            json!({ "get": ep_param("Get messages", "getSessionMessages", "Sessions", "session_id") }),
        ),
        (
            "/api/v1/memory/episodes",
            json!({ "get": ep("List episodes", "listEpisodes", "Memory") }),
        ),
        (
            "/api/v1/memory/episodes/search",
            json!({ "get": ep("Search episodes", "searchEpisodes", "Memory") }),
        ),
        (
            "/api/v1/memory/facts",
            json!({ "get": ep("List facts", "listFacts", "Memory"), "post": ep_body("Upsert fact", "upsertFact", "Memory", "Fact") }),
        ),
        (
            "/api/v1/memory/facts/search",
            json!({ "get": ep("Search facts", "searchFacts", "Memory") }),
        ),
        (
            "/api/v1/memory/facts/{fact_id}",
            json!({ "delete": ep_param("Delete fact", "deleteFact", "Memory", "fact_id") }),
        ),
        (
            "/api/v1/bus/agents",
            json!({ "get": ep("List bus agents", "busListAgents", "AgentBus") }),
        ),
        (
            "/api/v1/bus/send",
            json!({ "post": ep("Send message", "busSend", "AgentBus") }),
        ),
        (
            "/api/v1/bus/request",
            json!({ "post": ep("Request-reply", "busRequest", "AgentBus") }),
        ),
        (
            "/api/v1/evolution/feedback",
            json!({ "post": ep("Submit feedback", "submitFeedback", "Evolution") }),
        ),
        (
            "/api/v1/evolution/feedback/{agent_id}",
            json!({ "get": ep_param("Get feedback", "getFeedback", "Evolution", "agent_id") }),
        ),
        (
            "/api/v1/evolution/evaluate/{agent_id}",
            json!({ "get": ep_param("Evaluate agent", "evaluateAgent", "Evolution", "agent_id") }),
        ),
        (
            "/api/v1/evolution/distill/{agent_id}",
            json!({ "post": ep_param("Distill prompt", "distillPrompt", "Evolution", "agent_id") }),
        ),
        (
            "/api/v1/evolution/candidates/{agent_id}",
            json!({ "get": ep_param("List candidates", "listCandidates", "Evolution", "agent_id") }),
        ),
        (
            "/api/v1/evolution/candidates/{candidate_id}/accept",
            json!({ "post": ep_param("Accept candidate", "acceptCandidate", "Evolution", "candidate_id") }),
        ),
        (
            "/api/v1/evolution/candidates/{candidate_id}/reject",
            json!({ "post": ep_param("Reject candidate", "rejectCandidate", "Evolution", "candidate_id") }),
        ),
        (
            "/api/v1/cron/jobs",
            json!({ "get": ep("List cron jobs", "listCronJobs", "Cron"), "post": ep_body("Upsert cron job", "upsertCronJob", "Cron", "CronJob") }),
        ),
        (
            "/api/v1/cron/jobs/{job_id}",
            json!({ "get": ep_param("Get cron job", "getCronJob", "Cron", "job_id"), "delete": ep_param("Delete cron job", "deleteCronJob", "Cron", "job_id") }),
        ),
        (
            "/api/v1/plugins",
            json!({ "get": ep("List plugins", "listPlugins", "Plugins") }),
        ),
        (
            "/api/v1/plugins/{plugin_id}/invoke/{capability}",
            json!({ "post": ep("Invoke plugin", "invokePlugin", "Plugins") }),
        ),
        (
            "/api/v1/channels",
            json!({ "get": ep("List channels", "listChannels", "Channels") }),
        ),
        (
            "/webhook/{channel_id}",
            json!({ "post": ep_param("Channel webhook", "channelWebhook", "Channels", "channel_id") }),
        ),
        (
            "/api/v1/routes",
            json!({ "get": ep("List routes", "listRoutes", "DynamicRoutes"), "post": ep("Add route", "addRoute", "DynamicRoutes") }),
        ),
        (
            "/api/v1/routes/{id}",
            json!({ "put": ep_param("Update route", "updateRoute", "DynamicRoutes", "id"), "delete": ep_param("Delete route", "deleteRoute", "DynamicRoutes", "id") }),
        ),
        (
            "/api/v1/traces",
            json!({ "get": ep("List traces", "listTraces", "Traces") }),
        ),
        (
            "/api/v1/traces/{trace_id}",
            json!({ "get": ep_param("Get trace", "getTrace", "Traces", "trace_id"), "delete": ep_param("Delete trace", "deleteTrace", "Traces", "trace_id") }),
        ),
        (
            "/api/v1/openapi.json",
            json!({ "get": ep("OpenAPI spec", "openapiSpec", "Meta") }),
        ),
    ];

    for (path, ops) in routes {
        paths.insert((*path).to_string(), ops.clone());
    }

    json!({
        "openapi": "3.1.0",
        "info": info,
        "paths": paths
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid_structure() {
        let spec = fastclaw_openapi_spec();
        assert_eq!(spec["openapi"], "3.1.0");
        assert_eq!(spec["info"]["title"], "FastClaw API");
        assert!(spec["paths"].as_object().unwrap().len() > 30);
    }

    #[test]
    fn openapi_contains_all_route_groups() {
        let spec = fastclaw_openapi_spec();
        let paths = spec["paths"].as_object().unwrap();
        let empty = vec![];
        let mut all_tags = std::collections::HashSet::new();
        for methods in paths.values() {
            for op in methods.as_object().unwrap().values() {
                for tag in op["tags"].as_array().unwrap_or(&empty) {
                    if let Some(s) = tag.as_str() {
                        all_tags.insert(s.to_string());
                    }
                }
            }
        }

        for tag in &[
            "Health", "Chat", "Agents", "Sessions", "Memory", "Traces", "Cron",
        ] {
            assert!(all_tags.contains(*tag), "missing tag: {tag}");
        }
    }

    #[test]
    fn openapi_version_matches_crate() {
        let spec = fastclaw_openapi_spec();
        assert_eq!(spec["info"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn ep_helpers_produce_valid_operations() {
        let simple = ep("Test op", "testOp", "TestTag");
        assert_eq!(simple["summary"], "Test op");
        assert_eq!(simple["operationId"], "testOp");
        assert!(simple["responses"]["200"].is_object());

        let with_body = ep_body("Post op", "postOp", "TestTag", "MySchema");
        assert!(with_body["requestBody"]["required"].as_bool().unwrap());
        let schema_ref = with_body["requestBody"]["content"]["application/json"]["schema"]["$ref"]
            .as_str()
            .unwrap();
        assert!(schema_ref.contains("MySchema"));

        let with_param = ep_param("Param op", "paramOp", "TestTag", "my_id");
        let params = with_param["parameters"].as_array().unwrap();
        assert_eq!(params[0]["name"], "my_id");
        assert_eq!(params[0]["in"], "path");
    }
}
