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
