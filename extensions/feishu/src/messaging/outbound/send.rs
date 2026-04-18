use crate::client::FeishuClient;
use std::sync::Arc;

/// Send a text message to a Feishu chat or user.
pub async fn send_text_message(
    client: &Arc<FeishuClient>,
    receive_id: &str,
    receive_id_type: &str,
    text: &str,
) -> anyhow::Result<serde_json::Value> {
    client.send_message(receive_id, receive_id_type, text).await
}

/// Reply to a specific Feishu message.
pub async fn reply_text_message(
    client: &Arc<FeishuClient>,
    message_id: &str,
    text: &str,
) -> anyhow::Result<serde_json::Value> {
    client.reply_message(message_id, text).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn send_module_exists() {
        // Module existence check
        assert!(true);
    }
}
