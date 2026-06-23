//! Machine-bound AES-256-GCM encryption for credentials at rest.

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context, Result};

const ENCRYPTED_PREFIX: &str = "XENC:";
const NONCE_LEN: usize = 12;
const KEY_CONTEXT: &str = "xiaolin-credential-encryption";

/// JSON config keys whose string values should be encrypted on disk.
pub const SECRET_CONFIG_KEYS: &[&str] = &[
    "apiKey",
    "api_key",
    "appSecret",
    "app_secret",
    "userAccessToken",
    "user_access_token",
    "verificationToken",
    "verification_token",
    "encryptKey",
    "encrypt_key",
];

pub fn is_encrypted_value(s: &str) -> bool {
    s.starts_with(ENCRYPTED_PREFIX)
}

fn read_trimmed_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
fn read_linux_machine_id() -> Option<String> {
    read_trimmed_file("/etc/machine-id")
}

#[cfg(not(target_os = "linux"))]
fn read_linux_machine_id() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn read_macos_serial() -> Option<String> {
    use std::process::Command;
    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if !line.contains("IOPlatformSerialNumber") {
            continue;
        }
        let parts: Vec<&str> = line.split('"').collect();
        if parts.len() >= 2 {
            let serial = parts[parts.len() - 2].trim();
            if !serial.is_empty() {
                return Some(serial.to_string());
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn read_macos_serial() -> Option<String> {
    None
}

fn fallback_machine_id() -> String {
    let hostname = read_trimmed_file("/etc/hostname").unwrap_or_else(|| {
        #[cfg(unix)]
        {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown-host".to_string())
        }
        #[cfg(not(unix))]
        {
            "unknown-host".to_string()
        }
    });
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-user".to_string());
    format!("{hostname}:{username}")
}

fn machine_id() -> String {
    if let Some(id) = read_linux_machine_id() {
        return id;
    }
    if let Some(id) = read_macos_serial() {
        return id;
    }
    fallback_machine_id()
}

fn derive_key() -> [u8; 32] {
    blake3::derive_key(KEY_CONTEXT, machine_id().as_bytes())
}

/// Encrypt plaintext credentials. Output is `XENC:` + base64(nonce || ciphertext || tag).
pub fn encrypt_credential(plaintext: &str) -> Result<String> {
    let key = derive_key();
    let cipher = Aes256Gcm::new_from_slice(&key).context("invalid encryption key")?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("encryption failed: {e}"))?;
    let mut payload = nonce.to_vec();
    payload.extend_from_slice(&ciphertext);
    Ok(format!(
        "{ENCRYPTED_PREFIX}{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, payload)
    ))
}

/// Decrypt credential data. Plaintext values (no `XENC:` prefix) pass through unchanged.
pub fn decrypt_credential(data: &str) -> Result<String> {
    if !is_encrypted_value(data) {
        return Ok(data.to_string());
    }
    let b64 = data
        .strip_prefix(ENCRYPTED_PREFIX)
        .ok_or_else(|| anyhow!("missing encryption prefix"))?;
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        .context("invalid base64 ciphertext")?;
    if raw.len() <= NONCE_LEN {
        return Err(anyhow!("ciphertext too short"));
    }
    let (nonce_bytes, ciphertext) = raw.split_at(NONCE_LEN);
    let key = derive_key();
    let cipher = Aes256Gcm::new_from_slice(&key).context("invalid decryption key")?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow!("decryption failed: {e}"))?;
    String::from_utf8(plaintext).context("decrypted credential is not valid UTF-8")
}

/// Encrypt a credential value. Already-encrypted values pass through unchanged.
pub fn maybe_encrypt_credential(plaintext: &str) -> Result<String> {
    if is_encrypted_value(plaintext) {
        return Ok(plaintext.to_string());
    }
    encrypt_credential(plaintext)
}

/// Recursively decrypt secret config field values.
pub fn decrypt_config_secrets(val: &serde_json::Value) -> Result<serde_json::Value> {
    match val {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if v.is_string() && SECRET_CONFIG_KEYS.contains(&k.as_str()) {
                    if let Some(s) = v.as_str() {
                        let decrypted = decrypt_credential(s).map_err(|e| {
                            tracing::error!(key = %k, error = %e, "failed to decrypt config secret");
                            anyhow!("failed to decrypt config secret '{k}'")
                        })?;
                        out.insert(k.clone(), serde_json::Value::String(decrypted));
                        continue;
                    }
                }
                out.insert(k.clone(), decrypt_config_secrets(v)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(arr) => Ok(serde_json::Value::Array(
            arr.iter().map(decrypt_config_secrets).collect::<Result<Vec<_>>>()?,
        )),
        other => Ok(other.clone()),
    }
}

/// Recursively encrypt secret config field values (skips already-encrypted values).
pub fn encrypt_config_secrets(val: &serde_json::Value) -> Result<serde_json::Value> {
    match val {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if v.is_string() && SECRET_CONFIG_KEYS.contains(&k.as_str()) {
                    if let Some(s) = v.as_str() {
                        out.insert(
                            k.clone(),
                            serde_json::Value::String(maybe_encrypt_credential(s)?),
                        );
                        continue;
                    }
                }
                out.insert(k.clone(), encrypt_config_secrets(v)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(arr) => Ok(serde_json::Value::Array(
            arr.iter()
                .map(encrypt_config_secrets)
                .collect::<Result<Vec<_>>>()?,
        )),
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let plain = "sk-test-api-key-12345";
        let enc = encrypt_credential(plain).expect("encrypt");
        assert!(is_encrypted_value(&enc));
        let dec = decrypt_credential(&enc).expect("decrypt");
        assert_eq!(dec, plain);
    }

    #[test]
    fn plaintext_passthrough() {
        let plain = "not-encrypted-value";
        assert_eq!(decrypt_credential(plain).unwrap(), plain);
    }

    #[test]
    fn config_secret_roundtrip() {
        let val = serde_json::json!({
            "credentials": {
                "openai": { "apiKey": "sk-secret" }
            },
            "channels": {
                "feishu": {
                    "appSecret": "feishu-secret",
                    "userAccessToken": "user-token"
                }
            }
        });
        let encrypted = encrypt_config_secrets(&val).expect("encrypt");
        let enc_key = encrypted["credentials"]["openai"]["apiKey"]
            .as_str()
            .unwrap();
        assert!(is_encrypted_value(enc_key));

        let decrypted = decrypt_config_secrets(&encrypted).expect("decrypt");
        assert_eq!(
            decrypted["credentials"]["openai"]["apiKey"].as_str().unwrap(),
            "sk-secret"
        );
        assert_eq!(
            decrypted["channels"]["feishu"]["appSecret"].as_str().unwrap(),
            "feishu-secret"
        );
    }

    #[test]
    fn does_not_double_encrypt() {
        let plain = "sk-once";
        let once = encrypt_credential(plain).unwrap();
        let val = serde_json::json!({ "apiKey": once });
        let again = encrypt_config_secrets(&val).expect("encrypt");
        assert_eq!(
            again["apiKey"].as_str().unwrap(),
            once,
            "already encrypted values must not be re-encrypted"
        );
    }
}
