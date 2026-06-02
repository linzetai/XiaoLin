use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::bus::{AgentMessage, MessageBus, MessageTarget};
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};
use xiaolin_session::SessionStore;

pub fn session_inbox_topic(session_id: &str) -> String {
    format!("xiaolin.session.{session_id}")
}

pub struct SessionsSpawnTool {
    sessions: Arc<SessionStore>,
    bus: Arc<MessageBus>,
}

impl SessionsSpawnTool {
    pub fn new(sessions: Arc<SessionStore>, bus: Arc<MessageBus>) -> Self {
        Self { sessions, bus }
    }
}

#[async_trait]
impl Tool for SessionsSpawnTool {
    fn name(&self) -> &str {
        "sessions_spawn"
    }

    fn description(&self) -> &str {
        "Create a new persisted chat session for agent_id, then publish the first user message to that session's inbox topic on the message bus. Response JSON includes session_id for later sessions_send calls. \
         Use sessions_spawn for parallel threads, a fresh context window, or automation that must hand work to another session on the same gateway. \
         sessions_send requires an existing id—always store the returned session_id. Optional title labels the thread in UIs only; it does not steer the model. \
         Anti-pattern: spawning in a loop without subscribers or follow-up handling. \
         Example: {\"agent_id\": \"main\", \"message\": \"Summarize PR #42 with risks\", \"title\": \"PR 42 review\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "agent_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Agent id string configured on the gateway (e.g. 'main', 'reviewer'). Must match a routable agent—typos yield sessions no worker will pick up."
            }),
        );
        props.insert(
            "message".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "First user-authored message delivered to the session inbox as if the user typed it—keep it self-contained (include goals, constraints, links) because the new session has no prior context."
            }),
        );
        props.insert(
            "title".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional human-readable label for dashboards (e.g. 'Release checklist'). Omit or null if not needed."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["agent_id".to_string(), "message".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "sessions_spawn: arguments are not valid JSON: {e}. \
                 Pass {{\"agent_id\": \"...\", \"message\": \"...\", \"title\": \"...\"}} with double-quoted keys; title is optional."
            )),
        };

        let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => {
                return ToolResult::err(
                    "sessions_spawn is missing or empty required string field 'agent_id'. \
                 Example: {\"agent_id\": \"main\", \"message\": \"Hello new thread\"}. \
                 Trim whitespace; null is not accepted."
                        .to_string(),
                )
            }
        };

        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolResult::err(
                    "sessions_spawn is missing or empty required string field 'message'. \
                 Provide the first user message for the new session—empty strings are rejected."
                        .to_string(),
                )
            }
        };

        let title = args.get("title").and_then(|v| v.as_str());

        let session_id = uuid::Uuid::new_v4().to_string();

        if let Err(e) = self
            .sessions
            .create_session(&session_id, agent_id, title)
            .await
        {
            return ToolResult::err(format!(
                "sessions_spawn could not persist the new session in SessionStore: {e}. \
                 What to do next: verify the session backend (disk/DB) is reachable and not full; fix agent_id if the store enforces foreign keys; retry once—do not assume the session exists if this errors."
            ));
        }

        let topic = session_inbox_topic(&session_id);
        let bus_msg = AgentMessage::new(
            "gateway".into(),
            MessageTarget::Topic(topic),
            "session.message",
            serde_json::json!({
                "session_id": session_id,
                "message": message,
            }),
        );

        if let Err(e) = self.bus.send(bus_msg).await {
            return ToolResult::err(format!(
                "sessions_spawn created session_id={session_id} but failed to publish the initial inbox message on the bus: {e}. \
                 What to do next: check broker/backpressure configuration; you may need operators to reconcile the empty session; avoid reusing the same client-generated id—spawn again if policy allows."
            ));
        }

        ToolResult::ok(
            serde_json::json!({
                "session_id": session_id,
                "ok": true,
            })
            .to_string(),
        )
    }
}

pub struct SessionsSendTool {
    sessions: Arc<SessionStore>,
    bus: Arc<MessageBus>,
}

impl SessionsSendTool {
    pub fn new(sessions: Arc<SessionStore>, bus: Arc<MessageBus>) -> Self {
        Self { sessions, bus }
    }
}

#[async_trait]
impl Tool for SessionsSendTool {
    fn name(&self) -> &str {
        "sessions_send"
    }

    fn description(&self) -> &str {
        "Publish another user message to an existing session's inbox topic. The session row must already exist—normally created via sessions_spawn (or the host UI) before the first send. \
         Use sessions_send for follow-ups in a spawned thread, multi-step automations, or injecting user text while a human also participates. \
         Fire-and-forget on the bus: it does not await model replies; workers consume the topic like normal user traffic. \
         Anti-pattern: fabricating UUIDs—reuse ids from spawn responses or session lists. \
         Example: {\"session_id\": \"550e8400-e29b-41d4-a716-446655440000\", \"message\": \"Add stack trace:\\n...\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "session_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Existing session UUID string returned by sessions_spawn or the session API—must match exactly including case."
            }),
        );
        props.insert(
            "message".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "User message body to append to the session inbox (plain text; include your own formatting such as Markdown if downstream agents expect it)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["session_id".to_string(), "message".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "sessions_send: arguments are not valid JSON: {e}. \
                 Pass {{\"session_id\": \"...\", \"message\": \"...\"}} with double-quoted keys, then retry."
            )),
        };

        let session_id = match args.get("session_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => {
                return ToolResult::err(
                    "sessions_send is missing or empty required string field 'session_id'. \
                 Reuse the session_id returned by sessions_spawn or from your session list UI."
                        .to_string(),
                )
            }
        };

        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolResult::err(
                    "sessions_send is missing or empty required string field 'message'. \
                 Provide the user text you want delivered to that session inbox."
                        .to_string(),
                )
            }
        };

        match self.sessions.get_session(session_id).await {
            Ok(None) => {
                return ToolResult::err(format!(
                    "sessions_send: session not found for session_id '{session_id}'. \
                     What went wrong: SessionStore has no record of that id (typo, expired session, or different store). \
                     What to do next: spawn a new session with sessions_spawn and use the fresh session_id, or list sessions through the host UI/API if available—do not fabricate UUIDs."
                ))
            }
            Ok(Some(_)) => {}
            Err(e) => {
                return ToolResult::err(format!(
                    "sessions_send could not look up session_id '{session_id}' in SessionStore: {e}. \
                     What to do next: treat the backend as unhealthy—retry once, then stop spamming sends and surface the error to the operator."
                ))
            }
        }

        let topic = session_inbox_topic(session_id);
        let bus_msg = AgentMessage::new(
            "gateway".into(),
            MessageTarget::Topic(topic),
            "session.message",
            serde_json::json!({
                "session_id": session_id,
                "message": message,
            }),
        );

        if let Err(e) = self.bus.send(bus_msg).await {
            return ToolResult::err(format!(
                "sessions_send could not publish to session_id '{session_id}' (topic session inbox): {e}. \
                 What to do next: verify the message bus is running and not saturated; retry after backoff; if sends always fail, report a gateway/bus misconfiguration—the session row may still exist even though delivery failed."
            ));
        }

        ToolResult::ok(serde_json::json!({ "ok": true }).to_string())
    }
}

#[cfg(test)]
mod session_tools_tests {
    use super::*;
    use crate::builtin_tools::register_session_tools;
    use xiaolin_core::tool::ToolRegistry;

    #[tokio::test]
    async fn sessions_spawn_creates_session_and_returns_id() {
        let store = Arc::new(SessionStore::open_memory().await.unwrap());
        let bus = Arc::new(MessageBus::new(16));
        let tool = SessionsSpawnTool::new(store.clone(), bus);

        let out = tool
            .execute(r#"{"agent_id": "agent-a", "message": "hello spawn", "title": "t1"}"#)
            .await;
        assert!(out.success, "{}", out.output);

        let v: serde_json::Value = serde_json::from_str(&out.output).unwrap();
        let sid = v["session_id"].as_str().unwrap();
        assert!(!sid.is_empty());

        let s = store.get_session(sid).await.unwrap().unwrap();
        assert_eq!(s.agent_id, "main");
        assert_eq!(s.title.as_deref(), Some("t1"));
    }

    #[tokio::test]
    async fn sessions_spawn_then_send_delivers_on_session_topic() {
        let store = Arc::new(SessionStore::open_memory().await.unwrap());
        let bus = Arc::new(MessageBus::new(16));
        let spawn_tool = SessionsSpawnTool::new(store.clone(), bus.clone());

        let spawn_out = spawn_tool
            .execute(r#"{"agent_id": "agent-z", "message": "initial", "title": "t"}"#)
            .await;
        assert!(spawn_out.success, "{}", spawn_out.output);

        let v: serde_json::Value = serde_json::from_str(&spawn_out.output).unwrap();
        let sid = v["session_id"].as_str().unwrap().to_string();

        let topic = session_inbox_topic(&sid);
        let mut rx = bus.subscribe_topic(&topic);

        let send_tool = SessionsSendTool::new(store, bus.clone());
        let send_out = send_tool
            .execute(
                &serde_json::json!({
                    "session_id": sid,
                    "message": "follow-up peer"
                })
                .to_string(),
            )
            .await;
        assert!(send_out.success, "{}", send_out.output);

        let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert_eq!(got.topic, "session.message");
        assert_eq!(got.payload["session_id"].as_str().unwrap(), sid);
        assert_eq!(got.payload["message"], "follow-up peer");
    }

    #[tokio::test]
    async fn sessions_send_delivers_on_bus_when_subscribed() {
        let store = Arc::new(SessionStore::open_memory().await.unwrap());
        store.create_session("sx", "ag", None).await.unwrap();
        let bus = Arc::new(MessageBus::new(16));
        let topic = session_inbox_topic("sx");
        let mut rx = bus.subscribe_topic(&topic);

        let tool = SessionsSendTool::new(store, bus.clone());
        let out = tool
            .execute(r#"{"session_id": "sx", "message": "from peer"}"#)
            .await;
        assert!(out.success, "{}", out.output);

        let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert_eq!(got.topic, "session.message");
        assert_eq!(got.payload["session_id"], "sx");
        assert_eq!(got.payload["message"], "from peer");
    }

    #[tokio::test]
    async fn sessions_send_unknown_session_fails() {
        let store = Arc::new(SessionStore::open_memory().await.unwrap());
        let bus = Arc::new(MessageBus::new(16));
        let tool = SessionsSendTool::new(store, bus);

        let out = tool
            .execute(r#"{"session_id": "nope", "message": "x"}"#)
            .await;
        assert!(!out.success);
        assert!(out.output.contains("not found"));
    }

    #[tokio::test]
    async fn register_session_tools_adds_definitions() {
        let reg = ToolRegistry::new();
        let store = Arc::new(SessionStore::open_memory().await.unwrap());
        let bus = Arc::new(MessageBus::new(8));
        register_session_tools(&reg, store, bus);
        let defs = reg.definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"sessions_spawn"));
        assert!(names.contains(&"sessions_send"));
    }
}
