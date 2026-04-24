use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use fastclaw_core::bus::{AgentMessage, MessageBus, MessageTarget};
use fastclaw_core::error::{FastClawError, FastClawResult};
use fastclaw_core::types::AgentId;

pub const DELEGATION_TOPIC: &str = "fastclaw.delegation";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationRequest {
    pub from_agent: AgentId,
    pub to_agent: AgentId,
    pub task: String,
    pub context: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationResult {
    pub success: bool,
    pub output: serde_json::Value,
}

#[deprecated(
    since = "0.1.0",
    note = "Use SubAgentDelegation::delegate() for streaming, lifecycle management, and typed sub-agents"
)]
pub async fn delegate_task(
    bus: Arc<MessageBus>,
    req: DelegationRequest,
    timeout: Duration,
) -> FastClawResult<DelegationResult> {
    let payload = serde_json::to_value(&req).map_err(FastClawError::Json)?;
    let mut msg = AgentMessage::new(
        req.from_agent.clone(),
        MessageTarget::Agent(req.to_agent.clone()),
        DELEGATION_TOPIC,
        payload,
    );
    bus.sign_if_hmac(&mut msg)?;
    let reply = bus.request(msg, timeout).await?;
    serde_json::from_value(reply.payload).map_err(FastClawError::Json)
}

pub fn delegation_reply(incoming: &AgentMessage, from_agent: AgentId, result: DelegationResult) -> FastClawResult<AgentMessage> {
    let payload = serde_json::to_value(&result).map_err(FastClawError::Json)?;
    Ok(incoming.reply(from_agent, payload))
}

/// Build a reply and sign it for HMAC-enabled buses.
pub fn delegation_reply_signed(
    bus: &MessageBus,
    incoming: &AgentMessage,
    from_agent: AgentId,
    result: DelegationResult,
) -> FastClawResult<AgentMessage> {
    let mut reply = delegation_reply(incoming, from_agent, result)?;
    bus.sign_if_hmac(&mut reply)?;
    Ok(reply)
}

/// Interpret [`DelegationResult::output`] as a single string for logging or parent summaries.
pub fn delegation_output_to_text(output: &Value) -> FastClawResult<String> {
    if let Some(s) = output.as_str() {
        return Ok(s.to_string());
    }
    if let Some(obj) = output.as_object() {
        for key in ["argument", "text", "content", "opinion", "output"] {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                return Ok(s.to_string());
            }
        }
    }
    Err(FastClawError::Agent(
        "delegation output is not a string and has no known text field (argument|text|content|opinion|output)"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::bus::AgentMessage;
    use fastclaw_core::error::FastClawError;
    use serde_json::json;

    #[tokio::test]
    async fn delegate_task_times_out_when_worker_silent() {
        let bus = Arc::new(MessageBus::new(32));
        // Worker is registered but never reads the mailbox — no reply is sent.
        let _rx = bus.register("worker").await;

        let req = DelegationRequest {
            from_agent: "lead".into(),
            to_agent: "worker".into(),
            task: "compute".into(),
            context: json!({}),
        };
        let err = delegate_task(bus, req, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(matches!(err, FastClawError::BusRequestTimeout(_)));
    }

    #[test]
    fn delegation_reply_preserves_correlation() {
        let incoming = AgentMessage::new(
            "lead".into(),
            MessageTarget::Agent("worker".into()),
            DELEGATION_TOPIC,
            json!({"task": "x"}),
        );
        let expected_id = incoming.id.clone();
        let result = DelegationResult {
            success: true,
            output: json!({"ok": true}),
        };
        let reply = delegation_reply(&incoming, "worker".into(), result).unwrap();
        assert_eq!(reply.reply_to, Some(expected_id));
        match &reply.to {
            MessageTarget::Agent(id) => assert_eq!(id, "lead"),
            other => panic!("expected reply to lead agent, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn delegation_request_reply() {
        let bus = Arc::new(MessageBus::new(32));
        let mut rx = bus.register("worker").await;
        let bus_clone = bus.clone();
        tokio::spawn(async move {
            if let Some(msg) = rx.recv().await {
                if msg.topic == DELEGATION_TOPIC {
                    let result = DelegationResult {
                        success: true,
                        output: json!({"n": 7}),
                    };
                    let reply = delegation_reply(&msg, "worker".into(), result).unwrap();
                    assert_eq!(reply.reply_to, Some(msg.id.clone()));
                    let _ = bus_clone.send(reply).await;
                }
            }
        });

        let req = DelegationRequest {
            from_agent: "lead".into(),
            to_agent: "worker".into(),
            task: "compute".into(),
            context: json!({}),
        };
        let out = delegate_task(bus, req, Duration::from_secs(3))
            .await
            .unwrap();
        assert!(out.success);
        assert_eq!(out.output["n"], 7);
    }
}
