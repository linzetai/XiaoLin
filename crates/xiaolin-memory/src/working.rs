use std::collections::VecDeque;
use xiaolin_core::types::ChatMessage;

/// Short-term context window for the current conversation turn.
///
/// Maintains a bounded sliding window of recent messages so the LLM
/// prompt stays within token limits while preserving the most relevant
/// context. When full, the **least recently used** message (by explicit
/// [`WorkingMemory::touch`] / [`WorkingMemory::get`] or by insertion order
/// for untouched entries) is evicted.
pub struct WorkingMemory {
    /// `(message, last_access_tick)` — higher tick = more recently used.
    buffer: VecDeque<(ChatMessage, u64)>,
    max_messages: usize,
    system_prompt: Option<String>,
    next_tick: u64,
}

impl WorkingMemory {
    pub fn new(max_messages: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(max_messages),
            max_messages,
            system_prompt: None,
            next_tick: 1,
        }
    }

    fn bump_tick(&mut self) -> u64 {
        let t = self.next_tick;
        self.next_tick = self.next_tick.wrapping_add(1);
        t
    }

    fn evict_lru(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let idx = self
            .buffer
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, tick))| *tick)
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.buffer.remove(idx);
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = Some(prompt);
    }

    pub fn push(&mut self, msg: ChatMessage) {
        if self.buffer.len() >= self.max_messages {
            self.evict_lru();
        }
        let tick = self.bump_tick();
        self.buffer.push_back((msg, tick));
    }

    pub fn push_many(&mut self, msgs: impl IntoIterator<Item = ChatMessage>) {
        for m in msgs {
            self.push(m);
        }
    }

    /// Mark the message at `index` (in chronological buffer order) as most
    /// recently used. Returns a reference to that message if the index exists.
    pub fn touch(&mut self, index: usize) -> Option<&ChatMessage> {
        let tick = self.bump_tick();
        let entry = self.buffer.get_mut(index)?;
        entry.1 = tick;
        Some(&entry.0)
    }

    /// Same as [`Self::touch`] but returns `None` when out of bounds.
    pub fn get(&mut self, index: usize) -> Option<&ChatMessage> {
        self.touch(index)
    }

    /// Build the message list to send to the LLM.
    /// System prompt (if set) is always first, followed by the sliding window.
    pub fn build_prompt(&self) -> Vec<ChatMessage> {
        let mut out = Vec::with_capacity(self.buffer.len() + 1);
        if let Some(sp) = &self.system_prompt {
            out.push(ChatMessage {
                role: xiaolin_core::types::Role::System,
                content: Some(serde_json::Value::String(sp.clone())),
                ..Default::default()
            });
        }
        out.extend(self.buffer.iter().map(|(m, _)| m.clone()));
        out
    }

    /// Inject a summary of older messages as a system-level recap.
    /// Useful when the window is full — callers can summarize evicted
    /// messages and prepend the summary.
    pub fn inject_summary(&mut self, summary: &str) {
        let tick = self.bump_tick();
        let recap = ChatMessage {
            role: xiaolin_core::types::Role::System,
            content: Some(serde_json::Value::String(format!(
                "[conversation recap] {summary}"
            ))),
            ..Default::default()
        };
        self.buffer.push_front((recap, tick));
        while self.buffer.len() > self.max_messages {
            self.evict_lru();
        }
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn messages(&self) -> impl Iterator<Item = &ChatMessage> {
        self.buffer.iter().map(|(m, _)| m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_core::types::Role;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn sliding_window_eviction_lru_without_touch_matches_fifo_for_linear_push() {
        let mut wm = WorkingMemory::new(3);
        wm.push(user_msg("a"));
        wm.push(assistant_msg("b"));
        wm.push(user_msg("c"));
        assert_eq!(wm.len(), 3);

        wm.push(assistant_msg("d"));
        assert_eq!(wm.len(), 3);
        let msgs: Vec<_> = wm.messages().collect();
        assert_eq!(msgs[0].text_content().as_deref(), Some("b"));
        assert_eq!(msgs[2].text_content().as_deref(), Some("d"));
    }

    #[test]
    fn lru_eviction_order_differs_from_insertion_order_after_access() {
        let mut wm = WorkingMemory::new(3);
        wm.push(user_msg("a"));
        wm.push(user_msg("b"));
        wm.push(user_msg("c"));
        assert_eq!(wm.len(), 3);

        // Refresh "a" so it becomes MRU; "b" stays LRU among a,b,c.
        let _ = wm.get(0);

        wm.push(user_msg("d"));
        assert_eq!(wm.len(), 3);
        let contents: Vec<_> = wm
            .messages()
            .map(|m| m.text_content().unwrap_or_default())
            .collect();
        assert_eq!(
            contents,
            vec!["a".to_string(), "c".to_string(), "d".to_string(),],
            "expected LRU eviction to drop 'b', not oldest-inserted 'a'"
        );
    }

    #[test]
    fn build_prompt_with_system() {
        let mut wm = WorkingMemory::new(10);
        wm.set_system_prompt("You are helpful.".into());
        wm.push(user_msg("hi"));

        let prompt = wm.build_prompt();
        assert_eq!(prompt.len(), 2);
        assert!(matches!(prompt[0].role, Role::System));
        assert!(matches!(prompt[1].role, Role::User));
    }

    #[test]
    fn inject_summary_works() {
        let mut wm = WorkingMemory::new(3);
        wm.push(user_msg("a"));
        wm.push(assistant_msg("b"));
        wm.push(user_msg("c"));

        wm.inject_summary("earlier: user asked about Rust");
        assert_eq!(wm.len(), 3);
        let first = wm.messages().next().unwrap();
        assert!(first.text_content().unwrap().contains("conversation recap"));
    }
}
