use std::sync::Arc;

use fastclaw_core::types::ChatMessage;

use crate::llm::LlmProvider;
use super::context_compressor;
use super::context_budget::{apply_token_budget, BudgetConfig};
use super::session_memory;
use super::tool_executor::{
    cache_window_for_occupancy, collect_eviction_manifest, compute_protected_indices,
    dedup_repeated_tool_calls, keep_recent_for_context_window,
    microcompact_tool_results_with_protection, snapshot_tool_contents,
    time_based_microcompact_with_protection, rebuild_recall_registry, ProtectionWindowConfig,
};

/// Result of the unified pre-query compression pipeline.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct UnifiedCompactResult {
    pub estimated_tokens: usize,
    pub compressed_by_llm: bool,
    pub tokens_saved_by_llm: usize,
    pub pipeline_applied: bool,
    pub session_memory_extracted: bool,
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn unified_pre_query_compact(
    messages: &mut Vec<ChatMessage>,
    pipeline: &mut fastclaw_context::ContextPipeline,
    context_window: u32,
    max_tokens: Option<u32>,
    provider: &Arc<dyn LlmProvider>,
    model: &str,
    last_estimated_tokens: usize,
    iteration_boundaries: &[(usize, std::time::Instant)],
    todo_store: Option<&crate::builtin_tools::TodoStore>,
    enable_smart_compression: bool,
) -> UnifiedCompactResult {
    // Compute the protection window — tool results from the last N iterations
    // are immune to all forms of compression.
    let protected = if enable_smart_compression {
        let protection_config = ProtectionWindowConfig::default();
        compute_protected_indices(messages, iteration_boundaries, &protection_config)
    } else {
        std::collections::HashSet::new()
    };

    // Snapshot tool contents before compression for eviction manifest.
    let pre_snapshot = if enable_smart_compression {
        snapshot_tool_contents(messages)
    } else {
        Vec::new()
    };

    // Step 0: Time-based microcompact — use occupancy-aware cache window.
    let pre_estimate = fastclaw_context::estimate_messages_tokens(messages);
    let dynamic_cache_window = if enable_smart_compression {
        cache_window_for_occupancy(pre_estimate, context_window)
    } else {
        super::tool_executor::DEFAULT_CACHE_WINDOW_DURATION
    };
    let time_compacted = time_based_microcompact_with_protection(
        messages,
        iteration_boundaries,
        dynamic_cache_window,
        &protected,
    );
    if time_compacted > 0 {
        tracing::debug!(time_compacted, "time-based microcompact collapsed stale tool results");
    }

    // Step 1: Tier-aware microcompact of old tool results.
    let keep_recent = keep_recent_for_context_window(context_window);
    microcompact_tool_results_with_protection(messages, keep_recent, &protected);

    // Step 1.5: Token budget allocation — enforce the 30/40/20/10 split
    // so that older tool results don't crowd out recent ones.
    let budget_result = apply_token_budget(messages, context_window, &BudgetConfig::default());
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
                role: fastclaw_core::types::Role::System,
                content: Some(serde_json::Value::String(manifest_text)),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            };
            let insert_pos = messages
                .iter()
                .rposition(|m| matches!(m.role, fastclaw_core::types::Role::User))
                .unwrap_or(messages.len());
            messages.insert(insert_pos, manifest_msg);
        }
    }

    // Step 3: Content filter — truncate oversized tool results, remove empty,
    // deduplicate consecutive identical system messages.
    // When smart compression is enabled, threshold is occupancy-aware.
    {
        let max_tool_chars = if enable_smart_compression {
            let current_estimate = fastclaw_context::estimate_messages_tokens(messages);
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
        let filter = fastclaw_context::ContentFilterHook::new(max_tool_chars);
        let _ = fastclaw_context::ContextHook::on_assemble(&filter, messages).await;
    }

    // Step 4: System reminder — nudge every 20 user turns
    {
        let reminder = fastclaw_context::SystemReminderHook::default();
        let _ = fastclaw_context::ContextHook::on_assemble(&reminder, messages).await;
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

    // Detect and strip the [COMPACT_REQUESTED] marker injected by /compact.
    let force_compact = messages.iter().any(|m| {
        matches!(m.role, fastclaw_core::types::Role::System)
            && m.content
                .as_ref()
                .and_then(|c| c.as_str())
                .map_or(false, |t| t.contains("[COMPACT_REQUESTED]"))
    });
    if force_compact {
        messages.retain(|m| {
            !(matches!(m.role, fastclaw_core::types::Role::System)
                && m.content
                    .as_ref()
                    .and_then(|c| c.as_str())
                    .map_or(false, |t| t.contains("[COMPACT_REQUESTED]")))
        });
        tracing::info!("force-compact requested via /compact command");
    }

    // Step 5.5: Session memory extraction — before LLM compression, try to
    // extract key facts/decisions/task state so that even aggressive compression
    // preserves essential context.
    let pre_compress_estimate = fastclaw_context::estimate_messages_tokens(messages);
    let extraction = session_memory::extract_session_memory(
        messages,
        provider,
        model,
        context_window,
        if last_estimated_tokens > 0 {
            last_estimated_tokens
        } else {
            pre_compress_estimate
        },
    )
    .await;
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
        && iteration_count % PERIODIC_CLEANUP_INTERVAL == 0
        && !force_compact;
    if periodic_cleanup {
        tracing::info!(
            iteration = iteration_count,
            "periodic context cleanup triggered (every {} iterations)",
            PERIODIC_CLEANUP_INTERVAL,
        );
    }

    // Step 6: LLM-based compression
    // When force_compact is set (user ran /compact), bypass the circuit breaker
    // and use a context_window of 1 so the threshold is effectively 0 — this
    // guarantees compression triggers regardless of current token usage.
    // periodic_cleanup uses a lowered threshold (0.25) to proactively compress.
    let compress_result = if force_compact || periodic_cleanup || pipeline.should_attempt_autocompact() {
        let local_estimate = fastclaw_context::estimate_messages_tokens(messages);
        let effective_window = if force_compact { 1 } else { context_window };

        let dynamic_threshold = if force_compact {
            0.0_f32
        } else if periodic_cleanup {
            0.25_f32
        } else if enable_smart_compression {
            let (sys_tok, tool_tok, conv_tok) = estimate_token_distribution(messages);
            let has_active_task = todo_store.map_or(false, |t| t.has_in_progress_items());
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
                "post-compact: LLM compression reduced context"
            );
        } else if result.original_tokens > 0 && !force_compact {
            pipeline.record_autocompact_failure();
        }
        result
    } else {
        tracing::debug!("LLM autocompact skipped (circuit breaker tripped)");
        context_compressor::CompressionResult {
            compressed: false,
            original_tokens: 0,
            new_tokens: 0,
            messages: messages.clone(),
            history_file: None,
        }
    };

    // Step 7: Hard fit messages within context window budget
    let estimated_tokens = fastclaw_context::ContextEngine::fit_to_context_window(
        messages,
        context_window,
        max_tokens,
    );

    // Step 8: Rebuild the auto-recall registry from the compacted messages
    // so cleared results can be re-executed on demand.
    rebuild_recall_registry(messages);

    let tokens_saved_by_llm = if compress_result.compressed {
        compress_result
            .original_tokens
            .saturating_sub(compress_result.new_tokens)
    } else {
        0
    };

    UnifiedCompactResult {
        estimated_tokens,
        compressed_by_llm: compress_result.compressed,
        tokens_saved_by_llm,
        pipeline_applied,
        session_memory_extracted,
    }
}

/// Estimate token distribution across system, tool, and conversation messages.
/// Returns (system_tokens, tool_tokens, conversation_tokens).
fn estimate_token_distribution(messages: &[ChatMessage]) -> (usize, usize, usize) {
    use fastclaw_core::types::Role;

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
