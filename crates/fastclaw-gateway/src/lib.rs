pub mod consolidation;
pub mod error;
pub mod extract;
pub mod chat_pipeline;
pub mod cron_tool;
pub mod mcp_tool;
mod memory_scope;
pub mod notification_store;
pub mod routes;
mod scoped_tool;
mod state;
mod ws;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{middleware, Extension, Router};
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use fastclaw_core::config::FastClawConfig;
use fastclaw_security::{ApiKeyAuth, RateLimitConfig, RateLimiter};

pub use state::AppState;

fn ensure_auth_for_exposed_bind(config: &FastClawConfig) -> anyhow::Result<()> {
    if config.security.api_keys.is_empty() {
        let bind_addr = config.gateway.bind_addr();
        if !bind_addr.ip().is_loopback() {
            anyhow::bail!(
                "refusing to start gateway on non-loopback address {} without security.api_keys configured",
                bind_addr
            );
        }
    }
    Ok(())
}

fn build_cors(config: &FastClawConfig) -> CorsLayer {
    let origins = &config.gateway.cors_origins;
    if origins.iter().any(|o| o == "*") {
        tracing::warn!(
            "CORS is set to permissive (allow all origins). \
             This is insecure for production — configure explicit origins in gateway.corsOrigins."
        );
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderName::from_static("x-api-key"),
            ])
    } else if origins.is_empty() {
        CorsLayer::new()
    } else {
        let origins: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderName::from_static("x-api-key"),
            ])
    }
}

/// Build the full axum application (Router) with all layers.
/// Exposed publicly so integration tests can reuse the same stack.
pub fn build_app(state: AppState, auth: ApiKeyAuth) -> Router {
    let rl_cfg = RateLimitConfig {
        enabled: state.config.gateway.rate_limit.enabled,
        max_requests: state.config.gateway.rate_limit.max_requests,
        window_secs: state.config.gateway.rate_limit.window_secs,
        trusted_proxies: state.config.gateway.rate_limit.trusted_proxies.clone(),
    };
    let rate_limiter = RateLimiter::new(&rl_cfg);
    let cors = build_cors(&state.config);

    Router::new()
        .merge(routes::api_routes())
        .with_state(state)
        .layer(middleware::from_fn(
            fastclaw_security::rate_limit::rate_limit_middleware,
        ))
        .layer(middleware::from_fn(
            fastclaw_security::auth::auth_middleware,
        ))
        .layer(Extension(auth))
        .layer(Extension(rate_limiter))
        .layer(CompressionLayer::new().gzip(true))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
}

pub async fn run(config: FastClawConfig) -> anyhow::Result<()> {
    ensure_auth_for_exposed_bind(&config)?;
    let state = AppState::new(config).await?;
    let bind_addr = state.config.gateway.bind_addr();

    let auth_config = fastclaw_security::AuthConfig {
        enabled: !state.config.security.api_keys.is_empty(),
        api_keys: state.config.security.api_keys.clone(),
    };
    let auth = ApiKeyAuth::new(&auth_config);
    if state.config.gateway.rate_limit.enabled {
        tracing::info!(
            max_requests = state.config.gateway.rate_limit.max_requests,
            window_secs = state.config.gateway.rate_limit.window_secs,
            "rate limiting enabled"
        );
    }

    spawn_config_watcher(state.clone());
    spawn_cron_scheduler(state.clone());

    let app = build_app(state, auth);

    tracing::info!(%bind_addr, "fastclaw gateway starting");

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    eprintln!("  ✓  Gateway ready on http://{local_addr}/");
    eprintln!();

    let shutdown = shutdown_signal();
    let serve = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown.await;
        tracing::info!(
            "graceful shutdown: not accepting new connections; draining in-flight (max 30s)"
        );
    });

    match serve.await {
        Ok(()) => tracing::info!("gateway stopped after graceful shutdown"),
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

/// Start the gateway with a pre-bound TCP listener and an external shutdown signal.
///
/// Used by `fastclaw-app` to embed the gateway in-process with a caller-controlled
/// listener (for port-conflict resolution) and a oneshot channel for graceful shutdown.
pub async fn run_with_listener(
    config: FastClawConfig,
    listener: tokio::net::TcpListener,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    ensure_auth_for_exposed_bind(&config)?;
    let state = AppState::new(config).await?;
    serve_with_state(state, listener, shutdown_rx).await
}

/// Start the gateway with a pre-built [`AppState`] and a pre-bound TCP listener.
///
/// This allows the caller to retain a clone of `AppState` for direct in-process access
/// (e.g. Tauri IPC commands) while the gateway serves HTTP/WS traffic on the listener.
pub async fn serve_with_state(
    state: AppState,
    listener: tokio::net::TcpListener,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    ensure_auth_for_exposed_bind(&state.config)?;
    let auth_config = fastclaw_security::AuthConfig {
        enabled: !state.config.security.api_keys.is_empty(),
        api_keys: state.config.security.api_keys.clone(),
    };
    let auth = ApiKeyAuth::new(&auth_config);

    spawn_config_watcher(state.clone());
    spawn_cron_scheduler(state.clone());

    let app = build_app(state, auth);

    let local_addr = listener.local_addr()?;
    tracing::info!(%local_addr, "embedded gateway starting");

    let serve = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.await;
        tracing::info!("embedded gateway: graceful shutdown requested");
    });

    serve.await?;
    Ok(())
}

/// SIGINT, SIGTERM (Unix), and Ctrl+C (all platforms) initiate graceful shutdown.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())
            .expect("failed to install SIGTERM listener for graceful shutdown");
        let mut sigint = signal(SignalKind::interrupt())
            .expect("failed to install SIGINT listener for graceful shutdown");
        tokio::select! {
            _ = sigterm.recv() => {},
            _ = sigint.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}

/// Start the cron scheduler in a background task.
fn spawn_cron_scheduler(state: AppState) {
    use std::sync::Arc;

    struct GatewayCronTrigger {
        state: AppState,
    }

    #[async_trait::async_trait]
    impl fastclaw_cron::JobTrigger for GatewayCronTrigger {
        async fn trigger_agent_chat(
            &self,
            agent_id: &str,
            message: &str,
            session_id: Option<&str>,
        ) -> anyhow::Result<String> {
            let sid = session_id
                .map(String::from)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let title_preview: String = message.chars().take(30).collect();
            let title = format!("[定时] {title_preview}");
            let _ = self
                .state
                .session_store
                .create_session(&sid, agent_id, Some(&title))
                .await;

            let user_msg = fastclaw_core::types::ChatMessage {
                role: fastclaw_core::types::Role::User,
                content: Some(serde_json::Value::String(message.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            };
            let _ = self.state.session_store.append_message(&sid, &user_msg).await;

            let mut request = fastclaw_core::types::ChatRequest {
                agent_id: Some(agent_id.to_string()),
                session_id: Some(sid.clone()),
                messages: vec![user_msg],
                model: None,
                stream: false,
                max_tokens: None,
                temperature: None,
                tools: None,
                slash_intent: None,
                work_dir: None,
            };
            let agent_config = {
                let router = self.state.router.read().await;
                router.resolve(&request)?.clone()
            };
            let tool_definition_count =
                crate::routes::filtered_tool_definitions(&self.state.tool_registry, &agent_config)
                    .map_or(0, |d| d.len());
            let llm_override = crate::routes::apply_model_router_for_chat(
                &self.state,
                &agent_config,
                &mut request,
                tool_definition_count,
            );
            let result = self
                .state
                .runtime
                .execute(
                    &agent_config,
                    &request,
                    &self.state.tool_registry,
                    llm_override,
                )
                .await?;
            let charged_model = result.response.model.clone();
            crate::routes::record_chat_budget_actual(
                &self.state,
                charged_model.as_str(),
                result.response.usage.as_ref(),
            );
            let reply = result
                .response
                .choices
                .first()
                .and_then(|c| c.message.text_content())
                .unwrap_or_default();

            let assistant_msg = fastclaw_core::types::ChatMessage {
                role: fastclaw_core::types::Role::Assistant,
                content: Some(serde_json::Value::String(reply.clone())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            };
            let _ = self.state.session_store.append_message(&sid, &assistant_msg).await;

            Ok(reply)
        }

        async fn trigger_dag_execute(
            &self,
            dag: &serde_json::Value,
            input: Option<&serde_json::Value>,
        ) -> anyhow::Result<serde_json::Value> {
            let dag_json = serde_json::to_string(dag)?;
            let def = fastclaw_dag::DagDefinition::from_json(&dag_json)?;
            let graph = fastclaw_dag::DagGraph::build(&def)?;
            let handler = Arc::new(routes::CronDagHandler {
                tool_registry: self.state.tool_registry.clone(),
                runtime: self.state.runtime.clone(),
                router: self.state.router.clone(),
            });
            let dag_run_id = uuid::Uuid::new_v4().to_string();
            let executor = fastclaw_dag::DagExecutor::with_checkpoint_store(
                graph,
                handler,
                self.state.dag_checkpoint_store.clone(),
                dag_run_id,
            );
            let ctx = if let Some(inp) = input {
                fastclaw_dag::ExecutionContext::with_input(inp.clone())
            } else {
                fastclaw_dag::ExecutionContext::new()
            };
            let result = executor.run(ctx).await?;
            Ok(serde_json::to_value(result.snapshot().await)?)
        }

        async fn trigger_webhook(
            &self,
            url: &str,
            method: Option<&str>,
            body: Option<&serde_json::Value>,
        ) -> anyhow::Result<()> {
            fastclaw_security::ssrf::ssrf_check_url(url)
                .map_err(|e| anyhow::anyhow!("SSRF check failed for cron webhook: {e}"))?;
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(fastclaw_security::ssrf::ssrf_safe_redirect_policy())
                .build()?;
            let req = match method.unwrap_or("POST") {
                "GET" => client.get(url),
                "PUT" => client.put(url),
                "DELETE" => client.delete(url),
                _ => client.post(url),
            };
            let req = if let Some(b) = body { req.json(b) } else { req };
            let resp = req.send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("webhook returned {}", resp.status());
            }
            Ok(())
        }

        async fn on_job_completed(
            &self,
            _job_id: &str,
            job_name: &str,
            output: Option<&str>,
        ) {
            let preview: String = output
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();

            let nid = uuid::Uuid::new_v4().to_string();
            let body = if preview.is_empty() { "执行完成".to_string() } else { format!("完成：{preview}") };
            if let Err(e) = self.state.notification_store
                .insert(&nid, "cron", job_name, &body, None)
                .await
            {
                tracing::warn!(error = %e, "failed to persist cron completion notification");
            }

            let unread = self.state.notification_store.unread_count().await.unwrap_or(0);
            let event = serde_json::json!({
                "type": "event",
                "event": "notification.new",
                "data": {
                    "id": nid,
                    "category": "cron",
                    "title": job_name,
                    "body": body,
                    "isRead": false,
                    "unreadCount": unread,
                }
            });
            let _ = self.state.ws_broadcast.send(event.to_string());
        }

        async fn on_job_failed(&self, job_id: &str, job_name: &str, error: &str) {
            let nid = uuid::Uuid::new_v4().to_string();
            let body = format!("失败：{error}");
            let detail = Some(format!("Job ID: {job_id}\nError: {error}"));
            if let Err(e) = self.state.notification_store
                .insert(&nid, "cron", job_name, &body, detail.as_deref())
                .await
            {
                tracing::warn!(error = %e, "failed to persist cron failure notification");
            }

            let unread = self.state.notification_store.unread_count().await.unwrap_or(0);
            let event = serde_json::json!({
                "type": "event",
                "event": "notification.new",
                "data": {
                    "id": nid,
                    "category": "cron",
                    "title": job_name,
                    "body": body,
                    "isRead": false,
                    "unreadCount": unread,
                }
            });
            let _ = self.state.ws_broadcast.send(event.to_string());
        }
    }

    let trigger = Arc::new(GatewayCronTrigger {
        state: state.clone(),
    });
    let wake = state.cron_wake.clone();
    let scheduler =
        fastclaw_cron::CronScheduler::with_wake(state.cron_store.clone(), trigger, wake);

    tokio::spawn(async move {
        if let Err(e) = scheduler.run().await {
            tracing::error!(error = %e, "cron scheduler exited with error");
        }
    });

    tracing::info!("cron scheduler started");
}

/// Watch the agents config directory and hot-reload on changes.
/// On Unix, `SIGHUP` triggers the same `reload_agents` path as the file watcher.
fn spawn_config_watcher(state: AppState) {
    use notify::{RecursiveMode, Watcher};

    let agents_dir =
        fastclaw_core::paths::resolve_agents_dir_from(Some(&state.config.paths));
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    #[cfg(unix)]
    spawn_sighup_reload_trigger(reload_tx.clone());

    let mut started_file_watcher = false;
    if agents_dir.exists() {
        let reload_tx_thread = reload_tx.clone();
        std::thread::spawn({
            let agents_dir = agents_dir.clone();
            move || {
                let mut watcher =
                    match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                        if let Ok(event) = res {
                            if event.kind.is_modify()
                                || event.kind.is_create()
                                || event.kind.is_remove()
                            {
                                let _ = reload_tx_thread.send(());
                            }
                        }
                    }) {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to create file watcher");
                            return;
                        }
                    };

                if let Err(e) = watcher.watch(&agents_dir, RecursiveMode::Recursive) {
                    tracing::warn!(
                        error = %e,
                        dir = %agents_dir.display(),
                        "failed to watch agents dir"
                    );
                    return;
                }

                tracing::info!(dir = %agents_dir.display(), "watching agents config for hot-reload");

                loop {
                    std::thread::sleep(Duration::from_secs(86400));
                }
            }
        });
        started_file_watcher = true;
    } else {
        tracing::debug!("config/agents not found, skipping file watcher");
    }

    if !started_file_watcher {
        #[cfg(not(unix))]
        return;
    }

    tokio::spawn(async move {
        let mut last_reload = std::time::Instant::now() - Duration::from_secs(3600);
        while reload_rx.recv().await.is_some() {
            while reload_rx.try_recv().is_ok() {}
            let min_gap = Duration::from_millis(500);
            let elapsed = last_reload.elapsed();
            if elapsed < min_gap {
                tokio::time::sleep(min_gap - elapsed).await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            last_reload = std::time::Instant::now();
            match state.reload_agents().await {
                Ok(count) => {
                    fastclaw_observe::record_agent_reload(count);
                    tracing::info!(count, "hot-reload: agents reloaded");
                }
                Err(e) => {
                    tracing::error!(error = %e, "hot-reload: failed to reload agents");
                }
            }
        }
    });
}

#[cfg(unix)]
fn spawn_sighup_reload_trigger(reload_tx: tokio::sync::mpsc::UnboundedSender<()>) {
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut stream = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to register SIGHUP for agent hot-reload");
                return;
            }
        };
        while stream.recv().await.is_some() {
            let _ = reload_tx.send(());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::config::{BindMode, FastClawConfig};

    #[test]
    fn exposed_bind_requires_api_key() {
        let mut cfg = FastClawConfig::default();
        cfg.gateway.bind = BindMode::Lan;
        cfg.security.api_keys.clear();
        assert!(ensure_auth_for_exposed_bind(&cfg).is_err());
    }

    #[test]
    fn loopback_allows_empty_api_key() {
        let mut cfg = FastClawConfig::default();
        cfg.gateway.bind = BindMode::Loopback;
        cfg.security.api_keys.clear();
        assert!(ensure_auth_for_exposed_bind(&cfg).is_ok());
    }

    /// Ensures Unix signal kinds used for hot reload stay available at compile time.
    #[cfg(unix)]
    #[test]
    fn sighup_signal_kind_for_hot_reload() {
        use tokio::signal::unix::SignalKind;
        let _ = SignalKind::hangup();
    }
}
