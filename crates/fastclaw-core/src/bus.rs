use std::collections::HashMap;

use constant_time_eq::constant_time_eq;
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};

use crate::error::{FastClawError, FastClawResult};
use crate::types::AgentId;

type HmacSha256 = Hmac<Sha256>;

/// Default hop budget for [`AgentMessage::ttl`] (anti-loop).
pub const DEFAULT_AGENT_MESSAGE_TTL: u8 = 20;

/// Default maximum delegation depth (see [`AgentMessage::max_depth`] / [`AgentMessage::depth`]).
pub const DEFAULT_MAX_DELEGATION_DEPTH: u8 = 3;

fn default_agent_message_ttl() -> u8 {
    DEFAULT_AGENT_MESSAGE_TTL
}

fn default_max_delegation_depth() -> u8 {
    DEFAULT_MAX_DELEGATION_DEPTH
}

fn default_message_depth() -> u8 {
    0
}

/// Builds deterministic UTF-8 input for HMAC over an [`AgentMessage`].
///
/// Includes all fields except [`AgentMessage::signature`] so the MAC can be computed and verified
/// consistently. [`AgentMessage::ttl`] and [`AgentMessage::depth`] are included: each forwarding
/// agent should re-sign after adjusting hop counters.
pub fn signing_material(msg: &AgentMessage) -> FastClawResult<Vec<u8>> {
    let to_str = serde_json::to_string(&msg.to)?;
    let payload_str = serde_json::to_string(&msg.payload)?;
    let reply = msg.reply_to.as_deref().unwrap_or("");
    let line = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        msg.id,
        msg.from,
        to_str,
        msg.topic,
        payload_str,
        reply,
        msg.timestamp,
        msg.ttl,
        msg.max_depth,
        msg.depth,
    );
    Ok(line.into_bytes())
}

fn compute_hmac(secret: &[u8], msg: &AgentMessage) -> FastClawResult<[u8; 32]> {
    let material = signing_material(msg)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| {
        FastClawError::Config("bus HMAC key must be non-empty".to_string())
    })?;
    mac.update(&material);
    Ok(mac.finalize().into_bytes().into())
}

/// Sets [`AgentMessage::signature`] to hex-encoded HMAC-SHA256 of [`signing_material`].
pub fn sign_message(msg: &mut AgentMessage, secret: &[u8]) -> FastClawResult<()> {
    let digest = compute_hmac(secret, msg)?;
    msg.signature = Some(hex::encode(digest));
    Ok(())
}

/// Max age (in seconds) for a signed message to be considered valid.
const MESSAGE_MAX_AGE_SECS: i64 = 300;
/// Max future drift (in seconds) to tolerate due to clock skew.
const MESSAGE_MAX_FUTURE_SECS: i64 = 30;

/// Verifies [`AgentMessage::signature`] for the current field values using `secret`.
///
/// Also rejects messages whose `timestamp` is too far in the past (>5 min) or
/// future (>30 s) to provide replay protection.  Messages with unparseable
/// timestamps are **rejected** (fail-closed).
pub fn verify_message(msg: &AgentMessage, secret: &[u8]) -> bool {
    match chrono::DateTime::parse_from_rfc3339(&msg.timestamp) {
        Ok(ts) => {
            let age = chrono::Utc::now().signed_duration_since(ts);
            if age.num_seconds() > MESSAGE_MAX_AGE_SECS
                || age.num_seconds() < -MESSAGE_MAX_FUTURE_SECS
            {
                tracing::warn!(
                    msg_id = %msg.id,
                    timestamp = %msg.timestamp,
                    age_secs = age.num_seconds(),
                    "rejecting message: timestamp outside valid window"
                );
                return false;
            }
        }
        Err(_) => {
            tracing::warn!(
                msg_id = %msg.id,
                timestamp = %msg.timestamp,
                "rejecting message: unparseable timestamp"
            );
            return false;
        }
    }

    let Some(sig_hex) = msg.signature.as_ref() else {
        tracing::warn!(
            msg_id = %msg.id,
            from = %msg.from,
            "HMAC: rejecting message with missing signature"
        );
        return false;
    };
    let Ok(expected) = compute_hmac(secret, msg) else {
        tracing::warn!(msg_id = %msg.id, "HMAC: failed to compute expected MAC");
        return false;
    };
    let Ok(decoded) = hex::decode(sig_hex.as_bytes()) else {
        tracing::warn!(msg_id = %msg.id, "HMAC: signature is not valid hex");
        return false;
    };
    if decoded.len() != expected.len() {
        tracing::warn!(msg_id = %msg.id, "HMAC: signature length mismatch");
        return false;
    }
    let valid = constant_time_eq(&decoded, &expected);
    if !valid {
        tracing::warn!(
            msg_id = %msg.id,
            from = %msg.from,
            topic = %msg.topic,
            "HMAC: signature verification failed"
        );
    }
    valid
}

/// A message exchanged between agents on the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub from: AgentId,
    pub to: MessageTarget,
    pub topic: String,
    pub payload: serde_json::Value,
    pub reply_to: Option<String>,
    pub timestamp: String,
    /// Remaining hop budget; decremented on each [`MessageBus::send`]. At `0`
    /// on entry, the message is dropped and never delivered.
    #[serde(default = "default_agent_message_ttl")]
    pub ttl: u8,
    /// Maximum allowed [`AgentMessage::depth`] on ingress to [`MessageBus::send`] before the
    /// message is dropped (delegation nesting cap).
    #[serde(default = "default_max_delegation_depth")]
    pub max_depth: u8,
    /// Delegation depth; incremented on each successful [`MessageBus::send`] hop.
    #[serde(default = "default_message_depth")]
    pub depth: u8,
    /// Hex-encoded HMAC-SHA256 over [`signing_material`], when bus signing is in use.
    #[serde(default)]
    pub signature: Option<String>,
}

impl AgentMessage {
    pub fn new(from: AgentId, to: MessageTarget, topic: &str, payload: serde_json::Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from,
            to,
            topic: topic.to_string(),
            payload,
            reply_to: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            ttl: DEFAULT_AGENT_MESSAGE_TTL,
            max_depth: DEFAULT_MAX_DELEGATION_DEPTH,
            depth: 0,
            signature: None,
        }
    }

    pub fn reply(&self, from: AgentId, payload: serde_json::Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from,
            to: MessageTarget::Agent(self.from.clone()),
            topic: self.topic.clone(),
            payload,
            reply_to: Some(self.id.clone()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            ttl: DEFAULT_AGENT_MESSAGE_TTL,
            max_depth: DEFAULT_MAX_DELEGATION_DEPTH,
            depth: 0,
            signature: None,
        }
    }
}

/// Where a message is directed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageTarget {
    Agent(AgentId),
    Broadcast,
    Topic(String),
}

/// Per-agent mailbox: incoming messages arrive here.
struct Mailbox {
    sender: mpsc::Sender<AgentMessage>,
}

/// The central message bus for inter-agent communication.
///
/// Supports four patterns:
/// 1. **Direct** ([`MessageTarget::Agent`]): deliver to one registered agent mailbox
/// 2. **Broadcast** ([`MessageTarget::Broadcast`]): all [`MessageBus::subscribe`] receivers
/// 3. **Topic** ([`MessageTarget::Topic`]): only agents that called [`MessageBus::subscribe_topic`]
///    for that topic name (distinct from broadcast)
/// 4. **Request-reply**: [`MessageBus::request`] pairs with [`AgentMessage::reply_to`]
///
/// LRU-style dedup cache for message IDs (replay protection within the time window).
const REPLAY_CACHE_CAPACITY: usize = 8192;

pub struct MessageBus {
    mailboxes: RwLock<HashMap<AgentId, Mailbox>>,
    broadcast_tx: broadcast::Sender<AgentMessage>,
    /// Named-topic fan-out; not the same as broadcast.
    topic_subscribers: DashMap<String, Vec<mpsc::Sender<AgentMessage>>>,
    pending_replies: RwLock<HashMap<String, oneshot::Sender<AgentMessage>>>,
    /// When set, [`MessageBus::send`] requires a valid [`AgentMessage::signature`] ([`verify_message`]).
    hmac_key: Option<Vec<u8>>,
    /// Seen message IDs for replay protection. Bounded to prevent unbounded growth.
    seen_ids: std::sync::Mutex<std::collections::VecDeque<String>>,
}

impl MessageBus {
    pub fn new(broadcast_capacity: usize) -> Self {
        let (broadcast_tx, _) = broadcast::channel(broadcast_capacity);
        Self {
            mailboxes: RwLock::new(HashMap::new()),
            broadcast_tx,
            topic_subscribers: DashMap::new(),
            pending_replies: RwLock::new(HashMap::new()),
            hmac_key: None,
            seen_ids: std::sync::Mutex::new(std::collections::VecDeque::with_capacity(REPLAY_CACHE_CAPACITY)),
        }
    }

    /// Same as [`MessageBus::new`] but enforces HMAC verification on every [`MessageBus::send`].
    pub fn new_with_hmac(broadcast_capacity: usize, hmac_key: Vec<u8>) -> Self {
        let mut bus = Self::new(broadcast_capacity);
        bus.hmac_key = Some(hmac_key);
        bus
    }

    /// Sign a message using the bus's HMAC key, if one is configured.
    /// Returns `Ok(())` even if no HMAC key is set (message left unsigned).
    pub fn sign_if_hmac(&self, msg: &mut AgentMessage) -> FastClawResult<()> {
        if let Some(key) = self.hmac_key.as_deref() {
            sign_message(msg, key)?;
        }
        Ok(())
    }

    /// Register an agent and return a receiver for its incoming messages.
    pub async fn register(&self, agent_id: &str) -> mpsc::Receiver<AgentMessage> {
        let (tx, rx) = mpsc::channel(256);
        let mut mailboxes = self.mailboxes.write().await;
        mailboxes.insert(AgentId::from(agent_id), Mailbox { sender: tx });
        tracing::debug!(agent_id, "agent registered on message bus");
        rx
    }

    /// Unregister an agent from the bus.
    pub async fn unregister(&self, agent_id: &str) {
        let mut mailboxes = self.mailboxes.write().await;
        mailboxes.remove(agent_id);
    }

    /// Send a message according to its target.
    pub async fn send(&self, mut msg: AgentMessage) -> FastClawResult<()> {
        if msg.ttl == 0 {
            tracing::warn!(
                msg_id = %msg.id,
                from = %msg.from,
                topic = %msg.topic,
                "message bus: dropping message with ttl=0 (not delivered)"
            );
            return Ok(());
        }
        if msg.depth >= msg.max_depth {
            tracing::warn!(
                msg_id = %msg.id,
                from = %msg.from,
                depth = msg.depth,
                max_depth = msg.max_depth,
                "message bus: dropping message (delegation depth limit)"
            );
            return Ok(());
        }
        if let Some(key) = self.hmac_key.as_deref() {
            if !verify_message(&msg, key) {
                tracing::warn!(msg_id = %msg.id, "message bus: invalid or missing HMAC signature");
                return Err(FastClawError::BusInvalidSignature);
            }
            if let Ok(mut seen) = self.seen_ids.lock() {
                if seen.iter().any(|id| id == &msg.id) {
                    tracing::warn!(msg_id = %msg.id, "message bus: rejecting replayed message");
                    return Err(FastClawError::Config(
                        "replayed message ID rejected".to_string(),
                    ));
                }
                seen.push_back(msg.id.clone());
                if seen.len() > REPLAY_CACHE_CAPACITY {
                    seen.pop_front();
                }
            }
        }

        msg.ttl = msg.ttl.saturating_sub(1);
        msg.depth = msg.depth.saturating_add(1);

        if let Some(reply_to) = &msg.reply_to {
            let mut pending = self.pending_replies.write().await;
            if let Some(tx) = pending.remove(reply_to) {
                let _ = tx.send(msg);
                return Ok(());
            }
        }

        let target = msg.to.clone();
        match target {
            MessageTarget::Agent(ref target_id) => {
                let mailboxes = self.mailboxes.read().await;
                if let Some(mailbox) = mailboxes.get(target_id) {
                    mailbox
                        .sender
                        .send(msg)
                        .await
                        .map_err(|_| FastClawError::BusMailboxClosed)?;
                } else {
                    return Err(FastClawError::BusAgentNotFound(target_id.to_string()));
                }
            }
            MessageTarget::Broadcast => {
                let _ = self.broadcast_tx.send(msg);
            }
            MessageTarget::Topic(ref topic_name) => {
                let senders: Vec<mpsc::Sender<AgentMessage>> = self
                    .topic_subscribers
                    .get(topic_name)
                    .map(|r| r.value().clone())
                    .unwrap_or_default();
                if senders.is_empty() {
                    tracing::warn!(topic = %topic_name, "no subscribers for bus topic");
                } else {
                    for tx in senders {
                        if tx.send(msg.clone()).await.is_err() {
                            tracing::debug!(topic = %topic_name, "topic subscriber mailbox closed");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Send a message and wait for a reply (request-reply pattern).
    pub async fn request(
        &self,
        msg: AgentMessage,
        timeout: std::time::Duration,
    ) -> FastClawResult<AgentMessage> {
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending_replies.write().await;
            pending.insert(msg.id.clone(), tx);
        }

        if let Err(e) = self.send(msg.clone()).await {
            self.cleanup_pending(&msg.id).await;
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => {
                self.cleanup_pending(&msg.id).await;
                Err(FastClawError::BusReplyClosed)
            }
            Err(_) => {
                self.cleanup_pending(&msg.id).await;
                Err(FastClawError::BusRequestTimeout(timeout))
            }
        }
    }

    /// Subscribe to broadcast messages.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentMessage> {
        self.broadcast_tx.subscribe()
    }

    /// Subscribe to messages sent with [`MessageTarget::Topic`] for `topic`.
    ///
    /// Broadcast subscribers ([`MessageBus::subscribe`]) do **not** receive these.
    /// Stale (closed) senders are pruned on each subscription. A cap of 256 subscribers
    /// per topic prevents unbounded growth.
    pub fn subscribe_topic(&self, topic: &str) -> mpsc::Receiver<AgentMessage> {
        const MAX_SUBSCRIBERS_PER_TOPIC: usize = 256;
        let (tx, rx) = mpsc::channel(256);
        let mut entry = self
            .topic_subscribers
            .entry(topic.to_string())
            .or_default();
        entry.retain(|s| !s.is_closed());
        if entry.len() >= MAX_SUBSCRIBERS_PER_TOPIC {
            tracing::warn!(topic, "topic subscriber limit reached, dropping oldest");
            entry.remove(0);
        }
        entry.push(tx);
        rx
    }

    /// Get count of registered agents.
    pub async fn agent_count(&self) -> usize {
        self.mailboxes.read().await.len()
    }

    /// List registered agent ids.
    pub async fn registered_agents(&self) -> Vec<AgentId> {
        self.mailboxes.read().await.keys().cloned().collect()
    }

    async fn cleanup_pending(&self, msg_id: &str) {
        let mut pending = self.pending_replies.write().await;
        pending.remove(msg_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn direct_message() {
        let bus = MessageBus::new(16);
        let mut rx: mpsc::Receiver<AgentMessage> = bus.register("agent-b").await;

        let msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Agent("agent-b".into()),
            "task",
            json!({"action": "summarize"}),
        );

        bus.send(msg).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.from, "agent-a");
        assert_eq!(received.topic, "task");
    }

    #[tokio::test]
    async fn broadcast_message() {
        let bus = MessageBus::new(16);
        let mut sub: broadcast::Receiver<AgentMessage> = bus.subscribe();

        let msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Broadcast,
            "status",
            json!({"status": "ready"}),
        );

        bus.send(msg).await.unwrap();

        let received = sub.recv().await.unwrap();
        assert_eq!(received.topic, "status");
    }

    #[tokio::test]
    async fn topic_routing_not_broadcast() {
        let bus = MessageBus::new(16);
        let mut topic_rx = bus.subscribe_topic("alerts");
        let mut broadcast_rx = bus.subscribe();

        let msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Topic("alerts".to_string()),
            "ping",
            json!({}),
        );
        bus.send(msg).await.unwrap();

        let received = topic_rx
            .recv()
            .await
            .expect("topic subscriber should receive");
        assert_eq!(received.topic, "ping");

        let broadcast_res =
            tokio::time::timeout(std::time::Duration::from_millis(50), broadcast_rx.recv()).await;
        assert!(
            broadcast_res.is_err() || broadcast_res.unwrap().is_err(),
            "broadcast must not receive topic-targeted messages"
        );
    }

    #[tokio::test]
    async fn request_reply() {
        let bus: Arc<MessageBus> = Arc::new(MessageBus::new(16));
        let mut rx: mpsc::Receiver<AgentMessage> = bus.register("agent-b").await;

        let bus_clone: Arc<MessageBus> = bus.clone();
        tokio::spawn(async move {
            if let Some(incoming) = rx.recv().await {
                let reply: AgentMessage = incoming.reply("agent-b".into(), json!({"result": 42}));
                bus_clone.send(reply).await.unwrap();
            }
        });

        let msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Agent("agent-b".into()),
            "compute",
            json!({"x": 10}),
        );

        let reply: AgentMessage = bus
            .request(msg, std::time::Duration::from_secs(5))
            .await
            .unwrap();

        assert_eq!(reply.from, "agent-b");
        assert_eq!(reply.payload["result"], 42);
    }

    #[tokio::test]
    async fn request_timeout() {
        let bus = MessageBus::new(16);
        let _rx: mpsc::Receiver<AgentMessage> = bus.register("agent-b").await;

        let msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Agent("agent-b".into()),
            "compute",
            json!({}),
        );

        let result = bus
            .request(msg, std::time::Duration::from_millis(100))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn register_unregister() {
        let bus = MessageBus::new(16);
        let _rx: mpsc::Receiver<AgentMessage> = bus.register("agent-a").await;
        assert_eq!(bus.agent_count().await, 1);

        bus.unregister("agent-a").await;
        assert_eq!(bus.agent_count().await, 0);
    }

    #[tokio::test]
    async fn ttl_zero_never_delivered() {
        let bus = MessageBus::new(16);
        let mut rx: mpsc::Receiver<AgentMessage> = bus.register("agent-b").await;

        let mut msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Agent("agent-b".into()),
            "ping",
            json!({}),
        );
        msg.ttl = 0;

        bus.send(msg).await.unwrap();

        let recv = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(
            recv.is_err() || recv.unwrap().is_none(),
            "mailbox must not receive ttl=0 messages"
        );
    }

    #[tokio::test]
    async fn ttl_one_delivered_once_then_exhausted() {
        let bus = MessageBus::new(16);
        let mut rx: mpsc::Receiver<AgentMessage> = bus.register("agent-b").await;

        let mut msg = AgentMessage::new(
            "agent-a".into(),
            MessageTarget::Agent("agent-b".into()),
            "ping",
            json!({}),
        );
        msg.ttl = 1;

        bus.send(msg.clone()).await.unwrap();
        let received = rx.recv().await.expect("first hop delivers");
        assert_eq!(received.ttl, 0);

        bus.send(received).await.unwrap();
        let recv = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(
            recv.is_err() || recv.unwrap().is_none(),
            "re-forwarding ttl=0 must not deliver"
        );
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let secret = b"shared-bus-secret-key";
        let mut msg = AgentMessage::new(
            "a".into(),
            MessageTarget::Agent("b".into()),
            "t",
            json!({"k": 1}),
        );
        sign_message(&mut msg, secret).unwrap();
        assert!(verify_message(&msg, secret));
    }

    #[test]
    fn verify_fails_after_payload_tamper() {
        let secret = b"key";
        let mut msg = AgentMessage::new(
            "a".into(),
            MessageTarget::Agent("b".into()),
            "t",
            json!({"honest": true}),
        );
        sign_message(&mut msg, secret).unwrap();
        msg.payload = json!({"honest": false});
        assert!(!verify_message(&msg, secret));
    }

    #[tokio::test]
    async fn bus_with_hmac_rejects_unsigned() {
        let bus = MessageBus::new_with_hmac(8, b"sekrit".to_vec());
        let mut rx = bus.register("b").await;
        let msg = AgentMessage::new(
            "a".into(),
            MessageTarget::Agent("b".into()),
            "x",
            json!({}),
        );
        let err = bus.send(msg).await.unwrap_err();
        assert!(matches!(err, FastClawError::BusInvalidSignature));
        let recv = tokio::time::timeout(std::time::Duration::from_millis(30), rx.recv()).await;
        assert!(recv.is_err() || recv.unwrap().is_none());
    }

    #[tokio::test]
    async fn bus_with_hmac_accepts_signed() {
        let secret = b"correct-horse-battery-staple";
        let bus = MessageBus::new_with_hmac(8, secret.to_vec());
        let mut rx = bus.register("b").await;
        let mut msg = AgentMessage::new(
            "a".into(),
            MessageTarget::Agent("b".into()),
            "x",
            json!({"n": 1}),
        );
        sign_message(&mut msg, secret).unwrap();
        bus.send(msg).await.unwrap();
        let got = rx.recv().await.expect("delivered");
        assert_eq!(got.depth, 1);
        assert_eq!(got.ttl, DEFAULT_AGENT_MESSAGE_TTL - 1);
    }

    #[tokio::test]
    async fn bus_hmac_second_hop_requires_resign() {
        let secret = b"ring-shared";
        let bus = MessageBus::new_with_hmac(8, secret.to_vec());
        let mut rx = bus.register("b").await;
        let mut msg = AgentMessage::new(
            "a".into(),
            MessageTarget::Agent("b".into()),
            "chain",
            json!({}),
        );
        sign_message(&mut msg, secret).unwrap();
        bus.send(msg).await.unwrap();
        let mut forwarded = rx.recv().await.expect("first hop");
        assert!(
            !verify_message(&forwarded, secret),
            "MAC was computed before ttl/depth hop adjustment"
        );
        forwarded.id = uuid::Uuid::new_v4().to_string();
        sign_message(&mut forwarded, secret).unwrap();
        assert!(verify_message(&forwarded, secret));
        bus.send(forwarded).await.unwrap();
        let _ = rx.recv().await.expect("second hop after re-sign");
    }

    #[tokio::test]
    async fn max_depth_blocks_after_budget() {
        let bus = MessageBus::new(16);
        let mut rx = bus.register("b").await;
        let mut msg = AgentMessage::new(
            "a".into(),
            MessageTarget::Agent("b".into()),
            "hop",
            json!({}),
        );
        msg.max_depth = 2;
        msg.depth = 0;
        bus.send(msg.clone()).await.unwrap();
        let m1 = rx.recv().await.unwrap();
        assert_eq!(m1.depth, 1);
        bus.send(m1.clone()).await.unwrap();
        let m2 = rx.recv().await.unwrap();
        assert_eq!(m2.depth, 2);
        bus.send(m2).await.unwrap();
        let recv = tokio::time::timeout(std::time::Duration::from_millis(40), rx.recv()).await;
        assert!(
            recv.is_err() || recv.unwrap().is_none(),
            "depth >= max_depth must not deliver"
        );
    }
}
