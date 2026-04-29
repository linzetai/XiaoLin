use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use fastclaw_core::agent_config::AgentConfig;
use fastclaw_core::types::ExecutionMode;

/// Runtime context provided to each section's compute function.
pub struct PromptContext {
    pub agent_config: Arc<AgentConfig>,
    pub enabled_tools: HashSet<String>,
    pub deferred_tool_count: usize,
    pub model_id: String,
    pub cwd: PathBuf,
    pub is_git: bool,
    pub platform: String,
    pub shell: String,
    pub execution_mode: ExecutionMode,
    pub mcp_servers: Vec<McpServerInfo>,
    pub language_preference: Option<String>,
    pub token_budget: Option<usize>,
    pub memory_prompt: Option<String>,
    pub session_start_date: String,
}

/// Minimal info about a connected MCP server, used by prompt sections.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub id: String,
    pub instructions: Option<String>,
}

/// Compute function type for prompt sections.
pub type SectionCompute = Box<dyn Fn(&PromptContext) -> Option<String> + Send + Sync>;

/// A lazily-computed, optionally cached system prompt fragment.
///
/// Static sections (cache_break=false) are memoized after first computation
/// and reused until `clear_cache()` is called.
/// Dynamic sections (cache_break=true) are recomputed every invocation.
pub struct PromptSection {
    pub name: &'static str,
    pub compute: SectionCompute,
    pub cache_break: bool,
}

const DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";

/// Layered, cacheable prompt engine that assembles system prompt from sections.
///
/// Sections are split into static (cross-session cacheable) and dynamic
/// (per-turn / per-session). A boundary marker separates them for API prompt
/// cache alignment.
pub struct PromptEngine {
    static_sections: Vec<PromptSection>,
    dynamic_sections: Vec<PromptSection>,
    section_cache: DashMap<String, Option<String>>,
}

impl PromptEngine {
    pub fn new(
        static_sections: Vec<PromptSection>,
        dynamic_sections: Vec<PromptSection>,
    ) -> Self {
        Self {
            static_sections,
            dynamic_sections,
            section_cache: DashMap::new(),
        }
    }

    /// Build the full system prompt from all registered sections.
    ///
    /// Returns `Vec<String>` where each element is an independent section,
    /// suitable for API prompt cache segmentation.
    ///
    /// Order: static sections → dynamic boundary → dynamic sections.
    pub fn build_system_prompt(&self, ctx: &PromptContext) -> Vec<String> {
        let mut parts = Vec::new();

        for section in &self.static_sections {
            if let Some(v) = self.resolve_section(section, ctx) {
                parts.push(v);
            }
        }

        parts.push(DYNAMIC_BOUNDARY.into());

        for section in &self.dynamic_sections {
            let value = if section.cache_break {
                (section.compute)(ctx)
            } else {
                self.resolve_section(section, ctx)
            };
            if let Some(v) = value {
                parts.push(v);
            }
        }

        parts
    }

    /// Build the effective prompt with priority layering.
    ///
    /// Resolution order:
    /// 1. `override_prompt` — if set, used as-is (single element)
    /// 2. `agent_prompt` — agent-level system_prompt from config
    /// 3. `custom_prompt` — user-provided custom prompt
    /// 4. Default: `build_system_prompt(ctx)` with optional `append_prompt`
    pub fn build_effective_prompt(
        &self,
        ctx: &PromptContext,
        override_prompt: Option<&str>,
        agent_prompt: Option<&str>,
        custom_prompt: Option<&str>,
        append_prompt: Option<&str>,
    ) -> Vec<String> {
        if let Some(ovr) = override_prompt {
            return vec![ovr.to_string()];
        }

        let base = if let Some(ap) = agent_prompt {
            vec![ap.to_string()]
        } else if let Some(cp) = custom_prompt {
            vec![cp.to_string()]
        } else {
            self.build_system_prompt(ctx)
        };

        match append_prompt {
            Some(ap) => {
                let mut result = base;
                result.push(ap.to_string());
                result
            }
            None => base,
        }
    }

    /// Clear all cached section values.
    ///
    /// Called on `/clear`, `/compact`, mode switch, or session reset.
    pub fn clear_cache(&self) {
        self.section_cache.clear();
    }

    fn resolve_section(&self, section: &PromptSection, ctx: &PromptContext) -> Option<String> {
        if let Some(cached) = self.section_cache.get(section.name) {
            return cached.clone();
        }
        let value = (section.compute)(ctx);
        self.section_cache
            .insert(section.name.to_string(), value.clone());
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_ctx() -> PromptContext {
        PromptContext {
            agent_config: Arc::new(AgentConfig {
                agent_id: "test".into(),
                name: None,
                description: None,
                model: Default::default(),
                system_prompt: None,
                tools: vec![],
                behavior: Default::default(),
                mcp_servers: vec![],
                min_tier: None,
                max_tier: None,
                avatar: None,
                channels: Default::default(),
            }),
            enabled_tools: HashSet::new(),
            deferred_tool_count: 0,
            model_id: "test-model".into(),
            cwd: PathBuf::from("/tmp"),
            is_git: false,
            platform: "linux".into(),
            shell: "bash".into(),
            execution_mode: ExecutionMode::Agent,
            mcp_servers: vec![],
            language_preference: None,
            token_budget: None,
            memory_prompt: None,
            session_start_date: "2026-04-29".into(),
        }
    }


    #[test]
    fn build_system_prompt_order() {
        let engine = PromptEngine::new(
            vec![
                PromptSection {
                    name: "intro",
                    compute: Box::new(|_| Some("INTRO".into())),
                    cache_break: false,
                },
                PromptSection {
                    name: "system",
                    compute: Box::new(|_| Some("SYSTEM".into())),
                    cache_break: false,
                },
            ],
            vec![PromptSection {
                name: "env",
                compute: Box::new(|_| Some("ENV".into())),
                cache_break: true,
            }],
        );
        let ctx = make_ctx();
        let parts = engine.build_system_prompt(&ctx);

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "INTRO");
        assert_eq!(parts[1], "SYSTEM");
        assert_eq!(parts[2], DYNAMIC_BOUNDARY);
        assert_eq!(parts[3], "ENV");
    }

    #[test]
    fn none_sections_are_skipped() {
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "maybe",
                compute: Box::new(|_| None),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();
        let parts = engine.build_system_prompt(&ctx);

        assert_eq!(parts, vec![DYNAMIC_BOUNDARY]);
    }

    #[test]
    fn cache_hit_prevents_recomputation() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "counted",
                compute: Box::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                    Some("VALUE".into())
                }),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();

        engine.build_system_prompt(&ctx);
        engine.build_system_prompt(&ctx);
        engine.build_system_prompt(&ctx);

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cache_break_forces_recomputation() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let engine = PromptEngine::new(
            vec![],
            vec![PromptSection {
                name: "dynamic",
                compute: Box::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                    Some("DYN".into())
                }),
                cache_break: true,
            }],
        );
        let ctx = make_ctx();

        engine.build_system_prompt(&ctx);
        engine.build_system_prompt(&ctx);
        engine.build_system_prompt(&ctx);

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn clear_cache_resets_all() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "cached",
                compute: Box::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                    Some("V".into())
                }),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();

        engine.build_system_prompt(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        engine.clear_cache();
        engine.build_system_prompt(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn effective_prompt_override_wins() {
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "default",
                compute: Box::new(|_| Some("DEFAULT".into())),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();

        let result =
            engine.build_effective_prompt(&ctx, Some("OVERRIDE"), Some("AGENT"), None, None);
        assert_eq!(result, vec!["OVERRIDE"]);
    }

    #[test]
    fn effective_prompt_agent_before_custom() {
        let engine = PromptEngine::new(vec![], vec![]);
        let ctx = make_ctx();

        let result =
            engine.build_effective_prompt(&ctx, None, Some("AGENT"), Some("CUSTOM"), None);
        assert_eq!(result, vec!["AGENT"]);
    }

    #[test]
    fn effective_prompt_custom_before_default() {
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "base",
                compute: Box::new(|_| Some("BASE".into())),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();

        let result = engine.build_effective_prompt(&ctx, None, None, Some("CUSTOM"), None);
        assert_eq!(result, vec!["CUSTOM"]);
    }

    #[test]
    fn effective_prompt_default_with_append() {
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "base",
                compute: Box::new(|_| Some("BASE".into())),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();

        let result =
            engine.build_effective_prompt(&ctx, None, None, None, Some("SUBAGENT_BLOCK"));
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "BASE");
        assert_eq!(result[1], DYNAMIC_BOUNDARY);
        assert_eq!(result[2], "SUBAGENT_BLOCK");
    }

    #[test]
    fn effective_prompt_no_append_uses_default() {
        let engine = PromptEngine::new(
            vec![PromptSection {
                name: "intro",
                compute: Box::new(|_| Some("INTRO".into())),
                cache_break: false,
            }],
            vec![],
        );
        let ctx = make_ctx();

        let result = engine.build_effective_prompt(&ctx, None, None, None, None);
        assert_eq!(result, vec!["INTRO", DYNAMIC_BOUNDARY]);
    }

    #[test]
    fn dynamic_cacheable_section_cached_in_dynamic_zone() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let engine = PromptEngine::new(
            vec![],
            vec![PromptSection {
                name: "dyn_cached",
                compute: Box::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                    Some("DC".into())
                }),
                cache_break: false,
            }],
        );
        let ctx = make_ctx();

        engine.build_system_prompt(&ctx);
        engine.build_system_prompt(&ctx);

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}

/// Integration tests for the full PromptEngine assembly pipeline using the
/// default engine configuration (`AgentRuntime::default_prompt_engine()`).
///
/// These verify end-to-end behavior of the prompt system under realistic
/// configurations: mode switching, tool set changes, language preferences,
/// MCP injection, caching behavior, and priority layering.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::runtime::prompt_sections::{
        actions_section, doing_tasks_section, intro_section, output_efficiency_section,
        system_section, tone_and_style_section, using_tools_section,
    };
    use crate::runtime::prompt_sections::dynamic::{
        environment_section, frc_section, language_section, mcp_instructions_section,
        memory_section, session_guidance_section, token_budget_section,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn default_engine() -> PromptEngine {
        let static_sections: Vec<PromptSection> = vec![
            intro_section(),
            system_section(),
            doing_tasks_section(),
            actions_section(),
            using_tools_section(),
            tone_and_style_section(),
            output_efficiency_section(),
        ];

        let dynamic_sections: Vec<PromptSection> = vec![
            session_guidance_section(),
            environment_section(),
            memory_section(),
            language_section(),
            mcp_instructions_section(),
            token_budget_section(),
            frc_section(),
        ];

        PromptEngine::new(static_sections, dynamic_sections)
    }

    fn full_ctx(mode: ExecutionMode, tools: &[&str], lang: Option<&str>) -> PromptContext {
        PromptContext {
            agent_config: Arc::new(AgentConfig {
                agent_id: "main".into(),
                name: None,
                description: None,
                model: Default::default(),
                system_prompt: None,
                tools: vec![],
                behavior: Default::default(),
                mcp_servers: vec![],
                min_tier: None,
                max_tier: None,
                avatar: None,
                channels: Default::default(),
            }),
            enabled_tools: tools.iter().map(|s| s.to_string()).collect(),
            deferred_tool_count: 0,
            model_id: "anthropic/claude-4-sonnet".into(),
            cwd: PathBuf::from("/home/user/project"),
            is_git: true,
            platform: "linux x86_64".into(),
            shell: "bash".into(),
            execution_mode: mode,
            mcp_servers: vec![],
            language_preference: lang.map(String::from),
            token_budget: None,
            memory_prompt: None,
            session_start_date: "2026-04-29".into(),
        }
    }

    const FULL_TOOLS: &[&str] = &[
        "read_file", "write_file", "edit_file", "shell_exec",
        "search_in_files", "glob", "list_directory", "tool_search",
        "todo_write", "ask_question", "memory_store", "memory_search",
    ];

    // ── 1. Plan mode prompt contains readonly restriction ──

    #[test]
    fn integration_plan_mode_assembled_prompt_contains_readonly() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Plan, FULL_TOOLS, None);
        let prompt = engine.build_effective_prompt(&ctx, None, None, None, None);
        let joined = prompt.join("\n");

        assert!(
            joined.contains("Plan Mode") || joined.contains("Read-Only"),
            "Plan mode prompt must contain readonly restriction"
        );
        assert!(
            joined.contains("CANNOT write files") || joined.contains("read-only tools"),
            "Plan mode prompt must explicitly restrict writes"
        );
    }

    // ── 2. Agent mode prompt contains full tool guidance ──

    #[test]
    fn integration_agent_mode_assembled_prompt_contains_tool_guidance() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);
        let prompt = engine.build_effective_prompt(&ctx, None, None, None, None);
        let joined = prompt.join("\n");

        assert!(joined.contains("using_tools"), "must include using_tools section");
        assert!(joined.contains("Decision Tree"), "must include tool decision tree");
        assert!(joined.contains("`read_file`"), "must reference actual tool names");
        assert!(joined.contains("`edit_file`"), "must reference edit_file");
        assert!(joined.contains("Anti-Patterns"), "must include anti-patterns");
        assert!(!joined.contains("Plan Mode"), "Agent mode must not include Plan mode block");
    }

    // ── 3. Different enabled_tools produce different session_guidance ──

    #[test]
    fn integration_different_tool_sets_produce_different_guidance() {
        let engine = default_engine();

        let ctx_with_subagent = full_ctx(ExecutionMode::Agent, &["task_create", "read_file"], None);
        let ctx_without_subagent = full_ctx(ExecutionMode::Agent, &["read_file", "edit_file"], None);

        let prompt_with = engine.build_effective_prompt(&ctx_with_subagent, None, None, None, None);
        engine.clear_cache();
        let prompt_without = engine.build_effective_prompt(&ctx_without_subagent, None, None, None, None);

        let joined_with = prompt_with.join("\n");
        let joined_without = prompt_without.join("\n");

        assert!(
            joined_with.contains("Sub-Agent") || joined_with.contains("Task Delegation"),
            "with task_create should have sub-agent guidance"
        );
        assert!(
            !joined_without.contains("Sub-Agent"),
            "without task_create should not have sub-agent guidance"
        );
        assert_ne!(joined_with, joined_without, "different tools must produce different prompts");
    }

    // ── 4. language_preference='Chinese' produces language section ──

    #[test]
    fn integration_chinese_language_preference_produces_language_section() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, Some("zh-CN"));
        let prompt = engine.build_effective_prompt(&ctx, None, None, None, None);
        let joined = prompt.join("\n");

        assert!(
            joined.contains("language_preference"),
            "Chinese preference must generate language section"
        );
        assert!(
            joined.contains("zh-CN"),
            "must include the specified language code"
        );
    }

    #[test]
    fn integration_no_language_preference_omits_language_section() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);
        let prompt = engine.build_effective_prompt(&ctx, None, None, None, None);
        let joined = prompt.join("\n");

        assert!(
            !joined.contains("language_preference"),
            "no preference should omit language section"
        );
    }

    // ── 5. MCP section recomputes every call (cache_break=true) ──

    #[test]
    fn integration_mcp_section_recomputes_on_every_call() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let engine = PromptEngine::new(
            vec![intro_section()],
            vec![PromptSection {
                name: "mcp_instructions",
                compute: Box::new(move |_ctx| {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    Some(format!("MCP call #{}", n + 1))
                }),
                cache_break: true,
            }],
        );

        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);

        let p1 = engine.build_system_prompt(&ctx);
        let p2 = engine.build_system_prompt(&ctx);
        let p3 = engine.build_system_prompt(&ctx);

        assert_eq!(counter.load(Ordering::SeqCst), 3, "MCP must recompute every call");
        assert!(p1.iter().any(|s| s.contains("MCP call #1")));
        assert!(p2.iter().any(|s| s.contains("MCP call #2")));
        assert!(p3.iter().any(|s| s.contains("MCP call #3")));
    }

    #[test]
    fn integration_default_mcp_section_changes_with_server_list() {
        let engine = default_engine();
        let mut ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);

        let prompt_no_mcp = engine.build_system_prompt(&ctx);
        assert!(
            !prompt_no_mcp.iter().any(|s| s.contains("mcp_instructions")),
            "no MCP servers → no MCP section"
        );

        ctx.mcp_servers = vec![McpServerInfo {
            id: "test-server".into(),
            instructions: Some("Use this for DB queries".into()),
        }];
        let prompt_with_mcp = engine.build_system_prompt(&ctx);
        assert!(
            prompt_with_mcp.iter().any(|s| s.contains("test-server")),
            "MCP servers present → MCP section appears (cache_break recomputes)"
        );
    }

    // ── 6. clear_cache forces full recomputation ──

    #[test]
    fn integration_clear_cache_forces_all_sections_recompute() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let engine = PromptEngine::new(
            vec![
                PromptSection {
                    name: "static_counted",
                    compute: Box::new(move |_| {
                        c.fetch_add(1, Ordering::SeqCst);
                        Some("STATIC".into())
                    }),
                    cache_break: false,
                },
            ],
            vec![],
        );
        let ctx = full_ctx(ExecutionMode::Agent, &["read_file"], None);

        engine.build_system_prompt(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        engine.build_system_prompt(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 1, "cached, no recompute");

        engine.clear_cache();
        engine.build_system_prompt(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 2, "clear_cache forced recompute");

        engine.clear_cache();
        engine.build_system_prompt(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 3, "clear_cache works repeatedly");
    }

    // ── 7. override_prompt overrides all other sections ──

    #[test]
    fn integration_override_prompt_overrides_all_sections() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, Some("zh-CN"));

        let result = engine.build_effective_prompt(
            &ctx,
            Some("CUSTOM OVERRIDE SYSTEM PROMPT"),
            Some("Agent custom prompt"),
            Some("User custom prompt"),
            Some("Append block"),
        );

        assert_eq!(result.len(), 1, "override must produce single element");
        assert_eq!(result[0], "CUSTOM OVERRIDE SYSTEM PROMPT");
    }

    #[test]
    fn integration_agent_prompt_takes_priority_over_default() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);

        let result = engine.build_effective_prompt(
            &ctx,
            None,
            Some("I am a custom agent prompt"),
            None,
            None,
        );

        assert_eq!(result, vec!["I am a custom agent prompt"]);
    }

    #[test]
    fn integration_append_prompt_added_to_default_sections() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);

        let result = engine.build_effective_prompt(
            &ctx,
            None,
            None,
            None,
            Some("SUBAGENT BLOCK APPENDED"),
        );

        let last = result.last().unwrap();
        assert_eq!(last, "SUBAGENT BLOCK APPENDED");
        assert!(result.len() > 2, "should have default sections + append");
    }

    // ── 8. Full assembly sanity: default engine produces all expected sections ──

    #[test]
    fn integration_default_engine_full_assembly_contains_all_core_sections() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, Some("zh-CN"));
        let prompt = engine.build_system_prompt(&ctx);
        let joined = prompt.join("\n");

        assert!(joined.contains("FastClaw"), "intro section must be present");
        assert!(joined.contains("security"), "security directives must be present");
        assert!(
            joined.contains("system_communication") || joined.contains("auto_compression"),
            "system section must be present"
        );
        assert!(
            joined.contains("making_code_changes") || joined.contains("最小改动"),
            "doing_tasks section must be present"
        );
        assert!(
            joined.contains("actions_and_reversibility") || joined.contains("可逆"),
            "actions section must be present"
        );
        assert!(
            joined.contains("using_tools") || joined.contains("工具使用"),
            "using_tools section must be present"
        );
        assert!(
            joined.contains("tone_and_style") || joined.contains("沟通风格"),
            "tone section must be present"
        );
        assert!(
            joined.contains("output_efficiency") || joined.contains("沟通规范"),
            "output_efficiency section must be present"
        );
        assert!(joined.contains("environment"), "environment section must be present");
        assert!(joined.contains("session_guidance"), "session_guidance must be present");
        assert!(joined.contains("language_preference"), "language section must be present");
        assert!(
            joined.contains("function_result_clearing") || joined.contains("工具调用结果"),
            "frc section must be present"
        );
        assert!(
            joined.contains(DYNAMIC_BOUNDARY),
            "dynamic boundary marker must be present"
        );
    }

    #[test]
    fn integration_prompt_order_static_before_dynamic() {
        let engine = default_engine();
        let ctx = full_ctx(ExecutionMode::Agent, FULL_TOOLS, None);
        let prompt = engine.build_system_prompt(&ctx);

        let boundary_idx = prompt.iter().position(|s| s == DYNAMIC_BOUNDARY)
            .expect("boundary must exist");

        for part in &prompt[..boundary_idx] {
            assert!(
                !part.contains("environment") || !part.contains("Working directory"),
                "environment (dynamic) must not appear before boundary"
            );
        }

        let post_boundary = prompt[boundary_idx + 1..].join("\n");
        assert!(
            post_boundary.contains("environment") || post_boundary.contains("Working directory"),
            "environment must appear after boundary"
        );
    }
}
