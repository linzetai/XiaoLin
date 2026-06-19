use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use xiaolin_core::types::ChatMessage;
use xiaolin_protocol::Role;

/// Metadata header written as the first line of a sidechain JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidechainMeta {
    pub _meta: bool,
    pub run_id: String,
    pub agent_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub started_at: u64,
}

/// A single message line in the sidechain JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidechainMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub timestamp: u64,
    pub agent_id: String,
}

impl SidechainMessage {
    pub fn to_chat_message(&self) -> ChatMessage {
        let role = match self.role.as_str() {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            _ => Role::User,
        };

        let mut msg = ChatMessage {
            role,
            content: Some(serde_json::Value::String(self.content.clone())),
            ..Default::default()
        };

        if let Some(ref tc_json) = self.tool_calls_json {
            if let Ok(tool_calls) = serde_json::from_str(tc_json) {
                msg.tool_calls = Some(tool_calls);
            }
        }

        if let Some(ref tc_id) = self.tool_call_id {
            msg.tool_call_id = Some(tc_id.clone());
        }

        msg
    }
}

/// Resolves the session filesystem directory for a given session_id.
/// Uses the same pattern as `create_tool_result_storage`.
pub fn resolve_session_dir(session_id: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".xiaolin")
        .join("sessions")
        .join(session_id)
}

/// Resolves the sidechains subdirectory for a session.
pub fn resolve_sidechains_dir(session_id: &str) -> PathBuf {
    resolve_session_dir(session_id).join("sidechains")
}

/// Resolves the JSONL file path for a specific run's sidechain.
pub fn resolve_sidechain_path(session_id: &str, run_id: &str) -> PathBuf {
    resolve_sidechains_dir(session_id).join(format!("{run_id}.jsonl"))
}

/// Appends messages to a sidechain JSONL file during sub-agent execution.
pub struct SidechainWriter {
    file: tokio::fs::File,
    path: PathBuf,
}

impl SidechainWriter {
    /// Creates the sidechains directory and file, writes the metadata header.
    pub async fn new(session_id: &str, run_id: &str, meta: SidechainMeta) -> anyhow::Result<Self> {
        let path = resolve_sidechain_path(session_id, run_id);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        let header_line = serde_json::to_string(&meta)?;
        file.write_all(header_line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;

        Ok(Self { file, path })
    }

    /// Appends a message as a JSON line.
    pub async fn append(&mut self, msg: &SidechainMessage) -> anyhow::Result<()> {
        let line = serde_json::to_string(msg)?;
        self.file.write_all(line.as_bytes()).await?;
        self.file.write_all(b"\n").await?;
        self.file.flush().await?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Reads sidechain JSONL files back into structured data.
pub struct SidechainReader;

impl SidechainReader {
    /// Loads all messages from a sidechain file (skipping the metadata header).
    pub async fn load(session_id: &str, run_id: &str) -> anyhow::Result<Vec<SidechainMessage>> {
        let path = resolve_sidechain_path(session_id, run_id);
        Self::load_from_path(&path).await
    }

    /// Loads messages from an explicit path.
    pub async fn load_from_path(path: &Path) -> anyhow::Result<Vec<SidechainMessage>> {
        let content = fs::read_to_string(path).await?;
        let mut messages = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Skip metadata header line
            if line.contains("\"_meta\"") {
                continue;
            }
            match serde_json::from_str::<SidechainMessage>(line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!(error = %e, "skipping malformed sidechain line");
                }
            }
        }

        Ok(messages)
    }

    /// Loads sidechain messages and converts them to ChatMessages for context replay.
    pub async fn load_as_chat_messages(
        session_id: &str,
        run_id: &str,
    ) -> anyhow::Result<Vec<ChatMessage>> {
        let messages = Self::load(session_id, run_id).await?;
        Ok(messages.iter().map(|m| m.to_chat_message()).collect())
    }

    /// Loads just the metadata header from a sidechain file.
    pub async fn load_meta(session_id: &str, run_id: &str) -> anyhow::Result<SidechainMeta> {
        let path = resolve_sidechain_path(session_id, run_id);
        let content = fs::read_to_string(&path).await?;
        let first_line = content
            .lines()
            .next()
            .ok_or_else(|| anyhow::anyhow!("sidechain file is empty"))?;
        let meta: SidechainMeta = serde_json::from_str(first_line)?;
        if !meta._meta {
            anyhow::bail!("first line is not a valid metadata header");
        }
        Ok(meta)
    }

    /// Checks if a sidechain file exists for the given run.
    pub async fn exists(session_id: &str, run_id: &str) -> bool {
        let path = resolve_sidechain_path(session_id, run_id);
        fs::try_exists(&path).await.unwrap_or(false)
    }
}

/// Maximum result length returned to the parent agent.
/// 32 KB allows most sub-agent outputs (including large file writes) to pass
/// through without truncation, while still bounding context window usage.
pub const MAX_RESULT_CHARS: usize = 32768;

/// Hard upper bound for `max_result_chars` to prevent context window exhaustion.
const MAX_RESULT_CHARS_CEILING: usize = 131072;

/// Truncates a sub-agent result if it exceeds the given limit.
/// Keeps the head and tail of the result for maximum context preservation.
/// Uses char boundaries to avoid panicking on multi-byte UTF-8 text.
/// `max_chars` is clamped to `MAX_RESULT_CHARS_CEILING` (128 KB).
pub fn truncate_result(text: &str, max_chars: usize) -> String {
    let limit = max_chars.min(MAX_RESULT_CHARS_CEILING);
    if text.is_empty() {
        return "[subagent terminated without producing a result]".to_string();
    }
    if text.len() <= limit {
        return text.to_string();
    }
    let head_target = limit * 3 / 4;
    let tail_target = limit / 4;
    let head_end = floor_char_boundary(text, head_target);
    let tail_start = ceil_char_boundary(text, text.len().saturating_sub(tail_target));
    let head = &text[..head_end];
    let tail = &text[tail_start..];
    let omitted = text.len() - head.len() - tail.len();
    format!(
        "{head}\n\n[... {omitted} chars omitted — sub-agent's work (file writes, edits, etc.) \
         completed successfully; only this summary is shortened. Use subagent_get for full text.]\n\n{tail}"
    )
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Cleans up all sidechain files for a session.
pub async fn cleanup_session_sidechains(session_id: &str) -> anyhow::Result<()> {
    let dir = resolve_sidechains_dir(session_id);
    if fs::try_exists(&dir).await.unwrap_or(false) {
        fs::remove_dir_all(&dir).await?;
    }
    Ok(())
}

/// Cleans up the entire session filesystem directory (sidechains + tool results).
pub async fn cleanup_session_filesystem(session_id: &str) -> anyhow::Result<()> {
    let dir = resolve_session_dir(session_id);
    if fs::try_exists(&dir).await.unwrap_or(false) {
        fs::remove_dir_all(&dir).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[tokio::test]
    async fn test_writer_reader_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let session_id = "test-session-001";
        let run_id = "run-abc-123";

        // Override home dir by using the path directly
        let sidechains_dir = temp_dir.path().join("sidechains");
        let file_path = sidechains_dir.join(format!("{run_id}.jsonl"));
        fs::create_dir_all(&sidechains_dir).await.unwrap();

        let meta = SidechainMeta {
            _meta: true,
            run_id: run_id.to_string(),
            agent_id: "explore".to_string(),
            parent_session_id: session_id.to_string(),
            task: "Find all TODO comments".to_string(),
            started_at: now_ms(),
        };

        // Write header
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .unwrap();
        let header = serde_json::to_string(&meta).unwrap();
        file.write_all(header.as_bytes()).await.unwrap();
        file.write_all(b"\n").await.unwrap();

        // Write messages
        let msgs = vec![
            SidechainMessage {
                role: "user".to_string(),
                content: "Find all TODO comments in the codebase".to_string(),
                tool_calls_json: None,
                tool_call_id: None,
                timestamp: now_ms(),
                agent_id: "explore".to_string(),
            },
            SidechainMessage {
                role: "assistant".to_string(),
                content: "I found 3 TODO comments.".to_string(),
                tool_calls_json: None,
                tool_call_id: None,
                timestamp: now_ms(),
                agent_id: "explore".to_string(),
            },
        ];

        for msg in &msgs {
            let line = serde_json::to_string(msg).unwrap();
            file.write_all(line.as_bytes()).await.unwrap();
            file.write_all(b"\n").await.unwrap();
        }
        file.flush().await.unwrap();
        drop(file);

        // Read back
        let loaded = SidechainReader::load_from_path(&file_path).await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].role, "user");
        assert_eq!(loaded[0].content, "Find all TODO comments in the codebase");
        assert_eq!(loaded[1].role, "assistant");
        assert_eq!(loaded[1].content, "I found 3 TODO comments.");
    }

    #[test]
    fn test_truncate_result_short() {
        let text = "Hello world";
        assert_eq!(truncate_result(text, MAX_RESULT_CHARS), "Hello world");
    }

    #[test]
    fn test_truncate_result_empty() {
        assert_eq!(
            truncate_result("", MAX_RESULT_CHARS),
            "[subagent terminated without producing a result]"
        );
    }

    #[test]
    fn test_truncate_result_long() {
        let text = "x".repeat(50_000);
        let result = truncate_result(&text, MAX_RESULT_CHARS);
        assert!(result.len() < 50_000);
        assert!(result.contains("chars omitted"));
        assert!(result.contains("subagent_get"));
        assert!(result.starts_with("x"));
        assert!(result.ends_with("x"));
    }

    #[test]
    fn test_sidechain_message_to_chat_message() {
        let msg = SidechainMessage {
            role: "assistant".to_string(),
            content: "Hello".to_string(),
            tool_calls_json: None,
            tool_call_id: None,
            timestamp: 1000,
            agent_id: "test".to_string(),
        };
        let chat = msg.to_chat_message();
        assert_eq!(chat.role, Role::Assistant);
        assert_eq!(
            chat.content,
            Some(serde_json::Value::String("Hello".to_string()))
        );
    }
}
