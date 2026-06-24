use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use xiaolin_core::tool::{
    Tool, ToolKind, ToolParameterSchema, ToolResult, format_soft_failure_error,
    no_retry_recovery_hint,
};

/// Whether a memory search error looks like a backend/I/O failure rather than a query issue.
fn is_memory_backend_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("database")
        || lower.contains("sqlite")
        || lower.contains("connection")
        || lower.contains("i/o")
        || lower.contains("io error")
        || lower.contains("unavailable")
        || lower.contains("disk")
        || lower.contains("locked")
        || lower.contains("corrupt")
}

fn facts_search_error_field(err: impl std::fmt::Display) -> String {
    let detail = err.to_string();
    let hint = if is_memory_backend_error(&detail) {
        no_retry_recovery_hint(
            "Try scope 'episodes' if you only need session recaps; report persistent backend errors to the operator.",
        )
    } else {
        "Retry with a shorter, more concrete query; try scope 'episodes' if you only need session recaps."
            .to_string()
    };
    format_soft_failure_error(format!("Semantic memory search failed: {detail}"), hint)
}

fn episodes_search_error_field(err: impl std::fmt::Display) -> String {
    let detail = err.to_string();
    let hint = if is_memory_backend_error(&detail) {
        no_retry_recovery_hint(
            "Try scope 'facts' if semantic triples are enough; report persistent backend errors to the operator.",
        )
    } else {
        "Retry with different keywords, a lower limit, or scope 'facts' if semantic triples are enough."
            .to_string()
    };
    format_soft_failure_error(format!("Episodic memory search failed: {detail}"), hint)
}

// ---------- Memory Tools ----------

/// Search agent memory (both episodic and semantic) using hybrid keyword + vector search.
pub struct MemorySearchTool {
    episodic: Arc<xiaolin_memory::EpisodicMemory>,
    semantic: Arc<xiaolin_memory::SemanticMemory>,
    embedder: Option<Arc<dyn xiaolin_memory::EmbeddingProvider>>,
}

impl MemorySearchTool {
    pub fn new(
        episodic: Arc<xiaolin_memory::EpisodicMemory>,
        semantic: Arc<xiaolin_memory::SemanticMemory>,
        embedder: Option<Arc<dyn xiaolin_memory::EmbeddingProvider>>,
    ) -> Self {
        Self {
            episodic,
            semantic,
            embedder,
        }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search durable agent memory: semantic facts (subject–predicate–object triples) and episodic entries (short summaries of past work). \
         With embeddings configured, results mix lexical and vector scores; without embeddings, recall depends on keyword overlap—use concrete nouns, product names, and dates instead of vague pronouns. \
         Use memory_search for user-stated preferences, prior decisions, named services, or \"remember this\" items—not for guessing current repo state or the live web. Always reconcile important claims with read_file, list_directory, or rg; refresh external facts via web_search + web_fetch. \
         scope selects facts, episodes, or both (default all); limit bounds rows per side—start around 5–10 and increase only if recall is sparse. \
         If nothing relevant returns, say so and ask the user; do not invent memory. \
         Anti-pattern: trusting memory over git for file contents. \
         Example: {\"query\": \"Postgres HA decision user approved\", \"scope\": \"all\", \"limit\": 8}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Natural-language question or compact keywords. Examples: 'Postgres HA decision April 2026', 'user prefers dark UI and fish shell'. Use concrete nouns; vague pronouns ('it', 'that bug') retrieve poorly without entity names."
            }),
        );
        props.insert(
            "scope".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["all", "facts", "episodes"],
                "description": "all (default): merge facts and episodes; facts: semantic triple hits only; episodes: episodic summaries only. Use facts for stable preferences/rules; use episodes when you need narrative 'what happened' recall."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Integer maximum hits per scope side (default 10; typical 3–20). Non-integer JSON values fall back to defaults—pass a bare number. Lower to keep responses small; raise only when recall is thin."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "memory_search arguments are not valid JSON: {e}. \
                 Pass {{\"query\": \"...\", \"scope\": \"all\", \"limit\": 10}}; only query is required."
            )),
        };

        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => return ToolResult::err(
                "memory_search is missing required string field 'query'. \
                 Example: {\"query\": \"preferred cloud region\", \"scope\": \"facts\", \"limit\": 5}."
                    .to_string(),
            ),
        };

        let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("all");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let query_vec = if let Some(ref ep) = self.embedder {
            ep.embed(query).await.ok()
        } else {
            None
        };

        let alpha = if query_vec.is_some() { 0.5 } else { 0.0 };
        let mut result = serde_json::json!({});

        if scope == "all" || scope == "facts" {
            let facts = self
                .semantic
                .hybrid_search(query, query_vec.as_ref(), alpha, limit)
                .await;
            match facts {
                Ok(hits) => {
                    let items: Vec<serde_json::Value> = hits
                        .into_iter()
                        .map(|(f, score)| {
                            serde_json::json!({
                                "id": f.id,
                                "category": f.category,
                                "subject": f.subject,
                                "predicate": f.predicate,
                                "object": f.object,
                                "confidence": f.confidence,
                                "score": (score * 1000.0).round() / 1000.0,
                            })
                        })
                        .collect();
                    result["facts"] = serde_json::json!(items);
                }
                Err(e) => {
                    result["facts_error"] = serde_json::json!(facts_search_error_field(e));
                }
            }
        }

        if scope == "all" || scope == "episodes" {
            let episodes = self
                .episodic
                .hybrid_search(query, query_vec.as_ref(), alpha, limit)
                .await;
            match episodes {
                Ok(hits) => {
                    let items: Vec<serde_json::Value> = hits
                        .into_iter()
                        .map(|(ep, score)| {
                            serde_json::json!({
                                "id": ep.id,
                                "summary": ep.summary,
                                "importance": ep.importance,
                                "tags": ep.tags,
                                "created_at": ep.created_at,
                                "score": (score * 1000.0).round() / 1000.0,
                            })
                        })
                        .collect();
                    result["episodes"] = serde_json::json!(items);
                }
                Err(e) => {
                    result["episodes_error"] =
                        serde_json::json!(episodes_search_error_field(e));
                }
            }
        }

        ToolResult::ok(serde_json::to_string(&result).unwrap_or_default())
    }
}

// ─── Unified Memory Tool ──────────────────────────────────────────────

pub struct UnifiedMemoryTool {
    search: MemorySearchTool,
    store: MemoryStoreTool,
}

impl UnifiedMemoryTool {
    pub fn new(
        episodic: Arc<xiaolin_memory::EpisodicMemory>,
        semantic: Arc<xiaolin_memory::SemanticMemory>,
        embedder: Option<Arc<dyn xiaolin_memory::EmbeddingProvider>>,
        agent_id: String,
    ) -> Self {
        Self {
            search: MemorySearchTool::new(episodic.clone(), semantic.clone(), embedder.clone()),
            store: MemoryStoreTool::new(episodic, semantic, embedder, agent_id),
        }
    }
}

#[async_trait]
impl Tool for UnifiedMemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Agent long-term memory: search or store facts and episodes. \
         action 'search': find stored knowledge (requires query). \
         action 'store': persist a fact (subject/predicate/object) or episode (summary). \
         Use search for user preferences, prior decisions, project rules. \
         Use store for durable knowledge worth recalling across sessions. \
         LONG TASK PROTOCOL: for tasks spanning 10+ turns, store a progress checkpoint every 5 turns \
         (decisions made, files changed, current status, next steps). Before resuming after compression, \
         search memory first to rebuild context. \
         Never store secrets (passwords, API keys, tokens)."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["search", "store"],
                "description": "search: query memory; store: persist new knowledge."
            }),
        );
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "For search: natural-language query or keywords."
            }),
        );
        props.insert(
            "scope".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["all", "facts", "episodes"],
                "description": "For search: which memory to query (default all)."
            }),
        );
        props.insert(
            "limit".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "For search: max results per scope (default 10)."
            }),
        );
        props.insert(
            "type".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["fact", "episode"],
                "description": "For store: what kind of memory entry."
            }),
        );
        props.insert(
            "subject".to_string(),
            serde_json::json!({"type": "string", "description": "For store fact: subject."}),
        );
        props.insert(
            "predicate".to_string(),
            serde_json::json!({"type": "string", "description": "For store fact: relation."}),
        );
        props.insert(
            "object".to_string(),
            serde_json::json!({"type": "string", "description": "For store fact: value."}),
        );
        props.insert(
            "category".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["user_preference", "user_fact", "domain_knowledge", "correction"],
                "description": "For store fact: category."
            }),
        );
        props.insert(
            "summary".to_string(),
            serde_json::json!({"type": "string", "description": "For store episode: recap."}),
        );
        props.insert(
            "importance".to_string(),
            serde_json::json!({"type": "number", "description": "For store episode: 0.0-1.0."}),
        );
        props.insert("tags".to_string(), serde_json::json!({"type": "string", "description": "For store episode: comma-separated."}));
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("memory: invalid JSON: {e}")),
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::err(
                    "memory requires 'action': 'search' or 'store'.".to_string(),
                )
            }
        };

        match action {
            "search" => self.search.execute(arguments).await,
            "store" => self.store.execute(arguments).await,
            other => ToolResult::err(format!(
                "memory: unknown action '{other}'. Use 'search' or 'store'."
            )),
        }
    }
}

/// Store a fact or episode into agent memory.
pub struct MemoryStoreTool {
    episodic: Arc<xiaolin_memory::EpisodicMemory>,
    semantic: Arc<xiaolin_memory::SemanticMemory>,
    embedder: Option<Arc<dyn xiaolin_memory::EmbeddingProvider>>,
    agent_id: String,
}

impl MemoryStoreTool {
    pub fn new(
        episodic: Arc<xiaolin_memory::EpisodicMemory>,
        semantic: Arc<xiaolin_memory::SemanticMemory>,
        embedder: Option<Arc<dyn xiaolin_memory::EmbeddingProvider>>,
        agent_id: String,
    ) -> Self {
        Self {
            episodic,
            semantic,
            embedder,
            agent_id,
        }
    }
}

#[async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Write durable knowledge to long-term memory as either a structured fact (subject–predicate–object) or an episodic summary (what happened in a session). \
         Facts belong to stable preferences, architecture constraints, naming rules, and explicit user corrections; episodes capture decisions, blockers, and resolutions so memory_search can surface narrative context later. \
         Never store secrets—no passwords, API keys, tokens, or raw cookies; summarize safely and keep credentials in proper secret stores. \
         Duplicate-ish facts may upsert—still prefer one clear triple over many noisy variants. \
         Anti-pattern: dumping whole chat logs; anti-pattern: mirroring large source files that belong in git. \
         Fact example: {\"type\": \"fact\", \"subject\": \"ci\", \"predicate\": \"requires_green_tests\", \"object\": \"true\", \"category\": \"domain_knowledge\"}. Episode example: {\"type\": \"episode\", \"summary\": \"User chose serde_json over simd-json for readability.\", \"importance\": 0.6, \"tags\": \"rust,deps\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert("type".to_string(), serde_json::json!({
            "type": "string",
            "enum": ["fact", "episode"],
            "description": "fact: requires subject, predicate, object (optional category). episode: requires summary (optional tags string, optional importance 0.0–1.0). Any other type string is rejected with guidance."
        }));
        props.insert(
            "subject".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Fact subject node—who/what the statement is about. Examples: 'user', 'deploy_pipeline', 'repo_xiaolin'."
            }),
        );
        props.insert(
            "predicate".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Relation between subject and object—short snake_case verb phrase. Examples: 'default_branch', 'prefers_shell', 'blocked_by'."
            }),
        );
        props.insert(
            "object".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Literal value tied to subject+predicate. Examples: 'main', 'eu-west-1', 'must_use_sandboxed_shell'."
            }),
        );
        props.insert(
            "category".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["user_preference", "user_fact", "domain_knowledge", "correction"],
                "description": "user_preference: tastes; user_fact: stable bio; domain_knowledge: project rules (default); correction: explicit fixes. Prefer these enum strings—other values may be stored but filters may ignore them."
            }),
        );
        props.insert(
            "summary".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "One-paragraph episode recap, e.g. 'User chose Postgres over SQLite for HA requirement'."
            }),
        );
        props.insert(
            "importance".to_string(),
            serde_json::json!({
                "type": "number",
                "description": "Episode salience 0.0–1.0 (default 0.5); raise for milestones, lower for noise."
            }),
        );
        props.insert(
            "tags".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Comma-separated labels for episodes, e.g. 'infra,decision,postgres'."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["type".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "memory_store arguments are not valid JSON: {e}. \
                 Pass {{\"type\": \"fact\", \"subject\": \"...\", \"predicate\": \"...\", \"object\": \"...\"}} or {{\"type\": \"episode\", \"summary\": \"...\"}}."
            )),
        };

        let entry_type = match args.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return ToolResult::err(
                "memory_store is missing required string field 'type'. \
                 Use {\"type\": \"fact\", ...} with subject/predicate/object, or {\"type\": \"episode\", ...} with summary."
                    .to_string(),
            ),
        };

        let embedder = self.embedder.as_deref();

        match entry_type {
            "fact" => {
                let subject = match args.get("subject").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => return ToolResult::err(
                        "memory_store fact is missing string field 'subject'. \
                         Provide subject, predicate, and object together, e.g. {\"type\":\"fact\",\"subject\":\"user\",\"predicate\":\"prefers_shell\",\"object\":\"fish\"}."
                            .to_string(),
                    ),
                };
                let predicate = match args.get("predicate").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => return ToolResult::err(
                        "memory_store fact is missing string field 'predicate'. \
                         Use a short verb phrase describing the relation, e.g. 'default_branch_is' or 'avoids_tool'."
                            .to_string(),
                    ),
                };
                let object = match args.get("object").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => return ToolResult::err(
                        "memory_store fact is missing string field 'object'. \
                         State the literal value bound to subject+predicate, e.g. 'main' or 'sandboxed_shell_only'."
                            .to_string(),
                    ),
                };
                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("domain_knowledge");

                let id = format!(
                    "{}_{}",
                    subject.replace(' ', "_"),
                    uuid::Uuid::new_v4()
                        .to_string()
                        .split('-')
                        .next()
                        .unwrap_or("x")
                );
                let now = chrono::Utc::now().to_rfc3339();
                let fact = xiaolin_memory::Fact {
                    id: id.clone(),
                    category: category.to_string(),
                    subject: subject.to_string(),
                    predicate: predicate.to_string(),
                    object: object.to_string(),
                    confidence: 1.0,
                    source_session: None,
                    created_at: now.clone(),
                    updated_at: now,
                };

                match self.semantic.upsert_auto(&fact, embedder).await {
                    Ok(()) => ToolResult::ok(
                        serde_json::json!({
                            "status": "stored",
                            "type": "fact",
                            "id": id,
                        })
                        .to_string(),
                    ),
                    Err(e) => ToolResult::err(format!(
                        "memory_store could not persist the fact to semantic memory: {e}. \
                         Retry with shorter strings; if the error looks like I/O or database connectivity, stop looping and report the backend issue to the operator."
                    )),
                }
            }
            "episode" => {
                let summary = match args.get("summary").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => return ToolResult::err(
                        "memory_store episode is missing string field 'summary'. \
                         Add one or two sentences capturing what future-you must recall, e.g. 'Chose Postgres because HA requirement; SQLite ruled out'."
                            .to_string(),
                    ),
                };
                let importance = args
                    .get("importance")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5) as f32;
                let tags = args.get("tags").and_then(|v| v.as_str()).unwrap_or("");

                let id = format!("ep_{}", uuid::Uuid::new_v4());
                let episode = xiaolin_memory::Episode {
                    id: id.clone(),
                    session_id: "tool".to_string(),
                    agent_id: self.agent_id.clone(),
                    summary: summary.to_string(),
                    importance,
                    tags: tags.to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dreamed_at: None,
                };

                match self.episodic.record_auto(&episode, embedder).await {
                    Ok(()) => ToolResult::ok(
                        serde_json::json!({
                            "status": "stored",
                            "type": "episode",
                            "id": id,
                        })
                        .to_string(),
                    ),
                    Err(e) => ToolResult::err(format!(
                        "memory_store could not persist the episode to episodic memory: {e}. \
                         Shorten summary/tags and retry once; if it persists, the episodic backend may be unavailable—surface the error instead of spamming writes."
                    )),
                }
            }
            other => ToolResult::err(format!(
                "memory_store received unknown type '{other}'. \
                 Use exactly the string 'fact' (requires subject, predicate, object) or 'episode' (requires summary), then retry."
            )),
        }
    }
}

#[cfg(test)]
mod memory_search_error_tests {
    use super::{episodes_search_error_field, facts_search_error_field, is_memory_backend_error};

    #[test]
    fn backend_error_includes_stop_retrying() {
        let out = facts_search_error_field("database connection refused");
        assert!(out.contains("What to do next:"));
        assert!(out.contains("Stop retrying"));
        assert!(is_memory_backend_error("database connection refused"));
    }

    #[test]
    fn query_error_omits_stop_retrying() {
        let out = facts_search_error_field("query tokenization failed");
        assert!(out.contains("What to do next:"));
        assert!(!out.contains("Stop retrying"));
        assert!(!is_memory_backend_error("query tokenization failed"));
    }

    #[test]
    fn episodes_backend_error_includes_recovery_guidance() {
        let out = episodes_search_error_field("sqlite disk I/O error");
        assert!(out.contains("Episodic memory search failed"));
        assert!(out.contains("What to do next:"));
        assert!(out.contains("Stop retrying"));
    }
}
