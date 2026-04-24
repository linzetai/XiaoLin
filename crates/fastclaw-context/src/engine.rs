use async_trait::async_trait;
use fastclaw_core::types::{ChatMessage, Role};
use std::sync::Arc;

/// Context produced by the engine, ready to send to the LLM.
#[derive(Debug, Clone)]
pub struct AssembledContext {
    pub messages: Vec<ChatMessage>,
    pub injected_system_parts: Vec<String>,
    pub metadata: serde_json::Value,
}

/// Global token budget split across the six conceptual layers.
#[derive(Debug, Clone)]
pub struct ContextBudget {
    pub total_tokens: usize,
    pub system_ratio: f32,
    pub profile_ratio: f32,
    pub summary_ratio: f32,
    pub recall_ratio: f32,
    pub recent_ratio: f32,
    pub current_ratio: f32,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            total_tokens: 8192,
            system_ratio: 0.15,
            profile_ratio: 0.05,
            summary_ratio: 0.15,
            recall_ratio: 0.15,
            recent_ratio: 0.40,
            current_ratio: 0.10,
        }
    }
}

impl ContextBudget {
    fn normalized_ratios(&self) -> (f32, f32, f32, f32, f32, f32) {
        let sum = self.system_ratio
            + self.profile_ratio
            + self.summary_ratio
            + self.recall_ratio
            + self.recent_ratio
            + self.current_ratio;
        let denom = if sum <= f32::EPSILON { 1.0 } else { sum };
        (
            self.system_ratio / denom,
            self.profile_ratio / denom,
            self.summary_ratio / denom,
            self.recall_ratio / denom,
            self.recent_ratio / denom,
            self.current_ratio / denom,
        )
    }

    /// Per-layer token ceilings derived from [`Self::total_tokens`] and ratios.
    pub fn layer_token_limits(&self) -> LayerTokenLimits {
        let (s, p, u, r, rc, c) = self.normalized_ratios();
        let tot = self.total_tokens.max(64) as f32;
        LayerTokenLimits {
            system: (tot * s).floor().max(32.0) as usize,
            profile: (tot * p).floor().max(16.0) as usize,
            summary: (tot * u).floor().max(16.0) as usize,
            recall: (tot * r).floor().max(16.0) as usize,
            recent: (tot * rc).floor().max(64.0) as usize,
            current: (tot * c).floor().max(16.0) as usize,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayerTokenLimits {
    pub system: usize,
    pub profile: usize,
    pub summary: usize,
    pub recall: usize,
    pub recent: usize,
    pub current: usize,
}

/// Six-layer inputs prior to trimming and [`assemble_context`].
#[derive(Debug, Clone)]
pub struct ContextLayers {
    /// Layer 1 — fixed agent system prompt (+ optional static workspace system text).
    pub system_prompt: String,
    /// Layer 2 — user profile text (typically from [`crate::user_profile::UserProfile::to_prompt_text`]).
    pub profile_text: String,
    /// Layer 3 — compressed session / history summary.
    pub session_summary: String,
    /// Layer 4 — vector / memory recall block.
    pub recall_text: String,
    /// Layer 5 — sliding recent dialogue (excluding [`Self::current_input`] when split out).
    pub recent_messages: Vec<ChatMessage>,
    /// Layer 6 — the active user utterance (last turn).
    pub current_input: Option<ChatMessage>,
}

const DEFAULT_CHARS_PER_TOKEN: usize = 4;

fn estimate_message_tokens(msg: &ChatMessage, chars_per_token: usize) -> usize {
    let content_len = msg.content.as_ref().map_or(0, |c| {
        serde_json::to_string(c)
            .map(|s| s.len())
            .unwrap_or(0)
    });
    let tool_len = msg.tool_calls.as_ref().map_or(0, |tc| {
        tc.iter()
            .map(|t| t.function.name.len() + t.function.arguments.len())
            .sum::<usize>()
    });
    let overhead = 4usize;
    content_len / chars_per_token + tool_len / chars_per_token + overhead
}

fn truncate_to_token_budget(s: &str, max_tokens: usize, chars_per_token: usize) -> String {
    let max_chars = max_tokens.saturating_mul(chars_per_token);
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect::<String>() + "…"
}

/// Build the final `messages` array: system layers (1–4) then recent (5) then current user (6).
///
/// Trimming is **best-effort** using the same char/token heuristic as [`crate::compressor::ContextCompactor`].
pub fn assemble_context(budget: &ContextBudget, layers: &ContextLayers) -> AssembledContext {
    let cpt = DEFAULT_CHARS_PER_TOKEN;
    let lim = budget.layer_token_limits();

    let system_body = truncate_to_token_budget(&layers.system_prompt, lim.system, cpt);
    let profile_body = truncate_to_token_budget(&layers.profile_text.trim(), lim.profile, cpt);
    let summary_body = truncate_to_token_budget(&layers.session_summary.trim(), lim.summary, cpt);
    let recall_body = truncate_to_token_budget(&layers.recall_text.trim(), lim.recall, cpt);

    let mut recent_kept: Vec<ChatMessage> = Vec::new();
    let mut used_recent = 0usize;
    for m in layers.recent_messages.iter().rev() {
        let cost = estimate_message_tokens(m, cpt);
        if used_recent + cost > lim.recent && !recent_kept.is_empty() {
            break;
        }
        recent_kept.push(m.clone());
        used_recent += cost;
    }
    recent_kept.reverse();

    let current_trimmed = layers.current_input.as_ref().map(|m| {
        let mut m = m.clone();
        match &m.content {
            Some(serde_json::Value::Array(arr)) => {
                let new_parts: Vec<serde_json::Value> = arr
                    .iter()
                    .map(|part| {
                        if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                let trimmed = truncate_to_token_budget(t, lim.current, cpt);
                                serde_json::json!({"type": "text", "text": trimmed})
                            } else {
                                part.clone()
                            }
                        } else {
                            part.clone()
                        }
                    })
                    .collect();
                m.content = Some(serde_json::Value::Array(new_parts));
            }
            _ => {
                if let Some(t) = m.text_content() {
                    let trimmed = truncate_to_token_budget(&t, lim.current, cpt);
                    m.content = Some(serde_json::Value::String(trimmed));
                }
            }
        }
        m
    });

    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut injected = Vec::new();

    if !system_body.is_empty() {
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(system_body.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    if !profile_body.is_empty() {
        let block = format!("[User profile — inferred]\n{profile_body}");
        injected.push(block.clone());
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(block.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    if !summary_body.is_empty() {
        let block = format!("[Session summary]\n{summary_body}");
        injected.push(block.clone());
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(block.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    if !recall_body.is_empty() {
        let block = format!("[Retrieved context]\n{recall_body}");
        injected.push(block.clone());
        messages.push(ChatMessage {
            role: Role::System,
            content: Some(block.into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    messages.extend(recent_kept);

    if let Some(m) = current_trimmed {
        messages.push(m);
    }

    let metadata = serde_json::json!({
        "total_token_budget": budget.total_tokens,
        "layer_limits": {
            "system": lim.system,
            "profile": lim.profile,
            "summary": lim.summary,
            "recall": lim.recall,
            "recent": lim.recent,
            "current": lim.current,
        },
        "assembled_non_system": messages.iter().filter(|m| !matches!(m.role, Role::System)).count(),
    });

    AssembledContext {
        messages,
        injected_system_parts: injected,
        metadata,
    }
}

/// Input to the ingest phase — new user turn plus metadata.
#[derive(Debug, Clone)]
pub struct IngestInput {
    pub messages: Vec<ChatMessage>,
    pub agent_id: String,
    pub session_id: String,
    pub user_id: Option<String>,
}

/// Pluggable hook that runs during a specific lifecycle phase.
///
/// The engine invokes hooks in registration order within each phase:
///   bootstrap → ingest → assemble → compact → after_turn
#[async_trait]
pub trait ContextHook: Send + Sync {
    fn name(&self) -> &str;

    /// Called once at session start to inject persistent context (SOUL.md, USER.md, skills).
    async fn on_bootstrap(
        &self,
        _messages: &mut Vec<ChatMessage>,
        _agent_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Called when a new user message arrives. Can enrich with memory lookups, RAG, etc.
    async fn on_ingest(
        &self,
        _input: &IngestInput,
        _messages: &mut Vec<ChatMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Called to assemble final prompt. Can reorder, inject separators, enforce token budget.
    async fn on_assemble(&self, _messages: &mut Vec<ChatMessage>) -> anyhow::Result<()> {
        Ok(())
    }

    /// Called when context exceeds limits. Compacts/summarizes older messages.
    async fn on_compact(&self, _messages: &mut Vec<ChatMessage>) -> anyhow::Result<()> {
        Ok(())
    }

    /// Called after a full turn (user → agent → tool loops complete).
    /// Good for updating memory, logging token usage, etc.
    async fn on_after_turn(
        &self,
        _messages: &[ChatMessage],
        _agent_id: &str,
        _session_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Default non-system message count at which [`ContextEngine::process`] invokes compaction hooks.
/// Matches [`crate::compressor::DEFAULT_IMPORTANCE_MAX_MESSAGES`] so compaction aligns with
/// [`crate::compressor::CompactionStrategy::ImportanceBased`] (the [`Default`] for [`crate::compressor::CompactionStrategy`]).
pub const DEFAULT_COMPACTION_THRESHOLD: usize = crate::compressor::DEFAULT_IMPORTANCE_MAX_MESSAGES;

/// Inject [`DEFAULT_SYSTEM_REMINDER_TEXT`] after each multiple of this many user messages.
pub const DEFAULT_SYSTEM_REMINDER_INTERVAL_USER_TURNS: usize = 20;

/// Brief nudge inserted by [`SystemReminderHook`] on assemble (e.g. every 20 user turns).
pub const DEFAULT_SYSTEM_REMINDER_TEXT: &str = "[System reminder] You have access to tools. Use read_file, write_file, shell_exec, web_search and other tools to accomplish tasks. Don't hallucinate — verify information. Be concise.";

/// Pluggable context engine that manages the full context lifecycle.
///
/// Hooks are registered per-phase and run in order.
pub struct ContextEngine {
    hooks: Vec<Arc<dyn ContextHook>>,
    compaction_threshold: usize,
}

impl ContextEngine {
    pub fn new(compaction_threshold: usize) -> Self {
        Self {
            hooks: Vec::new(),
            compaction_threshold,
        }
    }

    pub fn add_hook(&mut self, hook: Arc<dyn ContextHook>) {
        tracing::debug!(hook = hook.name(), "context engine: registered hook");
        self.hooks.push(hook);
    }

    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Run the bootstrap phase: inject persistent context.
    pub async fn bootstrap(
        &self,
        messages: &mut Vec<ChatMessage>,
        agent_id: &str,
    ) -> anyhow::Result<()> {
        for hook in &self.hooks {
            if let Err(e) = hook.on_bootstrap(messages, agent_id).await {
                tracing::warn!(hook = hook.name(), error = %e, "bootstrap hook failed");
            }
        }
        Ok(())
    }

    /// Run the ingest phase: enrich with memory/RAG results.
    pub async fn ingest(
        &self,
        input: &IngestInput,
        messages: &mut Vec<ChatMessage>,
    ) -> anyhow::Result<()> {
        for hook in &self.hooks {
            if let Err(e) = hook.on_ingest(input, messages).await {
                tracing::warn!(hook = hook.name(), error = %e, "ingest hook failed");
            }
        }
        Ok(())
    }

    /// Run the assemble phase: finalize prompt structure.
    pub async fn assemble(&self, messages: &mut Vec<ChatMessage>) -> anyhow::Result<()> {
        for hook in &self.hooks {
            if let Err(e) = hook.on_assemble(messages).await {
                tracing::warn!(hook = hook.name(), error = %e, "assemble hook failed");
            }
        }
        Ok(())
    }

    /// Run compact if threshold exceeded, then assemble.
    pub async fn process(&self, messages: &mut Vec<ChatMessage>) -> anyhow::Result<()> {
        let non_system = messages
            .iter()
            .filter(|m| !matches!(m.role, fastclaw_core::types::Role::System))
            .count();

        if non_system > self.compaction_threshold {
            tracing::info!(
                count = non_system,
                threshold = self.compaction_threshold,
                "context engine: triggering compaction"
            );
            for hook in &self.hooks {
                if let Err(e) = hook.on_compact(messages).await {
                    tracing::warn!(hook = hook.name(), error = %e, "compact hook failed");
                }
            }
        }

        self.assemble(messages).await
    }

    /// Run after_turn for all hooks.
    pub async fn after_turn(
        &self,
        messages: &[ChatMessage],
        agent_id: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        for hook in &self.hooks {
            if let Err(e) = hook.on_after_turn(messages, agent_id, session_id).await {
                tracing::warn!(hook = hook.name(), error = %e, "after_turn hook failed");
            }
        }
        Ok(())
    }

    pub fn set_compaction_threshold(&mut self, threshold: usize) {
        self.compaction_threshold = threshold;
    }

    /// Assemble the six-layer context into API-ready messages (no hooks).
    pub fn assemble_context_layers(
        budget: &ContextBudget,
        layers: &ContextLayers,
    ) -> AssembledContext {
        assemble_context(budget, layers)
    }
}

// --- Default Hook Implementations ---

/// Injects a short system reminder after every N user messages (see
/// [`DEFAULT_SYSTEM_REMINDER_INTERVAL_USER_TURNS`]). Skips insertion when the reminder already
/// follows that user turn (e.g. loaded from a persisted session).
pub struct SystemReminderHook {
    pub every_n_user_turns: usize,
}

impl Default for SystemReminderHook {
    fn default() -> Self {
        Self {
            every_n_user_turns: DEFAULT_SYSTEM_REMINDER_INTERVAL_USER_TURNS,
        }
    }
}

impl SystemReminderHook {
    pub fn new(every_n_user_turns: usize) -> Self {
        Self {
            every_n_user_turns: every_n_user_turns.max(1),
        }
    }
}

#[async_trait]
impl ContextHook for SystemReminderHook {
    fn name(&self) -> &str {
        "system_reminder"
    }

    async fn on_assemble(&self, messages: &mut Vec<ChatMessage>) -> anyhow::Result<()> {
        let interval = self.every_n_user_turns;
        let user_positions: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.role, Role::User))
            .map(|(i, _)| i)
            .collect();
        if user_positions.is_empty() {
            return Ok(());
        }
        let marker_indices: Vec<usize> = (1..=user_positions.len() / interval)
            .map(|m| user_positions[m * interval - 1])
            .collect();
        for &user_msg_idx in marker_indices.iter().rev() {
            let insert_at = user_msg_idx + 1;
            if insert_at < messages.len() {
                if let Some(next) = messages.get(insert_at) {
                    if matches!(next.role, Role::System)
                        && next.text_content().as_deref() == Some(DEFAULT_SYSTEM_REMINDER_TEXT)
                    {
                        continue;
                    }
                }
            }
            messages.insert(
                insert_at,
                ChatMessage {
                    role: Role::System,
                    content: Some(DEFAULT_SYSTEM_REMINDER_TEXT.into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        Ok(())
    }
}

/// Default compaction hook that wraps the existing [`crate::compressor::ContextCompactor`].
///
/// For importance-based eviction of non-system messages, use
/// [`crate::compressor::CompactionStrategy::ImportanceBased`] (also the [`Default`] for
/// [`crate::compressor::CompactionStrategy`]).
pub struct CompactionHook {
    compactor: std::sync::Mutex<crate::compressor::ContextCompactor>,
}

impl CompactionHook {
    pub fn new(strategy: crate::compressor::CompactionStrategy) -> Self {
        Self {
            compactor: std::sync::Mutex::new(crate::compressor::ContextCompactor::new(strategy)),
        }
    }
}

#[async_trait]
impl ContextHook for CompactionHook {
    fn name(&self) -> &str {
        "compaction"
    }

    async fn on_compact(&self, messages: &mut Vec<ChatMessage>) -> anyhow::Result<()> {
        let guard = self
            .compactor
            .lock()
            .map_err(|e| anyhow::anyhow!("lock error: {e}"))?;
        let result = guard.compact(messages);
        *messages = result.messages;
        if let Some(ref summary) = result.summary {
            tracing::info!(
                evicted = result.evicted_count,
                kept = result.compacted_count,
                "compaction hook: summarized"
            );
            let _ = summary;
        }
        Ok(())
    }
}

/// Hook that injects SOUL.md / USER.md content as system messages during bootstrap.
pub struct PersonalityHook {
    soul_content: Option<String>,
    user_content: Option<String>,
}

impl PersonalityHook {
    pub fn new(soul_content: Option<String>, user_content: Option<String>) -> Self {
        Self {
            soul_content,
            user_content,
        }
    }

    pub fn from_workspace(workspace: &fastclaw_core::workspace::AgentWorkspace) -> Self {
        let soul_path = workspace
            .root
            .join(fastclaw_core::workspace::DEFAULT_SOUL_FILENAME);
        let soul = std::fs::read_to_string(&soul_path)
            .ok()
            .filter(|s| !s.trim().is_empty());

        let user_path = workspace
            .root
            .join(fastclaw_core::workspace::DEFAULT_USER_FILENAME);
        let user = std::fs::read_to_string(&user_path)
            .ok()
            .filter(|s| !s.trim().is_empty());

        Self::new(soul, user)
    }
}

#[async_trait]
impl ContextHook for PersonalityHook {
    fn name(&self) -> &str {
        "personality"
    }

    async fn on_bootstrap(
        &self,
        messages: &mut Vec<ChatMessage>,
        _agent_id: &str,
    ) -> anyhow::Result<()> {
        if let Some(ref soul) = self.soul_content {
            messages.insert(
                0,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(soul.clone().into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        if let Some(ref user) = self.user_content {
            let insert_at = if self.soul_content.is_some() { 1 } else { 0 };
            messages.insert(
                insert_at,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(format!("[User Profile]\n{user}").into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        Ok(())
    }
}

/// Hook that auto-injects relevant memory snippets during ingest.
pub struct MemoryIngestHook {
    episodic: Arc<fastclaw_memory::EpisodicMemory>,
    semantic: Arc<fastclaw_memory::SemanticMemory>,
    embedder: Option<Arc<dyn fastclaw_memory::EmbeddingProvider>>,
    max_snippets: usize,
}

impl MemoryIngestHook {
    pub fn new(
        episodic: Arc<fastclaw_memory::EpisodicMemory>,
        semantic: Arc<fastclaw_memory::SemanticMemory>,
        embedder: Option<Arc<dyn fastclaw_memory::EmbeddingProvider>>,
        max_snippets: usize,
    ) -> Self {
        Self {
            episodic,
            semantic,
            embedder,
            max_snippets,
        }
    }
}

#[async_trait]
impl ContextHook for MemoryIngestHook {
    fn name(&self) -> &str {
        "memory_ingest"
    }

    async fn on_ingest(
        &self,
        input: &IngestInput,
        messages: &mut Vec<ChatMessage>,
    ) -> anyhow::Result<()> {
        let last_user_msg = input
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, fastclaw_core::types::Role::User))
            .and_then(ChatMessage::text_content);

        let query = match last_user_msg.as_deref() {
            Some(q) if !q.is_empty() => q,
            _ => return Ok(()),
        };

        let query_vec = if let Some(ref ep) = self.embedder {
            ep.embed(query).await.ok()
        } else {
            None
        };

        let alpha = if query_vec.is_some() { 0.5 } else { 0.0 };

        let mut memory_parts = Vec::new();

        if let Ok(facts) = self
            .semantic
            .hybrid_search(query, query_vec.as_ref(), alpha, self.max_snippets)
            .await
        {
            for (fact, score) in facts {
                if score > 0.3 {
                    memory_parts.push(format!(
                        "- [fact] {}: {} {} (confidence: {:.1})",
                        fact.subject, fact.predicate, fact.object, fact.confidence
                    ));
                }
            }
        }

        if let Ok(episodes) = self
            .episodic
            .hybrid_search(query, query_vec.as_ref(), alpha, self.max_snippets / 2)
            .await
        {
            for (ep, score) in episodes {
                if score > 0.3 {
                    memory_parts.push(format!("- [memory] {}", ep.summary));
                }
            }
        }

        if !memory_parts.is_empty() {
            let memory_block = format!("[Relevant memories]\n{}", memory_parts.join("\n"));
            let insert_pos = messages
                .iter()
                .position(|m| !matches!(m.role, fastclaw_core::types::Role::System))
                .unwrap_or(messages.len());
            messages.insert(
                insert_pos,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(memory_block.into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }

        Ok(())
    }
}

/// Multi-agent personality hook that stores SOUL/USER content per agent.
pub struct AgentPersonalityHook {
    workspaces: std::collections::HashMap<String, std::path::PathBuf>,
}

impl AgentPersonalityHook {
    pub fn new() -> Self {
        Self {
            workspaces: std::collections::HashMap::new(),
        }
    }

    pub fn add_agent(
        &mut self,
        agent_id: &str,
        workspace: &fastclaw_core::workspace::AgentWorkspace,
    ) {
        self.workspaces
            .insert(agent_id.to_string(), workspace.root.clone());
    }

    fn read_file(root: &std::path::Path, filename: &str) -> Option<String> {
        std::fs::read_to_string(root.join(filename))
            .ok()
            .filter(|s| !s.trim().is_empty())
    }
}

#[async_trait]
impl ContextHook for AgentPersonalityHook {
    fn name(&self) -> &str {
        "agent_personality"
    }

    async fn on_bootstrap(
        &self,
        messages: &mut Vec<ChatMessage>,
        agent_id: &str,
    ) -> anyhow::Result<()> {
        let Some(root) = self.workspaces.get(agent_id) else {
            return Ok(());
        };
        let soul = Self::read_file(root, fastclaw_core::workspace::DEFAULT_SOUL_FILENAME);
        let user = Self::read_file(root, fastclaw_core::workspace::DEFAULT_USER_FILENAME);
        let agents = Self::read_file(root, fastclaw_core::workspace::DEFAULT_AGENTS_FILENAME);

        if let Some(ref soul_content) = soul {
            messages.insert(
                0,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(soul_content.clone().into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        let mut pos = if soul.is_some() { 1 } else { 0 };
        if let Some(ref agents_content) = agents {
            messages.insert(
                pos,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(format!("[Operating Rules]\n{agents_content}").into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
            pos += 1;
        }
        if let Some(ref user_content) = user {
            messages.insert(
                pos,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(format!("[User Profile]\n{user_content}").into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        Ok(())
    }
}

/// Multi-agent memory ingest hook.
pub struct AgentMemoryIngestHook {
    episodic_map: std::collections::HashMap<String, Arc<fastclaw_memory::EpisodicMemory>>,
    semantic_map: std::collections::HashMap<String, Arc<fastclaw_memory::SemanticMemory>>,
    embedder: Option<Arc<dyn fastclaw_memory::EmbeddingProvider>>,
    max_snippets: usize,
}

impl AgentMemoryIngestHook {
    pub fn new(
        episodic_map: std::collections::HashMap<String, Arc<fastclaw_memory::EpisodicMemory>>,
        semantic_map: std::collections::HashMap<String, Arc<fastclaw_memory::SemanticMemory>>,
        embedder: Option<Arc<dyn fastclaw_memory::EmbeddingProvider>>,
        max_snippets: usize,
    ) -> Self {
        Self {
            episodic_map,
            semantic_map,
            embedder,
            max_snippets,
        }
    }
}

#[async_trait]
impl ContextHook for AgentMemoryIngestHook {
    fn name(&self) -> &str {
        "agent_memory_ingest"
    }

    async fn on_ingest(
        &self,
        input: &IngestInput,
        messages: &mut Vec<ChatMessage>,
    ) -> anyhow::Result<()> {
        let Some(semantic) = self.semantic_map.get(&input.agent_id) else {
            return Ok(());
        };
        let episodic = self.episodic_map.get(&input.agent_id);

        let last_user_msg = input
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, fastclaw_core::types::Role::User))
            .and_then(ChatMessage::text_content);
        let query = match last_user_msg.as_deref() {
            Some(q) if !q.is_empty() => q,
            _ => return Ok(()),
        };
        let query_vec = if let Some(ref ep) = self.embedder {
            ep.embed(query).await.ok()
        } else {
            None
        };
        let alpha = if query_vec.is_some() { 0.5 } else { 0.0 };
        let mut parts = Vec::new();

        if let Ok(facts) = semantic
            .hybrid_search(query, query_vec.as_ref(), alpha, self.max_snippets)
            .await
        {
            for (fact, score) in facts {
                if score > 0.3 {
                    parts.push(format!(
                        "- [fact] {}: {} {} (confidence: {:.1})",
                        fact.subject, fact.predicate, fact.object, fact.confidence
                    ));
                }
            }
        }
        if let Some(ep) = episodic {
            if let Ok(episodes) = ep
                .hybrid_search(query, query_vec.as_ref(), alpha, self.max_snippets / 2)
                .await
            {
                for (episode, score) in episodes {
                    if score > 0.3 {
                        parts.push(format!("- [memory] {}", episode.summary));
                    }
                }
            }
        }
        if !parts.is_empty() {
            let block = format!("[Relevant memories]\n{}", parts.join("\n"));
            let pos = messages
                .iter()
                .position(|m| !matches!(m.role, fastclaw_core::types::Role::System))
                .unwrap_or(messages.len());
            messages.insert(
                pos,
                ChatMessage {
                    role: fastclaw_core::types::Role::System,
                    content: Some(block.into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        Ok(())
    }
}

/// Default output token reservation: `min(max_tokens, context_window / 4)`.
fn default_reserved_output(context_window: u32, max_tokens: Option<u32>) -> u32 {
    let quarter = context_window / 4;
    match max_tokens {
        Some(mt) => mt.min(quarter),
        None => quarter,
    }
}

impl ContextEngine {
    /// Trim `messages` so total estimated tokens fit within `context_window - reserved_output`.
    ///
    /// 1. If already within budget, returns the estimated token count without modification.
    /// 2. Applies `ImportanceBased` compaction first (preserves recent + tool results).
    /// 3. Falls back to sliding-window truncation (drop oldest non-system messages) if still over.
    /// 4. Never drops system messages or the current user turn (last user message).
    pub fn fit_to_context_window(
        messages: &mut Vec<ChatMessage>,
        context_window: u32,
        max_tokens: Option<u32>,
    ) -> usize {
        let reserved = default_reserved_output(context_window, max_tokens);
        let budget = (context_window.saturating_sub(reserved)) as usize;

        let estimated = crate::compressor::estimate_messages_tokens(messages);
        if estimated <= budget {
            return estimated;
        }

        tracing::info!(
            estimated,
            budget,
            context_window,
            reserved,
            "context window exceeded — applying compaction"
        );

        // Phase 1: ImportanceBased compaction
        let compactor = crate::compressor::ContextCompactor::new(
            crate::compressor::CompactionStrategy::ImportanceBased {
                max_messages: crate::compressor::DEFAULT_IMPORTANCE_MAX_MESSAGES,
                recent_window: crate::compressor::DEFAULT_IMPORTANCE_RECENT_WINDOW,
            },
        );
        let result = compactor.compact(messages);
        *messages = result.messages;

        let estimated = crate::compressor::estimate_messages_tokens(messages);
        if estimated <= budget {
            return estimated;
        }

        // Phase 2: TokenBudget compaction (drop oldest conversational messages)
        let compactor = crate::compressor::ContextCompactor::new(
            crate::compressor::CompactionStrategy::TokenBudget { max_tokens: budget },
        );
        let result = compactor.compact(messages);
        *messages = result.messages;

        let estimated = crate::compressor::estimate_messages_tokens(messages);
        if estimated <= budget {
            return estimated;
        }

        // Phase 3: Hard sliding-window truncation — keep system msgs + last N non-system
        let (system_msgs, conversation): (Vec<_>, Vec<_>) = messages
            .iter()
            .partition(|m| matches!(m.role, Role::System));

        let mut kept = Vec::new();
        let mut used = 0usize;
        let system_tokens: usize = system_msgs
            .iter()
            .map(|m| crate::compressor::estimate_messages_tokens(std::slice::from_ref(*m)))
            .sum();
        let remaining = budget.saturating_sub(system_tokens);

        for msg in conversation.iter().rev() {
            let cost = crate::compressor::estimate_messages_tokens(std::slice::from_ref(*msg));
            if used + cost > remaining && !kept.is_empty() {
                break;
            }
            kept.push((*msg).clone());
            used += cost;
        }
        kept.reverse();

        let mut final_msgs: Vec<ChatMessage> = system_msgs.into_iter().cloned().collect();
        if !conversation.is_empty() && kept.len() < conversation.len() {
            final_msgs.push(ChatMessage {
                role: Role::System,
                content: Some("[Earlier conversation history was truncated to fit context window]".into()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        final_msgs.extend(kept);
        *messages = final_msgs;

        crate::compressor::estimate_messages_tokens(messages)
    }
}

/// Build a ContextEngine with sensible defaults.
pub fn build_default_engine(
    strategy: crate::compressor::CompactionStrategy,
    threshold: usize,
) -> ContextEngine {
    let mut engine = ContextEngine::new(threshold);
    engine.add_hook(Arc::new(CompactionHook::new(strategy)));
    engine
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::Role;

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(text.to_string().into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(text.to_string().into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    struct TestHook {
        name: String,
        inject_text: Option<String>,
    }

    impl TestHook {
        fn with_inject(name: &str, text: &str) -> Self {
            Self {
                name: name.to_string(),
                inject_text: Some(text.to_string()),
            }
        }
    }

    #[async_trait]
    impl ContextHook for TestHook {
        fn name(&self) -> &str {
            &self.name
        }

        async fn on_bootstrap(
            &self,
            messages: &mut Vec<ChatMessage>,
            _agent_id: &str,
        ) -> anyhow::Result<()> {
            if let Some(ref text) = self.inject_text {
                messages.insert(
                    0,
                    ChatMessage {
                        role: Role::System,
                        content: Some(text.clone().into()),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                );
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn engine_runs_hooks_in_order() {
        let mut engine = ContextEngine::new(50);
        engine.add_hook(Arc::new(TestHook::with_inject("first", "FIRST")));
        engine.add_hook(Arc::new(TestHook::with_inject("second", "SECOND")));

        let mut msgs = vec![user("hello")];
        engine.bootstrap(&mut msgs, "test").await.unwrap();

        assert_eq!(msgs.len(), 3);
        // Both hooks insert at position 0, so second hook's content ends up first
        assert_eq!(msgs[0].text_content().as_deref(), Some("SECOND"));
        assert_eq!(msgs[1].text_content().as_deref(), Some("FIRST"));
    }

    #[tokio::test]
    async fn engine_compacts_over_threshold() {
        let strategy = crate::compressor::CompactionStrategy::SlidingWindow { keep_recent: 2 };
        let engine = build_default_engine(strategy, 4);

        let mut msgs: Vec<ChatMessage> = (0..6)
            .flat_map(|i| vec![user(&format!("q{i}")), assistant(&format!("a{i}"))])
            .collect();

        engine.process(&mut msgs).await.unwrap();
        assert!(msgs.len() < 12, "messages should be compacted");
    }

    #[tokio::test]
    async fn engine_no_compact_under_threshold() {
        let engine = build_default_engine(
            crate::compressor::CompactionStrategy::SlidingWindow { keep_recent: 10 },
            100,
        );

        let mut msgs = vec![user("hi"), assistant("hello")];
        let original_len = msgs.len();
        engine.process(&mut msgs).await.unwrap();
        assert_eq!(msgs.len(), original_len);
    }

    #[tokio::test]
    async fn system_reminder_injected_every_n_user_turns() {
        let mut engine = ContextEngine::new(10_000);
        engine.add_hook(Arc::new(SystemReminderHook::new(3)));
        let mut msgs: Vec<ChatMessage> = (0..9)
            .map(|i| user(&format!("u{i}")))
            .collect();
        engine.process(&mut msgs).await.unwrap();
        let reminders = msgs
            .iter()
            .filter(|m| {
                matches!(m.role, Role::System)
                    && m.text_content().as_deref() == Some(DEFAULT_SYSTEM_REMINDER_TEXT)
            })
            .count();
        assert_eq!(reminders, 3);
    }

    #[test]
    fn assemble_context_respects_order() {
        let budget = ContextBudget {
            total_tokens: 2048,
            ..ContextBudget::default()
        };
        let layers = ContextLayers {
            system_prompt: "SYS".into(),
            profile_text: "PROFILE".into(),
            session_summary: "SUM".into(),
            recall_text: "REC".into(),
            recent_messages: vec![assistant("prev")],
            current_input: Some(user("now")),
        };
        let out = assemble_context(&budget, &layers);
        assert!(matches!(out.messages[0].role, Role::System));
        assert!(out
            .messages
            .iter()
            .any(|m| m.text_content().as_deref() == Some("now")));
    }
}
