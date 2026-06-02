use std::collections::HashMap;

use crate::agent_config::AgentConfig;
use crate::config::{AgentsConfig, BindingConfig, DmScope};
use crate::types::{AgentId, ChatRequest};

/// Ephemeral route row managed at runtime (not persisted with config files).
#[derive(Debug, Clone)]
pub struct RuntimeRouteBinding {
    pub id: String,
    pub binding: BindingConfig,
}

/// Merge bindings so **runtime** rows are evaluated first (they win on equal [`MatchTier`]).
pub fn merge_runtime_bindings_first(
    runtime: &[RuntimeRouteBinding],
    file: &[BindingConfig],
) -> Vec<BindingConfig> {
    let mut out = Vec::with_capacity(runtime.len() + file.len());
    for r in runtime {
        out.push(r.binding.clone());
    }
    out.extend_from_slice(file);
    out
}

/// Resolved routing result: which agent should handle this message.
#[derive(Debug, Clone)]
pub struct RouteResult {
    pub agent_id: String,
    pub match_tier: MatchTier,
}

/// How specific the match was (higher tiers are more specific and win).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    Default = 0,
    ChannelWild = 1,
    AccountId = 2,
    Channel = 3,
    Peer = 4,
}

/// Route an inbound message to the main agent.
///
/// Multi-agent routing has been deprecated. This function now always returns
/// "main" as the agent ID. The binding rules are still evaluated for logging
/// purposes to aid migration, but the result is always "main".
pub fn resolve_route(
    bindings: &[BindingConfig],
    _agents: &AgentsConfig,
    channel: &str,
    account_id: Option<&str>,
    peer_kind: Option<&str>,
    peer_id: Option<&str>,
) -> RouteResult {
    // Evaluate bindings for deprecation logging only
    let mut legacy_match: Option<(MatchTier, &str)> = None;
    for binding in bindings {
        let m = &binding.match_rule;
        if let Some(ref ch) = m.channel {
            if ch != channel {
                continue;
            }
        } else {
            continue;
        }

        let tier = match (&m.peer, &m.account_id) {
            (Some(peer), _) => {
                let kind_ok = peer_kind.is_some_and(|k| k == peer.kind);
                let id_ok = peer_id.is_some_and(|i| i == peer.id);
                if kind_ok && id_ok {
                    MatchTier::Peer
                } else {
                    continue;
                }
            }
            (None, Some(acc)) if acc == "*" => MatchTier::ChannelWild,
            (None, Some(acc)) => {
                if account_id == Some(acc.as_str()) {
                    MatchTier::AccountId
                } else {
                    continue;
                }
            }
            (None, None) => MatchTier::Channel,
        };

        if legacy_match.as_ref().is_none_or(|(t, _)| tier > *t) {
            legacy_match = Some((tier, &binding.agent_id));
        }
    }

    if let Some((_tier, matched_agent)) = &legacy_match {
        if *matched_agent != "main" {
            tracing::warn!(
                matched_agent = %matched_agent,
                channel = %channel,
                "deprecated: channel binding matched non-main agent; routing to main instead"
            );
        }
    }

    RouteResult {
        agent_id: "main".to_string(),
        match_tier: legacy_match.map(|(t, _)| t).unwrap_or(MatchTier::Default),
    }
}

/// Build a session key based on DM scope for session isolation.
///
/// The `agent_id` parameter is accepted for backward compatibility but is no
/// longer included in the generated key. All sessions now belong to the single
/// main agent.
pub fn build_session_key(
    dm_scope: &DmScope,
    _agent_id: &str,
    channel: &str,
    account_id: Option<&str>,
    peer_id: &str,
    chat_type: &str,
) -> String {
    if chat_type == "group" {
        return match dm_scope {
            DmScope::Main | DmScope::PerPeer | DmScope::PerChannelPeer => {
                format!("group:{channel}:{peer_id}")
            }
            DmScope::PerAccountChannelPeer => {
                let acc = account_id.unwrap_or("default");
                format!("group:{acc}:{channel}:{peer_id}")
            }
        };
    }

    match dm_scope {
        DmScope::Main => format!("main:{peer_id}"),
        DmScope::PerPeer => format!("peer:{peer_id}"),
        DmScope::PerChannelPeer => format!("{channel}:{peer_id}"),
        DmScope::PerAccountChannelPeer => {
            let acc = account_id.unwrap_or("default");
            format!("{acc}:{channel}:{peer_id}")
        }
    }
}

/// Resolves [`ChatRequest`] payloads to the configured [`AgentConfig`] (by `agent_id`).
///
/// For inbound channel binding (peer / channel / account rules), use [`resolve_route`].
pub struct Router {
    agents: HashMap<AgentId, AgentConfig>,
    default_agent_id: Option<AgentId>,
}

impl Router {
    pub fn new(configs: Vec<AgentConfig>) -> Self {
        let default_agent_id = configs
            .iter()
            .find(|c| c.agent_id == "main")
            .or_else(|| configs.first())
            .map(|c| c.agent_id.clone());
        let agents = configs
            .into_iter()
            .map(|c| (c.agent_id.clone(), c))
            .collect();
        Self {
            agents,
            default_agent_id,
        }
    }

    /// Resolve the agent config for a given chat request.
    /// Priority: explicit `agent_id` in request → default agent → error.
    pub fn resolve(&self, request: &ChatRequest) -> anyhow::Result<&AgentConfig> {
        let agent_id = request
            .agent_id
            .as_deref()
            .or(self.default_agent_id.as_deref());

        match agent_id {
            Some(id) => self
                .agents
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("agent not found: {id}")),
            None => anyhow::bail!("no agent configured"),
        }
    }

    pub fn agent_by_id(&self, id: &str) -> Option<&AgentConfig> {
        self.agents.get(id)
    }

    pub fn list_agents(&self) -> Vec<&AgentConfig> {
        self.agents.values().collect()
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Returns `true` when `id` is the identifier of a registered agent.
    /// Used by the gateway to detect when a caller passes an agent ID as the
    /// `model` field (an OpenAI-compatible alias pattern).
    pub fn has_agent(&self, id: &str) -> bool {
        self.agents.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    fn make_binding(agent_id: &str, channel: &str, peer: Option<(&str, &str)>) -> BindingConfig {
        BindingConfig {
            agent_id: agent_id.to_string(),
            match_rule: BindingMatch {
                channel: Some(channel.to_string()),
                account_id: None,
                peer: peer.map(|(kind, id)| PeerMatch {
                    kind: kind.to_string(),
                    id: id.to_string(),
                }),
            },
        }
    }

    fn make_agents(ids: &[&str], default_idx: usize) -> AgentsConfig {
        AgentsConfig {
            defaults: AgentDefaults::default(),
            list: ids
                .iter()
                .enumerate()
                .map(|(i, id)| AgentEntry {
                    id: id.to_string(),
                    name: None,
                    workspace: None,
                    agent_dir: None,
                    model: None,
                    default: i == default_idx,
                    identity: None,
                    group_chat: None,
                    tools: None,
                    skills: None,
                })
                .collect(),
        }
    }

    #[test]
    fn always_routes_to_main() {
        let bindings = vec![
            make_binding("chat", "feishu", None),
            make_binding("opus", "feishu", Some(("direct", "+1234"))),
        ];
        let agents = make_agents(&["chat", "opus"], 0);

        let result = resolve_route(
            &bindings,
            &agents,
            "feishu",
            None,
            Some("direct"),
            Some("+1234"),
        );
        assert_eq!(result.agent_id, "main");
        assert_eq!(result.match_tier, MatchTier::Peer);
    }

    #[test]
    fn channel_match_still_returns_main() {
        let bindings = vec![make_binding("chat", "feishu", None)];
        let agents = make_agents(&["chat"], 0);

        let result = resolve_route(
            &bindings,
            &agents,
            "feishu",
            None,
            Some("direct"),
            Some("+5678"),
        );
        assert_eq!(result.agent_id, "main");
        assert_eq!(result.match_tier, MatchTier::Channel);
    }

    #[test]
    fn no_binding_match_returns_main() {
        let bindings = vec![make_binding("opus", "telegram", None)];
        let agents = make_agents(&["chat", "opus"], 0);

        let result = resolve_route(&bindings, &agents, "feishu", None, None, None);
        assert_eq!(result.agent_id, "main");
        assert_eq!(result.match_tier, MatchTier::Default);
    }

    #[test]
    fn runtime_bindings_merge_order() {
        let file = vec![make_binding("a", "ch1", None)];
        let runtime = vec![RuntimeRouteBinding {
            id: "r1".into(),
            binding: make_binding("b", "ch1", None),
        }];
        let merged = merge_runtime_bindings_first(&runtime, &file);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].agent_id, "b");
        assert_eq!(merged[1].agent_id, "a");
    }

    #[test]
    fn session_key_dm_scope() {
        assert_eq!(
            build_session_key(&DmScope::Main, "main", "feishu", None, "user1", "p2p"),
            "main:user1"
        );
        assert_eq!(
            build_session_key(&DmScope::PerPeer, "main", "feishu", None, "user1", "p2p"),
            "peer:user1"
        );
        assert_eq!(
            build_session_key(
                &DmScope::PerChannelPeer,
                "main",
                "feishu",
                None,
                "user1",
                "p2p"
            ),
            "feishu:user1"
        );
        assert_eq!(
            build_session_key(&DmScope::Main, "main", "feishu", None, "group1", "group"),
            "group:feishu:group1"
        );
    }
}
