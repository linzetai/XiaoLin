pub mod agents;
mod bus;
mod channel;
mod chat;
pub mod common;
mod cron;
mod dynamic_routes;
mod error;
mod evolution;
mod health;
mod memory;
mod session;
mod subagent;
mod traces;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

pub(crate) use channel::handle_channel_message;
pub use common::{
    apply_model_router_for_chat, filtered_tool_definitions, map_router_resolve_err,
    record_chat_budget_actual, record_chat_budget_stream_estimate, try_reserve_budget,
};
pub use memory::auto_record_episode;
pub use session::{resolve_session_context, ResolvedSession};

pub fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(health::serve_ui))
        .route("/ui", get(health::serve_ui))
        .route("/health", get(health::health))
        .route("/ready", get(health::readiness))
        .route("/metrics", get(health::metrics_endpoint))
        .route("/api/v1/metrics", get(health::structured_metrics_v1))
        .route("/api/v1/auth/status", get(health::auth_status))
        .route("/ws", get(crate::ws::ws_handler))
        .route("/api/v1/chat", post(chat::chat_completions))
        .route("/api/v1/chat/completions", post(chat::chat_completions))
        .route(
            "/api/v1/agents",
            get(chat::list_agents).post(agents::post_agent),
        )
        .route("/api/v1/skills", get(chat::list_skills))
        .route(
            "/api/v1/agents/:agent_id/tools",
            get(agents::list_agent_tools).put(agents::put_agent_tools),
        )
        .route(
            "/api/v1/agents/:agent_id",
            get(agents::get_agent)
                .put(agents::put_agent)
                .delete(agents::delete_agent),
        )
        .route("/api/v1/tools", get(chat::list_tools))
        .route("/api/v1/sessions", get(session::list_sessions))
        .route("/api/v1/sessions/:session_id", get(session::get_session))
        .route(
            "/api/v1/sessions/:session_id",
            delete(session::delete_session),
        )
        .route(
            "/api/v1/sessions/:session_id/messages",
            get(session::get_session_messages),
        )
        .route("/api/v1/memory/episodes", get(memory::list_episodes))
        .route(
            "/api/v1/memory/episodes/search",
            get(memory::search_episodes),
        )
        .route("/api/v1/memory/facts", get(memory::list_facts))
        .route("/api/v1/memory/facts", post(memory::upsert_fact))
        .route("/api/v1/memory/facts/search", get(memory::search_facts))
        .route("/api/v1/memory/facts/:fact_id", delete(memory::delete_fact))
        .route("/api/v1/bus/agents", get(bus::bus_list_agents))
        .route("/api/v1/bus/send", post(bus::bus_send_message))
        .route("/api/v1/bus/request", post(bus::bus_request_reply))
        .route(
            "/api/v1/evolution/feedback",
            post(evolution::submit_feedback),
        )
        .route(
            "/api/v1/evolution/feedback/:agent_id",
            get(evolution::get_feedback),
        )
        .route(
            "/api/v1/evolution/evaluate/:agent_id",
            get(evolution::evaluate_agent),
        )
        .route(
            "/api/v1/evolution/distill/:agent_id",
            post(evolution::distill_prompt),
        )
        .route(
            "/api/v1/evolution/candidates/:agent_id",
            get(evolution::list_candidates),
        )
        .route(
            "/api/v1/evolution/candidates/:candidate_id/accept",
            post(evolution::accept_candidate),
        )
        .route(
            "/api/v1/evolution/candidates/:candidate_id/reject",
            post(evolution::reject_candidate),
        )
        .route("/api/v1/cron/jobs", get(cron::list_cron_jobs))
        .route("/api/v1/cron/jobs", post(cron::upsert_cron_job))
        .route("/api/v1/cron/jobs/:job_id", get(cron::get_cron_job))
        .route("/api/v1/cron/jobs/:job_id", delete(cron::delete_cron_job))
        .route("/api/v1/channels", get(channel::list_channels))
        .route("/api/v1/routes", get(dynamic_routes::list_routes))
        .route("/api/v1/routes", post(dynamic_routes::add_route))
        .route("/api/v1/routes/:id", delete(dynamic_routes::delete_route))
        .route("/api/v1/routes/:id", put(dynamic_routes::update_route))
        .route("/webhook/:channel_id", post(channel::channel_webhook))
        .route("/api/v1/openapi.json", get(health::openapi_spec))
        .route("/api/v1/traces", get(traces::list_traces))
        .route("/api/v1/traces/:trace_id", get(traces::get_trace))
        .route("/api/v1/traces/:trace_id", delete(traces::delete_trace))
        .route("/api/v1/subagents/runs", get(subagent::list_subagent_runs))
        .route(
            "/api/v1/subagents/runs/:run_id",
            get(subagent::get_subagent_run).delete(subagent::cancel_subagent_run),
        )
}
