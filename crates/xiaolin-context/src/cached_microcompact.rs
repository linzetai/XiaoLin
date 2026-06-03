use std::collections::HashMap;

use xiaolin_core::types::{ChatMessage, Role};

use crate::compressor::estimate_messages_tokens;

/// A lightweight content fingerprint used to detect whether a tool result has
/// already been compressed in a previous pipeline pass.
fn content_fingerprint(msg: &ChatMessage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Some(ref c) = msg.content {
        c.to_string().hash(&mut hasher);
    }
    if let Some(ref name) = msg.name {
        name.hash(&mut hasher);
    }
    hasher.finish()
}

/// Entry in the cached microcompact store.
#[derive(Debug, Clone)]
struct CacheEntry {
    fingerprint: u64,
    compressed_content: String,
}

/// Cached microcompact avoids re-compressing tool results that haven't changed
/// since the last pipeline pass.
///
/// How it works:
/// 1. On first pass, tool results exceeding `threshold_chars` are compressed
///    and the original fingerprint + compressed result are stored.
/// 2. On subsequent passes, if a tool result's fingerprint matches a cached
///    entry, the cached compressed version is used immediately (zero LLM cost).
/// 3. Entries are evicted when they haven't been referenced for `max_age` passes.
#[derive(Debug)]
pub struct CachedMicrocompactor {
    cache: HashMap<String, CacheEntry>,
    pass_count: u32,
    last_referenced: HashMap<String, u32>,
    config: CachedMicrocompactConfig,
}

/// Configuration for the cached microcompact layer.
#[derive(Debug, Clone)]
pub struct CachedMicrocompactConfig {
    /// Tool results exceeding this character count are candidates for compression.
    pub threshold_chars: usize,
    /// Maximum number of passes an unused cache entry survives before eviction.
    pub max_age_passes: u32,
    /// Maximum character count for compressed output.
    pub max_compressed_chars: usize,
    /// Number of recent tool results to skip (they stay uncompressed).
    pub recent_window: usize,
}

impl Default for CachedMicrocompactConfig {
    fn default() -> Self {
        Self {
            threshold_chars: 2000,
            max_age_passes: 5,
            max_compressed_chars: 400,
            recent_window: 4,
        }
    }
}

/// Result of a cached microcompact pass.
#[derive(Debug, Clone)]
pub struct CachedMicrocompactResult {
    pub cache_hits: usize,
    pub new_compressions: usize,
    pub tokens_freed: usize,
    pub entries_evicted: usize,
}

impl CachedMicrocompactor {
    pub fn new(config: CachedMicrocompactConfig) -> Self {
        Self {
            cache: HashMap::new(),
            pass_count: 0,
            last_referenced: HashMap::new(),
            config,
        }
    }

    /// Run a cached microcompact pass over tool result messages.
    ///
    /// Returns the number of tokens freed. Modifies `messages` in place.
    pub fn compact(&mut self, messages: &mut [ChatMessage]) -> CachedMicrocompactResult {
        self.pass_count += 1;
        let mut cache_hits = 0usize;
        let mut new_compressions = 0usize;
        let mut tokens_freed = 0usize;

        let tool_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::Tool)
            .map(|(i, _)| i)
            .collect();

        let skip_count = tool_indices.len().saturating_sub(self.config.recent_window);
        let candidates = &tool_indices[..skip_count];

        for &idx in candidates {
            let msg = &messages[idx];
            let text = match msg.text_content() {
                Some(t) => t,
                None => continue,
            };

            if text.len() < self.config.threshold_chars {
                continue;
            }

            if text.starts_with("[summarized]")
                || text.starts_with("[faded]")
                || text.starts_with("[oneliner]")
                || text.starts_with("[time-compacted]")
                || text.starts_with("[cached-mc]")
                || text == "[Old tool result content cleared]"
            {
                continue;
            }

            let fingerprint = content_fingerprint(msg);
            let cache_key = format!(
                "{}:{}",
                msg.name.as_deref().unwrap_or("unknown"),
                msg.tool_call_id.as_deref().unwrap_or("")
            );

            if let Some(entry) = self.cache.get(&cache_key) {
                if entry.fingerprint == fingerprint {
                    let before_tokens =
                        estimate_messages_tokens(std::slice::from_ref(&messages[idx]));
                    messages[idx].content = Some(serde_json::Value::String(format!(
                        "[cached-mc] {}",
                        entry.compressed_content
                    )));
                    let after_tokens =
                        estimate_messages_tokens(std::slice::from_ref(&messages[idx]));
                    tokens_freed += before_tokens.saturating_sub(after_tokens);
                    cache_hits += 1;
                    self.last_referenced
                        .insert(cache_key, self.pass_count);
                    continue;
                }
            }

            let before_tokens = estimate_messages_tokens(std::slice::from_ref(&messages[idx]));
            let compressed = Self::compress_content(&text, self.config.max_compressed_chars);
            let new_content = format!("[cached-mc] {compressed}");
            messages[idx].content = Some(serde_json::Value::String(new_content.clone()));
            let after_tokens = estimate_messages_tokens(std::slice::from_ref(&messages[idx]));
            tokens_freed += before_tokens.saturating_sub(after_tokens);

            self.cache.insert(
                cache_key.clone(),
                CacheEntry {
                    fingerprint,
                    compressed_content: compressed,
                },
            );
            self.last_referenced.insert(cache_key, self.pass_count);
            new_compressions += 1;
        }

        let entries_evicted = self.evict_stale();

        CachedMicrocompactResult {
            cache_hits,
            new_compressions,
            tokens_freed,
            entries_evicted,
        }
    }

    /// Evict cache entries that haven't been referenced within `max_age_passes`.
    fn evict_stale(&mut self) -> usize {
        let threshold = self.pass_count.saturating_sub(self.config.max_age_passes);
        let stale_keys: Vec<String> = self
            .last_referenced
            .iter()
            .filter(|(_, &last)| last < threshold)
            .map(|(k, _)| k.clone())
            .collect();

        let count = stale_keys.len();
        for key in stale_keys {
            self.cache.remove(&key);
            self.last_referenced.remove(&key);
        }
        count
    }

    /// Reset cache (e.g. after /clear or session change).
    pub fn clear(&mut self) {
        self.cache.clear();
        self.last_referenced.clear();
        self.pass_count = 0;
    }

    /// Number of entries currently in cache.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    fn compress_content(text: &str, max_chars: usize) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= 3 && text.len() <= max_chars {
            return text.to_string();
        }

        let mut result = String::with_capacity(max_chars);

        if let Some(first_line) = lines.first() {
            if first_line.len() <= 200 {
                result.push_str(first_line);
                result.push('\n');
            }
        }

        let important_lines: Vec<&&str> = lines
            .iter()
            .filter(|l| {
                l.contains("error")
                    || l.contains("Error")
                    || l.contains("fn ")
                    || l.contains("struct ")
                    || l.contains("impl ")
                    || l.contains("class ")
                    || l.contains("def ")
                    || l.contains("pub ")
                    || l.starts_with("//")
                    || l.starts_with('#')
                    || l.contains("TODO")
                    || l.contains("FIXME")
            })
            .take(8)
            .collect();

        for line in &important_lines {
            if result.len() + line.len() + 1 > max_chars {
                break;
            }
            result.push_str(line);
            result.push('\n');
        }

        if result.len() < max_chars {
            let remaining = max_chars - result.len();
            let tail_start = text.len().saturating_sub(remaining / 2);
            let tail = &text[tail_start..];
            if let Some(newline_pos) = tail.find('\n') {
                result.push_str("...\n");
                result.push_str(&tail[newline_pos + 1..]);
            }
        }

        if result.len() > max_chars {
            result.truncate(max_chars);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_msg(name: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Tool,
            content: Some(json!(content)),
            name: Some(name.to_string()),
            tool_call_id: Some(format!("call_{name}_1")),
            ..Default::default()
        }
    }

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(json!(text)),
            ..Default::default()
        }
    }

    #[test]
    fn skips_short_content() {
        let mut compactor = CachedMicrocompactor::new(CachedMicrocompactConfig {
            threshold_chars: 100,
            recent_window: 0,
            ..Default::default()
        });
        let mut msgs = vec![tool_msg("read_file", "short content")];
        let result = compactor.compact(&mut msgs);
        assert_eq!(result.new_compressions, 0);
        assert_eq!(result.cache_hits, 0);
    }

    #[test]
    fn compresses_large_content() {
        let mut compactor = CachedMicrocompactor::new(CachedMicrocompactConfig {
            threshold_chars: 50,
            recent_window: 0,
            ..Default::default()
        });
        let big = "x".repeat(500);
        let mut msgs = vec![tool_msg("read_file", &big)];
        let result = compactor.compact(&mut msgs);
        assert_eq!(result.new_compressions, 1);
        assert!(result.tokens_freed > 0);
        let text = msgs[0].text_content().unwrap();
        assert!(text.starts_with("[cached-mc]"));
    }

    #[test]
    fn cache_hit_on_second_pass() {
        let mut compactor = CachedMicrocompactor::new(CachedMicrocompactConfig {
            threshold_chars: 50,
            recent_window: 0,
            ..Default::default()
        });
        let big = "x".repeat(500);
        let mut msgs = vec![tool_msg("read_file", &big)];

        compactor.compact(&mut msgs);

        // Reset content to original (simulating next iteration with same content)
        msgs[0].content = Some(json!(big));
        let result = compactor.compact(&mut msgs);
        assert_eq!(result.cache_hits, 1);
        assert_eq!(result.new_compressions, 0);
    }

    #[test]
    fn respects_recent_window() {
        let mut compactor = CachedMicrocompactor::new(CachedMicrocompactConfig {
            threshold_chars: 50,
            recent_window: 2,
            ..Default::default()
        });
        let big = "x".repeat(500);
        let mut msgs = vec![
            tool_msg("read_file", &big),
            tool_msg("grep", &big),
            tool_msg("read_file", &big),   // recent window
            tool_msg("list_dir", &big),    // recent window
        ];
        let result = compactor.compact(&mut msgs);
        assert_eq!(result.new_compressions, 2); // only first 2
    }

    #[test]
    fn evicts_stale_entries() {
        let mut compactor = CachedMicrocompactor::new(CachedMicrocompactConfig {
            threshold_chars: 50,
            max_age_passes: 2,
            recent_window: 0,
            ..Default::default()
        });
        let big = "x".repeat(500);
        let mut msgs = vec![tool_msg("old_tool", &big)];
        compactor.compact(&mut msgs);
        assert_eq!(compactor.cache_size(), 1);

        // Run passes without referencing that entry
        let mut empty: Vec<ChatMessage> = vec![user_msg("hi")];
        compactor.compact(&mut empty);
        compactor.compact(&mut empty);
        compactor.compact(&mut empty);

        assert_eq!(compactor.cache_size(), 0);
    }

    #[test]
    fn clear_resets_all_state() {
        let mut compactor = CachedMicrocompactor::new(CachedMicrocompactConfig {
            threshold_chars: 50,
            recent_window: 0,
            ..Default::default()
        });
        let big = "x".repeat(500);
        let mut msgs = vec![tool_msg("read_file", &big)];
        compactor.compact(&mut msgs);
        assert!(compactor.cache_size() > 0);

        compactor.clear();
        assert_eq!(compactor.cache_size(), 0);
    }
}
