use serde::{Deserialize, Serialize};

/// Feishu brand variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LarkBrand {
    Feishu,
    Lark,
}

impl Default for LarkBrand {
    fn default() -> Self {
        Self::Feishu
    }
}

impl LarkBrand {
    pub fn base_url(&self) -> &str {
        match self {
            LarkBrand::Feishu => "https://open.feishu.cn/open-apis",
            LarkBrand::Lark => "https://open.larksuite.com/open-apis",
        }
    }
}

/// Feishu account configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LarkAccount {
    pub account_id: String,
    pub app_id: String,
    #[serde(skip_serializing)]
    pub app_secret: String,
    #[serde(default)]
    pub brand: LarkBrand,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub name: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Result of a Feishu connectivity probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuProbeResult {
    pub ok: bool,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub bot_name: Option<String>,
    #[serde(default)]
    pub bot_open_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Feishu message content types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Text,
    Image,
    Interactive,
    ShareChat,
    ShareUser,
    Audio,
    Media,
    File,
    Sticker,
    Post,
}

impl Default for MessageType {
    fn default() -> Self {
        Self::Text
    }
}

/// Chat type enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    Private,
    Group,
}

/// Raw sender information from Feishu event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSender {
    pub sender_id: SenderIdInfo,
    #[serde(default)]
    pub sender_type: Option<String>,
    #[serde(default)]
    pub tenant_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderIdInfo {
    #[serde(default)]
    pub open_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub union_id: Option<String>,
}

/// Raw message data from Feishu event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawMessage {
    pub message_id: String,
    #[serde(default)]
    pub root_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub message_type: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub chat_type: Option<String>,
    #[serde(default)]
    pub mentions: Option<Vec<MentionInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionInfo {
    pub key: String,
    pub id: MentionId,
    pub name: String,
    #[serde(default)]
    pub tenant_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionId {
    #[serde(default)]
    pub open_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub union_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_default() {
        assert_eq!(LarkBrand::default(), LarkBrand::Feishu);
    }

    #[test]
    fn brand_base_url() {
        assert!(LarkBrand::Feishu.base_url().contains("feishu.cn"));
        assert!(LarkBrand::Lark.base_url().contains("larksuite.com"));
    }

    #[test]
    fn probe_result_serde() {
        let r = FeishuProbeResult {
            ok: true,
            app_id: Some("cli_abc".into()),
            bot_name: Some("TestBot".into()),
            bot_open_id: Some("ou_123".into()),
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("cli_abc"));
    }
}
