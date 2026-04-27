use serde_json::json;

/// Safe config keys that UIs and tool endpoints are allowed to read.
pub const CONFIG_READABLE_KEYS: &[&str] = &[
    "gateway",
    "logging",
    "session",
    "memory",
    "models",
    "channels",
    "agents",
    "bindings",
    "workspace",
    "skills",
    "paths",
    "webSearch",
    "credentials",
    "modelRouter",
    "evolution",
    "mcpServers",
"onboarding",
    "security",
];

/// Keys that may be written through remote/interactive APIs.
pub const CONFIG_WRITABLE_KEYS: &[&str] = &[
    "logging",
    "session",
    "memory",
    "skills",
    "webSearch",
    "credentials",
    "models",
    "modelRouter",
    "evolution",
    "channels",
    "bindings",
    "mcpServers",
"onboarding",
    "security",
];

pub fn filter_config_for_read(full: &serde_json::Value) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    if let Some(obj) = full.as_object() {
        for key in CONFIG_READABLE_KEYS {
            if let Some(v) = obj.get(*key) {
                let masked = if *key == "credentials" || *key == "models" || *key == "security" {
                    mask_secret_values(v)
                } else {
                    v.clone()
                };
                result.insert(key.to_string(), masked);
            }
        }
    }
    serde_json::Value::Object(result)
}

pub fn navigate_config(val: &serde_json::Value, path: &str) -> serde_json::Value {
    let mut current = val;
    for part in path.split('.') {
        match current {
            serde_json::Value::Object(m) => {
                current = m.get(part).unwrap_or(&serde_json::Value::Null);
            }
            _ => return serde_json::Value::Null,
        }
    }
    current.clone()
}

pub fn set_nested_key(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), &'static str> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let Some(obj) = current.as_object_mut() {
                obj.insert(part.to_string(), value);
                return Ok(());
            }
            return Err("target is not an object");
        }
        if !current.get(*part).is_some_and(|v| v.is_object()) {
            if let Some(obj) = current.as_object_mut() {
                obj.insert(part.to_string(), json!({}));
            }
        }
        current = current
            .get_mut(*part)
            .ok_or("failed to access nested key while creating path")?;
    }
    Err("empty or invalid path")
}

/// Persist a single config key to the user's config file.
pub fn persist_config_key(key: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    // 获取当前配置模式（根据编译模式自动判断）
    let mode = crate::config::ConfigMode::from_flags(false, None);
    
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    let state_dir = match &mode {
        crate::config::ConfigMode::Development => home.join(".fastclaw-dev"),
        crate::config::ConfigMode::Profile(name) => home.join(format!(".fastclaw-{name}")),
        crate::config::ConfigMode::Production => home.join(".fastclaw"),
    };
    
    let cfg_path = state_dir.join("config/default.json");
    let mut cfg_value: serde_json::Value = if cfg_path.exists() {
        let text = std::fs::read_to_string(&cfg_path)?;
        json5::from_str(&text).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    set_nested_key(&mut cfg_value, key, value.clone())
        .map_err(|_| anyhow::anyhow!("failed to set nested key"))?;

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg_value)?)?;
    Ok(())
}

fn masked_secret(s: &str) -> serde_json::Value {
    let char_count = s.chars().count();
    if char_count > 8 {
        let prefix: String = s.chars().take(4).collect();
        let suffix: String = s
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<char>>()
            .into_iter()
            .rev()
            .collect();
        json!(format!("{prefix}…{suffix}"))
    } else if !s.is_empty() {
        json!("****")
    } else {
        json!(s)
    }
}

pub fn mask_secret_values(val: &serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let is_secret =
                    k == "apiKey" || k == "api_key" || k == "appSecret" || k == "app_secret";
                if is_secret {
                    if let Some(s) = v.as_str() {
                        out.insert(k.clone(), masked_secret(s));
                    } else {
                        out.insert(k.clone(), v.clone());
                    }
                } else {
                    out.insert(k.clone(), mask_secret_values(v));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(mask_secret_values).collect())
        }
        other => other.clone(),
    }
}

