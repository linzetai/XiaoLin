use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::id::TurnId;
use crate::message::{CompactTrigger, ContentPart, MessagePhase, Role};
use crate::usage::TokenUsage;

/// Model-visible conversation history item.
///
/// Unlike `AgentEvent` (which is for real-time streaming) and the old
/// `ChatMessage` (which mixes wire format with persistence), `HistoryItem`
/// is the canonical representation of what the model "sees" across turns.
///
/// Inspired by Codex's `ResponseItem` but adapted for XiaoLin's multi-agent
/// and compact-aware features.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryItem {
    /// A message from any role.
    Message {
        turn_id: TurnId,
        role: Role,
        content: Vec<ContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<MessagePhase>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },

    /// A tool invocation and its result.
    ToolUse {
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        arguments: String,
        output: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },

    /// A compaction boundary marker.
    CompactBoundary {
        turn_id: TurnId,
        trigger: CompactTrigger,
        pre_compact_tokens: usize,
        post_compact_tokens: usize,
        summary: String,
    },

    /// Usage statistics for a turn.
    TurnUsage {
        turn_id: TurnId,
        usage: TokenUsage,
    },
}

impl HistoryItem {
    pub fn turn_id(&self) -> &TurnId {
        match self {
            Self::Message { turn_id, .. }
            | Self::ToolUse { turn_id, .. }
            | Self::CompactBoundary { turn_id, .. }
            | Self::TurnUsage { turn_id, .. } => turn_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_item_message_roundtrip() {
        let item = HistoryItem::Message {
            turn_id: TurnId::new("t1"),
            role: Role::User,
            content: vec![ContentPart::Text {
                text: "hello".into(),
            }],
            phase: None,
            reasoning_content: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: HistoryItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.turn_id().as_str(), "t1");
    }

    #[test]
    fn history_item_tool_use_roundtrip() {
        let item = HistoryItem::ToolUse {
            turn_id: TurnId::new("t1"),
            call_id: "tc-1".into(),
            tool_name: "read_file".into(),
            arguments: r#"{"path":"a.txt"}"#.into(),
            output: "file content".into(),
            success: true,
            duration_ms: Some(150),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: HistoryItem = serde_json::from_str(&json).unwrap();
        if let HistoryItem::ToolUse { tool_name, .. } = back {
            assert_eq!(tool_name, "read_file");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn history_item_compact_boundary_roundtrip() {
        let item = HistoryItem::CompactBoundary {
            turn_id: TurnId::new("t1"),
            trigger: CompactTrigger::Auto,
            pre_compact_tokens: 100_000,
            post_compact_tokens: 20_000,
            summary: "Earlier conversation summarized".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: HistoryItem = serde_json::from_str(&json).unwrap();
        if let HistoryItem::CompactBoundary {
            pre_compact_tokens, ..
        } = back
        {
            assert_eq!(pre_compact_tokens, 100_000);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn history_item_tagged_serde() {
        let item = HistoryItem::TurnUsage {
            turn_id: TurnId::new("t1"),
            usage: TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
            },
        };
        let val = serde_json::to_value(&item).unwrap();
        assert_eq!(val["type"], "turn_usage");
    }
}
