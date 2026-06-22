use serde_json::json;

/// Returns the JSON Schema for Feishu channel configuration.
pub fn feishu_config_json_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "app_id": {
                "type": "string",
                "description": "Feishu application ID"
            },
            "app_secret": {
                "type": "string",
                "description": "Feishu application secret"
            },
            "verification_token": {
                "type": "string",
                "description": "Webhook verification token"
            },
            "encrypt_key": {
                "type": "string",
                "description": "Webhook encrypt key for event decryption"
            },
            "agent_id": {
                "type": "string",
                "description": "XiaoLin agent ID to handle messages",
                "default": "main"
            },
            "webhook_port": {
                "type": "integer",
                "description": "Custom webhook listen port"
            },
            "connection_mode": {
                "type": "string",
                "enum": ["webhook", "websocket"],
                "description": "Connection mode for receiving events",
                "default": "webhook"
            }
        },
        "required": ["app_id", "app_secret"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_required() {
        let schema = feishu_config_json_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "app_id"));
        assert!(required.iter().any(|v| v == "app_secret"));
    }
}
