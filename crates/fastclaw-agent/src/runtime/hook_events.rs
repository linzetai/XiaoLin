use std::time::Duration;

use fastclaw_core::types::ChatMessage;
use serde::{Deserialize, Serialize};

/// Comprehensive hook event types for the agent lifecycle.
/// These extend beyond the core `ToolHook` trait to cover agent-level events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookEvent {
    /// Fired before a tool is executed. Can block or modify the call.
    PreToolUse {
        tool_name: String,
        tool_use_id: String,
        input: serde_json::Value,
    },

    /// Fired after a tool completes execution.
    PostToolUse {
        tool_name: String,
        tool_use_id: String,
        input: serde_json::Value,
        output: serde_json::Value,
        #[serde(with = "duration_millis")]
        duration: Duration,
    },

    /// Fired when the agent loop is about to stop (normal completion).
    Stop {
        messages: Vec<ChatMessage>,
        assistant_messages: Vec<ChatMessage>,
    },

    /// Fired when a sub-agent finishes execution.
    SubagentStop {
        agent_id: String,
        messages: Vec<ChatMessage>,
    },

    /// Fired when a task (workflow step, todo item) is marked completed.
    TaskCompleted {
        task_id: String,
        task_subject: String,
    },

    /// Fired when an agent encounters a permission boundary.
    PermissionRequest {
        tool_name: String,
        resource: String,
        action: String,
    },

    /// Fired on notification events (e.g. cost threshold, context budget).
    Notification {
        level: NotificationLevel,
        message: String,
    },
}

impl HookEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::PreToolUse { .. } => "pre_tool_use",
            Self::PostToolUse { .. } => "post_tool_use",
            Self::Stop { .. } => "stop",
            Self::SubagentStop { .. } => "subagent_stop",
            Self::TaskCompleted { .. } => "task_completed",
            Self::PermissionRequest { .. } => "permission_request",
            Self::Notification { .. } => "notification",
        }
    }

    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::PreToolUse { tool_name, .. } | Self::PostToolUse { tool_name, .. } => {
                Some(tool_name)
            }
            Self::PermissionRequest { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }
}

/// Result of processing a hook event. Determines what the runtime does next.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookResult {
    /// If set, the operation is blocked and this error message is shown.
    pub blocking_error: Option<String>,
    /// If true, the agent loop stops after this event.
    pub prevent_continuation: bool,
    /// If set, replaces the tool output (only meaningful for PreToolUse/PostToolUse).
    pub modified_output: Option<serde_json::Value>,
}

impl HookResult {
    pub fn allow() -> Self {
        Self::default()
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            blocking_error: Some(reason.into()),
            prevent_continuation: false,
            modified_output: None,
        }
    }

    pub fn stop() -> Self {
        Self {
            blocking_error: None,
            prevent_continuation: true,
            modified_output: None,
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocking_error.is_some()
    }

    pub fn should_stop(&self) -> bool {
        self.prevent_continuation
    }
}

/// Severity levels for notification events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

/// Serde helper for Duration as milliseconds.
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_event_type_returns_correct_string() {
        let event = HookEvent::PreToolUse {
            tool_name: "read_file".into(),
            tool_use_id: "call_1".into(),
            input: serde_json::json!({"path": "/tmp/test"}),
        };
        assert_eq!(event.event_type(), "pre_tool_use");

        let event = HookEvent::Stop {
            messages: vec![],
            assistant_messages: vec![],
        };
        assert_eq!(event.event_type(), "stop");

        let event = HookEvent::TaskCompleted {
            task_id: "t1".into(),
            task_subject: "done".into(),
        };
        assert_eq!(event.event_type(), "task_completed");
    }

    #[test]
    fn hook_event_tool_name_extraction() {
        let event = HookEvent::PostToolUse {
            tool_name: "shell_exec".into(),
            tool_use_id: "call_2".into(),
            input: serde_json::json!({}),
            output: serde_json::json!({"exit_code": 0}),
            duration: Duration::from_millis(150),
        };
        assert_eq!(event.tool_name(), Some("shell_exec"));

        let event = HookEvent::SubagentStop {
            agent_id: "sub_1".into(),
            messages: vec![],
        };
        assert_eq!(event.tool_name(), None);
    }

    #[test]
    fn hook_result_block_sets_error() {
        let result = HookResult::block("dangerous operation");
        assert!(result.is_blocked());
        assert!(!result.should_stop());
        assert_eq!(
            result.blocking_error.as_deref(),
            Some("dangerous operation")
        );
    }

    #[test]
    fn hook_result_stop_prevents_continuation() {
        let result = HookResult::stop();
        assert!(!result.is_blocked());
        assert!(result.should_stop());
    }

    #[test]
    fn hook_event_serializes_correctly() {
        let event = HookEvent::PostToolUse {
            tool_name: "test".into(),
            tool_use_id: "id_1".into(),
            input: serde_json::json!({"key": "value"}),
            output: serde_json::json!("ok"),
            duration: Duration::from_millis(42),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"post_tool_use\""));
        assert!(json.contains("\"duration\":42"));

        let deserialized: HookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_type(), "post_tool_use");
    }
}
