use std::sync::Arc;

use xiaolin_core::types::ChatMessage;

use super::context_budget::{apply_token_budget, BudgetConfig};
use super::context_compressor;
use super::post_compact_restore::RestorationState;
use super::session_memory;
use super::tool_executor::{
    cache_window_for_occupancy, collect_eviction_manifest, compute_protected_indices,
    dedup_repeated_tool_calls, keep_recent_for_context_window,
    microcompact_tool_results_with_protection, rebuild_recall_registry, snapshot_tool_contents,
    time_based_microcompact_with_protection, ProtectionWindowConfig,
};
use crate::llm::LlmProvider;

/// Bridges [`xiaolin_context::collapse::CollapseSummarizer`] to the LLM provider,
/// reusing the same compact-model call path as autocompact.
struct LlmCollapseSummarizer {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

#[async_trait::async_trait]
impl xiaolin_context::collapse::CollapseSummarizer for LlmCollapseSummarizer {
    async fn summarize(&self, messages: &[ChatMessage]) -> anyhow::Result<String> {
        let mut conversation = Vec::with_capacity(messages.len() + 2);
        conversation.push(ChatMessage {
            role: xiaolin_core::types::Role::System,
            content: Some(serde_json::json!(
                "Summarize the following conversation rounds into a concise paragraph. \
                 Preserve key decisions, file paths, tool outputs, and action items. \
                 Omit greetings, filler, and redundant details."
            )),
            ..Default::default()
        });
        conversation.extend_from_slice(messages);
        conversation.push(ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::json!(
                "Please produce a concise summary of the above conversation."
            )),
            ..Default::default()
        });

        let params = crate::llm::CompletionParams {
            model: &self.model,
            messages: &conversation,
            temperature: 0.0,
            max_tokens: Some(1024),
            tools: None,
        };
        let response = self.provider.chat_completion(&params).await?;
        let text = response
            .choices
            .first()
            .and_then(|c| c.message.text_content().map(|s| s.to_string()))
            .unwrap_or_default();
        Ok(text)
    }
}

/// Read an environment variable as a boolean flag.
/// Returns `default` if the variable is unset; parses "1"/"true"/"yes" as true.
fn env_var_is_true(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(val) => match val.to_lowercase().as_str() {
            "1" | "true" | "yes" => true,
            "0" | "false" | "no" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

/// Feature gates for the unified compact pipeline.
/// These can be toggled via environment variables.
pub(crate) struct CompactFeatureGates {
    /// Enable time-based microcompact (XIAOLIN_ENABLE_TIME_MICROCOMPACT, default: true)
    pub enable_time_microcompact: bool,
    /// Enable importance-based microcompact (XIAOLIN_ENABLE_MICROCOMPACT, default: true)
    pub enable_importance_microcompact: bool,
    /// Enable token budget allocation (XIAOLIN_ENABLE_BUDGET, default: true)
    pub enable_budget_allocation: bool,
    /// Enable eviction manifest injection (XIAOLIN_ENABLE_EVICTION_MANIFEST, default: true)
    pub enable_eviction_manifest: bool,
    /// Enable LLM-based auto-compact (XIAOLIN_ENABLE_LLM_COMPACT, default: true)
    pub enable_llm_compact: bool,
}

impl Default for CompactFeatureGates {
    fn default() -> Self {
        Self {
            enable_time_microcompact: env_var_is_true("XIAOLIN_ENABLE_TIME_MICROCOMPACT", true),
            enable_importance_microcompact: env_var_is_true("XIAOLIN_ENABLE_MICROCOMPACT", true),
            enable_budget_allocation: env_var_is_true("XIAOLIN_ENABLE_BUDGET", true),
            enable_eviction_manifest: env_var_is_true("XIAOLIN_ENABLE_EVICTION_MANIFEST", true),
            enable_llm_compact: env_var_is_true("XIAOLIN_ENABLE_LLM_COMPACT", true),
        }
    }
}

/// Result of the unified pre-query compression pipeline.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct UnifiedCompactResult {
    pub estimated_tokens: usize,
    pub compressed_by_llm: bool,
    pub tokens_saved_by_llm: usize,
    pub pipeline_applied: bool,
    pub session_memory_extracted: bool,
    /// Whether post-compact state restoration was applied.
    pub state_restored: bool,
    /// Extracted/updated session memory (if any), for the caller to persist.
    pub extracted_memory: Option<session_memory::SessionMemory>,
    /// When true, the caller should invalidate `FileStateCache` to prevent
    /// stale dedup detection after context compression removed file content.
    pub file_state_needs_invalidation: bool,
}

/// Run all pre-query compression steps in a single call.
///
/// Replaces the ~80 lines of scattered compression code in `execute_stream_inner`:
///   1. Microcompact old tool results
///   2. Deduplicate repeated tool calls
///   3. ContentFilterHook (truncate oversized results, remove empty messages)
///   4. SystemReminderHook (nudge every N turns)
///   5. ContextPipeline::pre_query_compact (snip + importance)
///   6. LLM-based compression (with circuit breaker)
///   7. Hard fit to context window
///   8. Post-compact state restoration (files, skills, plan)
#[allow(clippy::too_many_arguments)]
pub(crate) async fn unified_pre_query_compact(
    messages: &mut Vec<ChatMessage>,
    pipeline: &mut xiaolin_context::ContextPipeline,
    context_window: u32,
    max_tokens: Option<u32>,
    provider: &Arc<dyn LlmProvider>,
    model: &str,
    last_estimated_tokens: usize,
    iteration_boundaries: &[(usize, std::time::Instant)],
    todo_store: Option<&crate::builtin_tools::TodoStore>,
    enable_smart_compression: bool,
    restoration_state: Option<&RestorationState>,
    existing_memory: Option<&session_memory::SessionMemory>,
) -> UnifiedCompactResult {
    // Compute the protection window — tool results from the last N iterations
    // are immune to all forms of compression.
    let gates = CompactFeatureGates::default();
    let protected = if enable_smart_compression {
        let protection_config = ProtectionWindowConfig::default();
        compute_protected_indices(messages, iteration_boundaries, &protection_config)
    } else {
        std::collections::HashSet::new()
    };

    // Snapshot tool contents before compression for eviction manifest.
    let pre_snapshot = if enable_smart_compression && gates.enable_eviction_manifest {
        snapshot_tool_contents(messages)
    } else {
        Vec::new()
    };

    // Step 0: Time-based microcompact — use occupancy-aware cache window.
    let pre_estimate = xiaolin_context::estimate_messages_tokens(messages);
    if enable_smart_compression && gates.enable_time_microcompact {
        let dynamic_cache_window = cache_window_for_occupancy(pre_estimate, context_window);
        let time_compacted = time_based_microcompact_with_protection(
            messages,
            iteration_boundaries,
            dynamic_cache_window,
            &protected,
        );
        if time_compacted > 0 {
            tracing::debug!(
                time_compacted,
                "time-based microcompact collapsed stale tool results"
            );
        }
    }

    // Step 0.5: Cached microcompact — reuse cross-turn cache before other steps.
    let cached_mc_result = pipeline.run_cached_microcompact(messages);
    if cached_mc_result.tokens_freed > 0 {
        tracing::debug!(
            cache_hits = cached_mc_result.cache_hits,
            new_compressions = cached_mc_result.new_compressions,
            tokens_freed = cached_mc_result.tokens_freed,
            entries_evicted = cached_mc_result.entries_evicted,
            "cached microcompact applied"
        );
    }

    // Step 1: Tier-aware microcompact of old tool results.
    if enable_smart_compression && gates.enable_importance_microcompact {
        let keep_recent = keep_recent_for_context_window(context_window);
        microcompact_tool_results_with_protection(messages, keep_recent, &protected);
    }

    // Step 1.5: Token budget allocation — enforce the 30/40/20/10 split
    // so that older tool results don't crowd out recent ones.
    let budget_result = if enable_smart_compression && gates.enable_budget_allocation {
        apply_token_budget(messages, context_window, &BudgetConfig::default())
    } else {
        super::context_budget::BudgetResult::default()
    };
    if budget_result.total_tokens_freed > 0 {
        tracing::debug!(
            older_summarized = budget_result.older_tools_summarized,
            recent_trimmed = budget_result.recent_tools_trimmed,
            tokens_freed = budget_result.total_tokens_freed,
            "token budget allocation freed context"
        );
    }

    // Step 2: Deduplicate repeated tool calls on the same target
    dedup_repeated_tool_calls(messages);

    // Step 2.5: Build eviction manifest from what was compressed above,
    // then inject as a system message so the agent knows what was evicted.
    if enable_smart_compression {
        let eviction_manifest = collect_eviction_manifest(&pre_snapshot, messages);
        if !eviction_manifest.is_empty() {
            let manifest_text = eviction_manifest.to_system_message();
            tracing::debug!(
                evicted_count = eviction_manifest.entries.len(),
                "injecting eviction manifest"
            );
            let manifest_msg = ChatMessage {
                role: xiaolin_core::types::Role::System,
                content: Some(serde_json::Value::String(manifest_text)),
                ..Default::default()
            };
            let insert_pos = messages
                .iter()
                .rposition(|m| matches!(m.role, xiaolin_core::types::Role::User))
                .unwrap_or(messages.len());
            messages.insert(insert_pos, manifest_msg);
        }
    }

    // Step 3: Content filter — truncate oversized tool results, remove empty,
    // deduplicate consecutive identical system messages.
    // When smart compression is enabled, threshold is occupancy-aware.
    {
        let max_tool_chars = if enable_smart_compression {
            let current_estimate = xiaolin_context::estimate_messages_tokens(messages);
            let occupancy = if context_window > 0 {
                current_estimate as f64 / context_window as f64
            } else {
                0.5
            };
            if occupancy < 0.50 {
                8000
            } else if occupancy < 0.80 {
                4000
            } else {
                2000
            }
        } else {
            2000
        };
        let filter = xiaolin_context::ContentFilterHook::new(max_tool_chars);
        let _ = xiaolin_context::ContextHook::on_assemble(&filter, messages).await;
    }

    // Step 4: System reminder — nudge every 20 user turns
    {
        let reminder = xiaolin_context::SystemReminderHook::default();
        let _ = xiaolin_context::ContextHook::on_assemble(&reminder, messages).await;
    }

    // Step 5: Pipeline pre_query_compact (snip + importance-based eviction)
    let (compacted, pipeline_meta) = pipeline.pre_query_compact(messages);
    let pipeline_applied = pipeline_meta.snip_applied || pipeline_meta.micro_applied;
    if pipeline_applied {
        tracing::info!(
            snip_freed = pipeline_meta.snip_tokens_freed,
            snip_rounds = pipeline_meta.snip_rounds_removed,
            micro_evicted = pipeline_meta.micro_evicted,
            total_freed = pipeline_meta.total_tokens_freed,
            "pre-query pipeline compacted context"
        );
        *messages = compacted;
    }

    // Step 5.1: Context Collapse — LLM-based round summarization (Layer 3).
    // When enabled, collapses old API rounds into summaries stored in CollapseStore,
    // then projects them into the message list. Mutually exclusive with autocompact.
    if pipeline.config().enable_collapse {
        let engine =
            xiaolin_context::collapse::CollapseEngine::new(Default::default());
        let summarizer = LlmCollapseSummarizer {
            provider: provider.clone(),
            model: model.to_string(),
        };
        match engine
            .collapse(
                messages,
                context_window as usize,
                &mut pipeline.collapse_store,
                &summarizer,
            )
            .await
        {
            Ok(result) => {
                if let Some(ref span) = result.span {
                    tracing::info!(
                        start = span.start_round,
                        end = span.end_round,
                        saved = span.tokens_saved(),
                        mode = ?result.mode,
                        "context collapse summarized rounds"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "context collapse failed, continuing without");
            }
        }
        // Project collapsed summaries into the message list.
        if !pipeline.collapse_store.is_empty() {
            *messages = xiaolin_context::collapse::project(messages, &pipeline.collapse_store);
        }
    }

    // Detect and strip the [COMPACT_REQUESTED] marker injected by /compact.
    let force_compact = messages.iter().any(|m| {
        matches!(m.role, xiaolin_core::types::Role::System)
            && m.content
                .as_ref()
                .and_then(|c| c.as_str())
                .is_some_and(|t| t.contains("[COMPACT_REQUESTED]"))
    });
    if force_compact {
        messages.retain(|m| {
            !(matches!(m.role, xiaolin_core::types::Role::System)
                && m.content
                    .as_ref()
                    .and_then(|c| c.as_str())
                    .is_some_and(|t| t.contains("[COMPACT_REQUESTED]")))
        });
        tracing::info!("force-compact requested via /compact command");
    }

    // Step 5.5: Session memory extraction — before LLM compression, try to
    // extract key facts/decisions/task state so that even aggressive compression
    // preserves essential context.
    // When existing memory is available, use incremental extraction (only new msgs).
    let pre_compress_estimate = xiaolin_context::estimate_messages_tokens(messages);
    let effective_estimate = if last_estimated_tokens > 0 {
        last_estimated_tokens
    } else {
        pre_compress_estimate
    };
    let extraction = if let Some(existing) = existing_memory {
        session_memory::extract_incremental(
            messages,
            existing,
            provider,
            model,
            context_window,
            effective_estimate,
        )
        .await
    } else {
        session_memory::extract_session_memory(
            messages,
            provider,
            model,
            context_window,
            effective_estimate,
        )
        .await
    };
    let session_memory_extracted = extraction.memory.is_some();
    if let Some(ref mem) = extraction.memory {
        session_memory::inject_session_memory(messages, mem);
    }

    // Step 5.6: Periodic cleanup — for long-running tasks, proactively compress
    // every PERIODIC_CLEANUP_INTERVAL iterations even if threshold isn't reached.
    // This prevents unbounded context growth in large context windows (e.g., 1M).
    const PERIODIC_CLEANUP_INTERVAL: usize = 15;
    let iteration_count = iteration_boundaries.len();
    let periodic_cleanup = iteration_count > 0
        && iteration_count.is_multiple_of(PERIODIC_CLEANUP_INTERVAL)
        && !force_compact;
    if periodic_cleanup {
        tracing::info!(
            iteration = iteration_count,
            "periodic context cleanup triggered (every {} iterations)",
            PERIODIC_CLEANUP_INTERVAL,
        );
    }

    // Step 6: LLM-based compression
    // ─────────────────────────────────────────────────────────────────────
    // Preemptive compression (Claude-Code style):
    // Triggers when: estimated_tokens > effective_window - 13K buffer
    // This is roughly 93% of context window (for 200K: 200K - 20K - 13K = 167K)
    // ─────────────────────────────────────────────────────────────────────
    let preemptive_threshold = context_compressor::compute_preemptive_threshold(context_window);
    let blocking_limit = context_compressor::compute_blocking_limit(context_window);
    let current_estimate = xiaolin_context::estimate_messages_tokens(messages);
    let should_preemptive_compact = current_estimate > preemptive_threshold;

    // When force_compact is set (user ran /compact), bypass the circuit breaker
    // and use a context_window of 1 so the threshold is effectively 0 — this
    // guarantees compression triggers regardless of current token usage.
    // periodic_cleanup uses a lowered threshold (0.25) to proactively compress.
    // Preemptive compact uses the buffer-based threshold (matches Claude-Code).
    // Skip LLM compression entirely if disabled via feature gate.
    let collapse_active = pipeline.config().enable_collapse;
    let compress_result = if gates.enable_llm_compact
        && !collapse_active
        && (force_compact
            || periodic_cleanup
            || (should_preemptive_compact && pipeline.should_attempt_autocompact()))
    {
        let local_estimate = current_estimate;
        let effective_window = if force_compact { 1 } else { context_window };

        let dynamic_threshold = if force_compact {
            0.0_f32
        } else if periodic_cleanup {
            0.25_f32
        } else if should_preemptive_compact {
            // Preemptive threshold: trigger when tokens > effective_window - buffer
            // Compute as a fraction for the existing compression logic
            let effective = context_compressor::effective_context_window(context_window);
            if effective > 0 {
                (preemptive_threshold as f32) / (effective as f32)
            } else {
                0.50
            }
        } else if enable_smart_compression {
            let (sys_tok, tool_tok, conv_tok) = estimate_token_distribution(messages);
            let has_active_task = todo_store.is_some_and(|t| t.has_in_progress_items());
            context_compressor::compute_compression_threshold(
                sys_tok,
                tool_tok,
                conv_tok,
                context_window,
                has_active_task,
            )
        } else {
            context_compressor::COMPRESSION_THRESHOLD
        };

        tracing::debug!(
            local_estimate,
            api_prompt_tokens = last_estimated_tokens,
            force_compact,
            periodic_cleanup,
            should_preemptive_compact,
            preemptive_threshold,
            blocking_limit,
            dynamic_threshold,
            "pre-compact: entering LLM compression"
        );
        let result = context_compressor::try_compress_chat_with_threshold(
            messages,
            effective_window,
            provider,
            model,
            last_estimated_tokens,
            todo_store,
            dynamic_threshold,
        )
        .await;

        if result.compressed {
            pipeline.record_autocompact_success();
            tracing::info!(
                original = result.original_tokens,
                new = result.new_tokens,
                saved = result.original_tokens.saturating_sub(result.new_tokens),
                force_compact,
                should_preemptive_compact,
                "post-compact: LLM compression reduced context"
            );
        } else if result.original_tokens > 0 && !force_compact {
            pipeline.record_autocompact_failure();
        }
        result
    } else {
        tracing::debug!(
            local_estimate = current_estimate,
            preemptive_threshold,
            circuit_breaker_ok = pipeline.should_attempt_autocompact(),
            "LLM autocompact skipped"
        );
        context_compressor::CompressionResult::no_op(messages.clone())
    };

    // Step 7: Post-compact state restoration
    // After LLM compression, inject restoration messages for files/skills/plan
    // so essential context is preserved even after aggressive compaction.
    let mut state_restored = false;
    if let Some(restoration) = restoration_state {
        let restoration_messages = restoration.generate_restoration_messages();
        if !restoration_messages.is_empty() {
            // Find the last user message position to inject before it
            let insert_pos = messages
                .iter()
                .rposition(|m| matches!(m.role, xiaolin_core::types::Role::User))
                .unwrap_or(messages.len().saturating_sub(1).max(0));

            // Insert restoration messages before the last user message
            for msg in restoration_messages.into_iter().rev() {
                messages.insert(insert_pos, msg);
            }
            state_restored = true;
            tracing::debug!("post-compact state restoration injected context");
        }
    }

    // Step 8: Hard fit messages within context window budget
    let estimated_tokens = xiaolin_context::ContextEngine::fit_to_context_window(
        messages,
        context_window,
        max_tokens,
    );

    // Step 9: Rebuild the auto-recall registry from the compacted messages
    // so cleared results can be re-executed on demand.
    rebuild_recall_registry(messages);

    let tokens_saved_by_llm = if compress_result.compressed {
        compress_result
            .original_tokens
            .saturating_sub(compress_result.new_tokens)
    } else {
        0
    };

    let any_significant_compression =
        compress_result.compressed || pipeline_applied || !pipeline.collapse_store.is_empty();

    UnifiedCompactResult {
        estimated_tokens,
        compressed_by_llm: compress_result.compressed,
        tokens_saved_by_llm,
        pipeline_applied,
        session_memory_extracted,
        extracted_memory: extraction.memory,
        state_restored,
        file_state_needs_invalidation: any_significant_compression,
    }
}

/// Estimate token distribution across system, tool, and conversation messages.
/// Returns (system_tokens, tool_tokens, conversation_tokens).
fn estimate_token_distribution(messages: &[ChatMessage]) -> (usize, usize, usize) {
    use xiaolin_core::types::Role;

    let mut system_tokens = 0usize;
    let mut tool_tokens = 0usize;
    let mut conversation_tokens = 0usize;

    for msg in messages {
        let chars = msg
            .content
            .as_ref()
            .map(|c| c.to_string().len())
            .unwrap_or(0);
        let tok_estimate = chars / 4 + 4; // rough token heuristic

        match msg.role {
            Role::System => system_tokens += tok_estimate,
            Role::Tool => tool_tokens += tok_estimate,
            Role::User | Role::Assistant => conversation_tokens += tok_estimate,
        }
    }

    (system_tokens, tool_tokens, conversation_tokens)
}
