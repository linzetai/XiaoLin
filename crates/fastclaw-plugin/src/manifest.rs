use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Plugin manifest (fastclaw.plugin.json).
///
/// Three-layer model:
/// - **Tools** = typed functions the model can invoke (execution surface)
/// - **Skills** = SKILL.md files injected into the system prompt (guidance)
/// - **Plugins** = extension packages that register channels, providers, tools, skills
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,

    /// Channel IDs this plugin registers (e.g. ["feishu", "slack"]).
    #[serde(default)]
    pub channels: Vec<String>,

    /// Model provider IDs this plugin registers.
    #[serde(default)]
    pub providers: Vec<String>,

    /// Tool names this plugin provides to agents.
    #[serde(default)]
    pub tools: Vec<String>,

    /// Relative paths to skill directories (each containing SKILL.md).
    #[serde(default)]
    pub skills: Vec<String>,

    /// WASM capabilities (legacy, for WASM-based plugins).
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,

    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,

    #[serde(default = "default_true")]
    pub enabled_by_default: bool,

    #[serde(default)]
    pub kind: Option<String>,

    /// Hex-encoded HMAC-SHA256 (or other attestation) over the plugin `.wasm` bytes; verified when
    /// [`crate::sandbox::SandboxConfig::trusted_public_keys`] is non-empty.
    #[serde(default)]
    pub signature: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A single WASM capability exported by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCapability {
    pub name: String,
    pub description: String,
    pub export_name: String,
    #[serde(default)]
    pub parameters_schema: Option<serde_json::Value>,
}

pub fn verify_plugin_wasm_signature(
    manifest: &PluginManifest,
    wasm_bytes: &[u8],
    trusted_keys_hex: &[String],
) -> anyhow::Result<()> {
    if trusted_keys_hex.is_empty() {
        return Ok(());
    }
    let Some(sig_hex) = manifest.signature.as_ref() else {
        anyhow::bail!(
            "plugin '{}' has no signature but trusted_keys are configured; \
             all plugins must be signed when signature verification is enabled",
            manifest.name
        );
    };
    let sig = hex::decode(sig_hex.trim()).map_err(|e| anyhow::anyhow!("invalid signature hex: {e}"))?;
    use constant_time_eq::constant_time_eq;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    for key_hex in trusted_keys_hex {
        let key_bytes = hex::decode(key_hex.trim())
            .map_err(|e| anyhow::anyhow!("invalid trusted key hex: {e}"))?;
        let mut mac = HmacSha256::new_from_slice(&key_bytes)
            .map_err(|e| anyhow::anyhow!("invalid HMAC key length: {e}"))?;
        mac.update(wasm_bytes);
        let expected = mac.finalize().into_bytes();
        if expected.len() == sig.len() && constant_time_eq(expected.as_slice(), &sig) {
            return Ok(());
        }
    }
    anyhow::bail!("plugin wasm signature verification failed")
}

impl PluginManifest {
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Self::from_json(&json)
    }

    pub fn is_channel_plugin(&self) -> bool {
        !self.channels.is_empty()
    }

    pub fn is_provider_plugin(&self) -> bool {
        !self.providers.is_empty()
    }
}

/// Discovered plugin on disk with its manifest and root directory.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub manifest: PluginManifest,
    pub root_dir: PathBuf,
}

impl DiscoveredPlugin {
    /// Resolve the absolute paths to skill directories for this plugin.
    pub fn skill_dirs(&self) -> Vec<PathBuf> {
        self.manifest
            .skills
            .iter()
            .map(|rel| self.root_dir.join(rel))
            .filter(|p| p.exists())
            .collect()
    }
}

/// Discover all plugins under a given extensions directory.
/// Each subdirectory with a `fastclaw.plugin.json` is treated as a plugin.
pub fn discover_plugins(extensions_dir: &Path) -> Vec<DiscoveredPlugin> {
    let mut plugins = Vec::new();

    if !extensions_dir.exists() || !extensions_dir.is_dir() {
        return plugins;
    }

    let entries = match std::fs::read_dir(extensions_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(dir = %extensions_dir.display(), error = %e, "cannot read extensions dir");
            return plugins;
        }
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        let manifest_path = dir.join("fastclaw.plugin.json");
        if !manifest_path.exists() {
            continue;
        }

        match PluginManifest::from_file(&manifest_path) {
            Ok(manifest) => {
                tracing::info!(
                    plugin_id = %manifest.id,
                    name = %manifest.name,
                    channels = ?manifest.channels,
                    skills = manifest.skills.len(),
                    tools = manifest.tools.len(),
                    "discovered plugin"
                );
                plugins.push(DiscoveredPlugin {
                    manifest,
                    root_dir: dir,
                });
            }
            Err(e) => {
                tracing::warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "failed to parse plugin manifest"
                );
            }
        }
    }

    plugins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_legacy() {
        let json = r#"{
            "id": "weather",
            "name": "Weather Plugin",
            "version": "1.0.0",
            "description": "Get weather data",
            "capabilities": [{
                "name": "get_weather",
                "description": "Get current weather for a city",
                "export_name": "get_weather",
                "parameters_schema": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                }
            }]
        }"#;

        let m = PluginManifest::from_json(json).unwrap();
        assert_eq!(m.id, "weather");
        assert_eq!(m.capabilities.len(), 1);
        assert_eq!(m.capabilities[0].export_name, "get_weather");
        assert!(m.channels.is_empty());
        assert!(m.tools.is_empty());
    }

    #[test]
    fn parse_channel_plugin_manifest() {
        let json = r#"{
            "id": "feishu",
            "name": "Feishu",
            "version": "0.1.0",
            "description": "Feishu channel plugin",
            "channels": ["feishu"],
            "skills": ["skills/feishu-bitable", "skills/feishu-calendar"],
            "tools": ["feishu_send_message"],
            "configSchema": {
                "type": "object",
                "properties": {
                    "app_id": { "type": "string" }
                }
            }
        }"#;

        let m = PluginManifest::from_json(json).unwrap();
        assert_eq!(m.id, "feishu");
        assert!(m.is_channel_plugin());
        assert!(!m.is_provider_plugin());
        assert_eq!(m.channels, vec!["feishu"]);
        assert_eq!(m.skills.len(), 2);
        assert_eq!(m.tools, vec!["feishu_send_message"]);
    }

    #[test]
    fn signed_wasm_passes_hmac_verification() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let wasm = b"\0asm\x01minimal";
        let key = b"test-hmac-key";
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(wasm);
        let sig = hex::encode(mac.finalize().into_bytes());
        let mut m = PluginManifest::from_json(
            r#"{"id":"p1","name":"P","version":"1","description":"","signature":""}"#,
        )
        .unwrap();
        m.signature = Some(sig);
        verify_plugin_wasm_signature(&m, wasm, &[hex::encode(key)]).unwrap();
    }

    #[test]
    fn tampered_wasm_fails_verification() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let wasm = b"\0asm\x01minimal";
        let key = b"test-hmac-key";
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(wasm);
        let sig = hex::encode(mac.finalize().into_bytes());
        let mut m = PluginManifest::from_json(
            r#"{"id":"p1","name":"P","version":"1","description":"","signature":""}"#,
        )
        .unwrap();
        m.signature = Some(sig);
        let err = verify_plugin_wasm_signature(&m, b"tampered-bytes", &[hex::encode(key)])
            .unwrap_err();
        assert!(err.to_string().contains("verification failed"));
    }

    #[test]
    fn verify_plugin_wasm_signature_wrong_key_fails() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let wasm = b"\0asm\x01wrong-key-case";
        let key_a = b"key-a-bytes-here!!";
        let key_b = b"key-b-different!!!";
        let mut mac = HmacSha256::new_from_slice(key_a).unwrap();
        mac.update(wasm);
        let sig = hex::encode(mac.finalize().into_bytes());
        let mut m = PluginManifest::from_json(
            r#"{"id":"p1","name":"P","version":"1","description":"","signature":""}"#,
        )
        .unwrap();
        m.signature = Some(sig);
        let err = verify_plugin_wasm_signature(&m, wasm, &[hex::encode(key_b)]).unwrap_err();
        assert!(err.to_string().contains("verification failed"));
    }

    #[test]
    fn verify_plugin_wasm_signature_invalid_hex() {
        let mut m = PluginManifest::from_json(
            r#"{"id":"p1","name":"P","version":"1","description":"","signature":""}"#,
        )
        .unwrap();
        m.signature = Some("gg".into());
        let err =
            verify_plugin_wasm_signature(&m, b"wasm", &[hex::encode(b"k")]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid signature hex"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn verify_plugin_wasm_signature_absent_rejects_when_keys_configured() {
        let mut m = PluginManifest::from_json(
            r#"{"id":"p1","name":"P","version":"1","description":"","signature":""}"#,
        )
        .unwrap();
        m.signature = None;
        let err =
            verify_plugin_wasm_signature(&m, b"any-wasm", &[hex::encode(b"trusted")]).unwrap_err();
        assert!(
            err.to_string().contains("no signature"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn verify_plugin_wasm_signature_absent_ok_without_keys() {
        let mut m = PluginManifest::from_json(
            r#"{"id":"p1","name":"P","version":"1","description":"","signature":""}"#,
        )
        .unwrap();
        m.signature = None;
        verify_plugin_wasm_signature(&m, b"any-wasm", &[]).unwrap();
    }
}
