pub mod agents;
mod bus;
mod channel;
mod chat;
pub mod common;
mod cost;
mod cron;
mod diagnostics;
mod dynamic_routes;
mod error;
mod evolution;
mod health;
mod llm_plugin;
mod memory;
mod pty;
mod session;
mod stt;
pub(crate) mod subagent;
mod traces;
mod wechat;

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

pub fn chat_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/chat", post(chat::chat_completions))
        .route("/api/v1/chat/completions", post(chat::chat_completions))
}

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
        .route(
            "/api/v1/chat/resolve-approval",
            post(chat::resolve_approval),
        )
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
        .route("/v1/audio/transcriptions", post(stt::audio_transcriptions))
        .route(
            "/api/v1/audio/transcriptions",
            post(stt::audio_transcriptions),
        )
        .route("/api/v1/traces", get(traces::list_traces))
        .route("/api/v1/traces/:trace_id", get(traces::get_trace))
        .route("/api/v1/traces/:trace_id", delete(traces::delete_trace))
        .route("/api/v1/subagents/defs", get(subagent::list_subagent_defs))
        .route("/api/v1/subagents/runs", get(subagent::list_subagent_runs))
        .route(
            "/api/v1/subagents/runs/:run_id",
            get(subagent::get_subagent_run).delete(subagent::cancel_subagent_run),
        )
        .route(
            "/api/v1/subagents/concurrency",
            get(subagent::get_concurrency_snapshot),
        )
        .route(
            "/api/v1/llm-plugins",
            get(llm_plugin::list_plugins).post(llm_plugin::create_plugin),
        )
        .route(
            "/api/v1/llm-plugins/:id",
            get(llm_plugin::get_plugin)
                .put(llm_plugin::update_plugin)
                .delete(llm_plugin::delete_plugin),
        )
        .route(
            "/api/v1/llm-plugins/:id/test",
            post(llm_plugin::test_plugin),
        )
        // WeChat channel login
        .route(
            "/api/v1/channels/wechat/login/start",
            post(wechat::login_start),
        )
        .route(
            "/api/v1/channels/wechat/login/status/:session_key",
            get(wechat::login_status),
        )
        .route(
            "/api/v1/channels/wechat/login/verify/:session_key",
            post(wechat::login_verify),
        )
        .route(
            "/api/v1/channels/wechat/accounts",
            get(wechat::list_accounts),
        )
        .route(
            "/api/v1/channels/wechat/accounts/:account_id",
            delete(wechat::delete_account),
        )
        .route(
            "/api/v1/channels/wechat/reload",
            post(wechat::reload_channel),
        )
        // PTY interactive terminal
        .route("/api/v1/pty", get(pty::pty_ws_handler))
        .route("/api/v1/pty/sessions", get(pty::pty_list_handler))
        // Cost tracking
        .route("/api/v1/cost/summary", get(cost::get_cost_summary))
        .route("/api/v1/cost/daily", get(cost::get_daily_tokens))
        .route("/api/v1/cost/tools", get(cost::get_tool_stats))
        .route("/api/v1/cost/sessions", get(cost::get_session_costs))
        // Runtime diagnostics
        .route(
            "/api/v1/diagnostics/runtime-quality/turns",
            get(diagnostics::list_runtime_quality_turns),
        )
        .route(
            "/api/v1/diagnostics/runtime-quality/turns/:session_id/:turn_id",
            get(diagnostics::get_runtime_quality_turn),
        )
        .route(
            "/api/v1/diagnostics/runtime-quality/export",
            get(diagnostics::export_runtime_quality_turns),
        )
}
