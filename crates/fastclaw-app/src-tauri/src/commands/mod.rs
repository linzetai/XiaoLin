pub mod agent;
pub mod channel;
pub mod chat;
pub mod config;
pub mod cron;
mod helpers;
pub mod mcp;
pub mod migration;
pub mod notification;
pub mod session;
pub mod skill;

pub use agent::{
    create_agent, delete_agent, get_agent, list_agent_tools, list_agents, list_tools,
    read_identity_files, update_agent, update_agent_tools, upload_agent_avatar,
};
pub use channel::{bind_agent_channel, list_channels, reload_channel, unbind_agent_channel};
pub use chat::{cancel_chat_stream, chat_stream, get_plan_file, set_execution_mode, submit_tool_answer};
pub use config::{
    get_config, get_gateway_info, health_check, list_models, set_config, test_model_connection,
};
pub use cron::{cron_delete_job, cron_get_job, cron_list_jobs, cron_list_runs, cron_upsert_job};
pub use mcp::{add_mcp_server, get_mcp_status, reload_mcp_servers, remove_mcp_server};
pub use migration::{export_data, import_data};
pub use notification::{
    notification_clear_read, notification_delete, notification_get, notification_list,
    notification_mark_all_read, notification_mark_read, notification_unread_count,
};
pub use session::{
    create_session, delete_session, export_session_content, get_session, get_session_messages,
    list_sessions, set_session_work_dir, update_session_title,
};
pub use skill::{list_skills, refresh_skills, upload_skill};

#[cfg(test)]
mod tests {
    use super::helpers::get_state;
    use fastclaw_core::config_access::{
        filter_config_for_read, mask_secret_values, navigate_config, set_nested_key,
        CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS,
    };
    use serde_json::json;

    // ═══════════════════════════════════════════════════════════════════
    // navigate_config
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn navigate_config_single_level() {
        let cfg = json!({"gateway": {"port": 18789}});
        assert_eq!(navigate_config(&cfg, "gateway"), json!({"port": 18789}));
    }

    #[test]
    fn navigate_config_nested_path() {
        let cfg = json!({"gateway": {"port": 18789, "host": "127.0.0.1"}});
        assert_eq!(navigate_config(&cfg, "gateway.port"), json!(18789));
    }

    #[test]
    fn navigate_config_deeply_nested() {
        let cfg = json!({"a": {"b": {"c": {"d": 42}}}});
        assert_eq!(navigate_config(&cfg, "a.b.c.d"), json!(42));
    }

    #[test]
    fn navigate_config_missing_intermediate_returns_null() {
        let cfg = json!({"a": {"b": 1}});
        assert!(navigate_config(&cfg, "a.x.y").is_null());
    }

    #[test]
    fn navigate_config_missing_leaf_returns_null() {
        let cfg = json!({"gateway": {}});
        assert!(navigate_config(&cfg, "gateway.missing").is_null());
    }

    #[test]
    fn navigate_config_empty_key_returns_null() {
        let cfg = json!({"x": 1});
        let result = navigate_config(&cfg, "");
        assert!(result.is_null(), "empty key matches no object key → null");
    }

    #[test]
    fn navigate_config_non_object_root() {
        let cfg = json!("just a string");
        assert!(navigate_config(&cfg, "anything").is_null());
    }

    #[test]
    fn navigate_config_array_value() {
        let cfg = json!({"items": [1, 2, 3]});
        assert_eq!(navigate_config(&cfg, "items"), json!([1, 2, 3]));
    }

    // ═══════════════════════════════════════════════════════════════════
    // set_nested_key
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_nested_key_simple_overwrite() {
        let mut root = json!({"a": {"b": 1}});
        set_nested_key(&mut root, "a.b", json!(2)).unwrap();
        assert_eq!(root["a"]["b"], 2);
    }

    #[test]
    fn set_nested_key_creates_intermediate_objects() {
        let mut root = json!({});
        set_nested_key(&mut root, "x.y.z", json!("hello")).unwrap();
        assert_eq!(root["x"]["y"]["z"], "hello");
    }

    #[test]
    fn set_nested_key_top_level() {
        let mut root = json!({"a": 1});
        set_nested_key(&mut root, "b", json!(2)).unwrap();
        assert_eq!(root["b"], 2);
        assert_eq!(root["a"], 1);
    }

    #[test]
    fn set_nested_key_overwrites_non_object_intermediate() {
        let mut root = json!({"a": "string"});
        set_nested_key(&mut root, "a.b", json!(1)).unwrap();
        assert_eq!(root["a"]["b"], 1);
    }

    #[test]
    fn set_nested_key_preserves_siblings() {
        let mut root = json!({"a": {"b": 1, "c": 2}});
        set_nested_key(&mut root, "a.b", json!(99)).unwrap();
        assert_eq!(root["a"]["b"], 99);
        assert_eq!(root["a"]["c"], 2);
    }

    #[test]
    fn set_nested_key_array_value() {
        let mut root = json!({});
        set_nested_key(&mut root, "items", json!([1, 2, 3])).unwrap();
        assert_eq!(root["items"], json!([1, 2, 3]));
    }

    // ═══════════════════════════════════════════════════════════════════
    // mask_secret_values
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn mask_secrets_long_api_key() {
        let val = json!({"openai": {"apiKey": "sk-1234567890abcdef", "baseUrl": "https://api.openai.com/v1"}});
        let masked = mask_secret_values(&val);
        let key = masked["openai"]["apiKey"].as_str().unwrap();
        assert!(key.contains("…"), "long key should be partially masked");
        assert!(key.starts_with("sk-1"));
        assert!(key.ends_with("cdef"));
        assert_eq!(masked["openai"]["baseUrl"], "https://api.openai.com/v1");
    }

    #[test]
    fn mask_secrets_short_api_key() {
        let val = json!({"apiKey": "short"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "****");
    }

    #[test]
    fn mask_secrets_empty_key_unchanged() {
        let val = json!({"apiKey": ""});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "");
    }

    #[test]
    fn mask_secrets_app_secret_field() {
        let val = json!({"appSecret": "0123456789abcdef"});
        let masked = mask_secret_values(&val);
        let s = masked["appSecret"].as_str().unwrap();
        assert!(s.contains("…"), "appSecret should be masked");
    }

    #[test]
    fn mask_secrets_api_key_snake_case() {
        let val = json!({"api_key": "a1b2c3d4e5f6g7h8"});
        let masked = mask_secret_values(&val);
        let s = masked["api_key"].as_str().unwrap();
        assert!(s.contains("…"));
    }

    #[test]
    fn mask_secrets_non_string_value_unchanged() {
        let val = json!({"apiKey": 12345});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], 12345);
    }

    #[test]
    fn mask_secrets_nested_arrays() {
        let val = json!({"providers": [
            {"name": "openai", "apiKey": "sk-xxxxxxxxxxxxxxxx"},
            {"name": "anthropic", "apiKey": "sk-ant-yyyyyyyyyyyy"}
        ]});
        let masked = mask_secret_values(&val);
        for item in masked["providers"].as_array().unwrap() {
            let key = item["apiKey"].as_str().unwrap();
            assert!(key.contains("…") || key == "****");
        }
    }

    #[test]
    fn mask_secrets_non_secret_fields_untouched() {
        let val = json!({"name": "test", "baseUrl": "http://example.com", "model": "gpt-4"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked, val);
    }

    // ═══════════════════════════════════════════════════════════════════
    // filter_config_for_read
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn filter_config_includes_all_readable_keys() {
        let mut cfg = json!({});
        for key in CONFIG_READABLE_KEYS {
            cfg[key] = json!({"dummy": true});
        }
        cfg["dangerousInternal"] = json!("should not appear");
        let filtered = filter_config_for_read(&cfg);
        for key in CONFIG_READABLE_KEYS {
            assert!(filtered.get(key).is_some(), "should include {key}");
        }
        assert!(filtered.get("dangerousInternal").is_none());
    }

    #[test]
    fn filter_config_masks_credentials() {
        let cfg = json!({
            "credentials": {"openai": {"apiKey": "sk-1234567890abcdef"}},
            "models": {"openai": {"apiKey": "sk-9876543210fedcba"}},
            "gateway": {"port": 18789}
        });
        let filtered = filter_config_for_read(&cfg);
        let cred_key = filtered["credentials"]["openai"]["apiKey"]
            .as_str()
            .unwrap();
        assert!(cred_key.contains("…"), "credentials should be masked");
        let model_key = filtered["models"]["openai"]["apiKey"].as_str().unwrap();
        assert!(model_key.contains("…"), "models should be masked");
        assert_eq!(filtered["gateway"]["port"], 18789, "gateway not masked");
    }

    #[test]
    fn filter_config_missing_keys_omitted() {
        let cfg = json!({"gateway": {"port": 18789}});
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.get("gateway").is_some());
        assert!(filtered.get("logging").is_none());
    }

    #[test]
    fn filter_config_empty_config() {
        let cfg = json!({});
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.as_object().unwrap().is_empty());
    }

    #[test]
    fn filter_config_non_object_returns_empty() {
        let cfg = json!("not an object");
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.as_object().unwrap().is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Config key ACL
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn writable_keys_are_subset_of_readable() {
        for key in CONFIG_WRITABLE_KEYS {
            assert!(
                CONFIG_READABLE_KEYS.contains(key),
                "writable key '{key}' must also be readable"
            );
        }
    }

    #[test]
    fn readable_keys_include_expected_entries() {
        for expected in [
            "gateway",
            "logging",
            "session",
            "memory",
            "models",
            "credentials",
            "modelRouter",
            "evolution",
            "webSearch",
            "security",
        ] {
            assert!(
                CONFIG_READABLE_KEYS.contains(&expected),
                "missing readable key: {expected}"
            );
        }
    }

    #[test]
    fn writable_keys_include_expected_entries() {
        for expected in [
            "logging",
            "session",
            "memory",
            "credentials",
            "models",
            "modelRouter",
            "evolution",
            "webSearch",
            "security",
        ] {
            assert!(
                CONFIG_WRITABLE_KEYS.contains(&expected),
                "missing writable key: {expected}"
            );
        }
    }

    #[test]
    fn gateway_is_not_writable() {
        assert!(
            !CONFIG_WRITABLE_KEYS.contains(&"gateway"),
            "gateway should be read-only"
        );
    }

    #[test]
    fn security_is_readable_and_writable() {
        assert!(CONFIG_READABLE_KEYS.contains(&"security"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"security"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // get_state helper
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn get_state_none_returns_error() {
        let gw: Option<crate::embedded::EmbeddedGateway> = None;
        let result = get_state(&gw);
        assert!(result.is_err());
        match result {
            Err(msg) => assert_eq!(msg, "gateway not started"),
            Ok(_) => panic!("expected error"),
        }
    }
}
