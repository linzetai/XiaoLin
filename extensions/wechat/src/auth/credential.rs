use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatCredential {
    pub token: String,
    pub base_url: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub cdn_base_url: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

fn credentials_dir() -> PathBuf {
    let base = xiaolin_core::paths::resolve_state_dir().join("credentials");
    std::fs::create_dir_all(&base).ok();
    base
}

fn credential_path(account_id: &str) -> PathBuf {
    credentials_dir().join(format!("wechat-{account_id}.json"))
}

pub fn save_credential(account_id: &str, cred: &WechatCredential) -> anyhow::Result<()> {
    let path = credential_path(account_id);
    let json = serde_json::to_string_pretty(cred)?;
    let payload = xiaolin_core::credential_crypto::encrypt_credential(&json)?;
    std::fs::write(&path, payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(account_id, path = %path.display(), "saved wechat credential");
    Ok(())
}

pub fn load_credential(account_id: &str) -> Option<WechatCredential> {
    let path = credential_path(account_id);
    let data = std::fs::read_to_string(&path).ok()?;
    let json = match xiaolin_core::credential_crypto::decrypt_credential(&data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, account_id, path = %path.display(), "wechat credential decryption failed");
            if data.trim_start().starts_with('{') {
                data
            } else {
                return None;
            }
        }
    };
    serde_json::from_str(&json).ok()
}

pub fn list_credentials() -> Vec<(String, WechatCredential)> {
    let dir = credentials_dir();
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(id) = name
                .strip_prefix("wechat-")
                .and_then(|s| s.strip_suffix(".json"))
            {
                if let Some(cred) = load_credential(id) {
                    results.push((id.to_string(), cred));
                }
            }
        }
    }
    results
}

pub fn delete_credential(account_id: &str) -> bool {
    let path = credential_path(account_id);
    if path.exists() {
        std::fs::remove_file(&path).ok();
        tracing::info!(account_id, "deleted wechat credential");
        true
    } else {
        false
    }
}

/// Normalize a raw ilink_bot_id (e.g. "hex@im.bot") to a filesystem-safe key.
pub fn normalize_account_id(raw: &str) -> String {
    raw.replace(['@', '.'], "-")
}
