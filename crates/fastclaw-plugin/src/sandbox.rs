use serde::{Deserialize, Serialize};

/// Configuration for the WASM sandbox resource limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Maximum memory in bytes a plugin instance can use.
    pub max_memory_bytes: u64,
    /// Maximum execution time in milliseconds per invocation.
    pub max_execution_ms: u64,
    /// Maximum fuel (instruction count) per invocation. 0 = unlimited.
    pub max_fuel: u64,
    /// Reserved for future WASI integration (file system caps); not enforced yet.
    pub allow_fs: bool,
    /// Reserved for future WASI integration (network caps); not enforced yet.
    pub allow_net: bool,

    /// Hex-encoded symmetric keys used to verify `PluginManifest::signature` (HMAC-SHA256 over WASM bytes).
    #[serde(default)]
    pub trusted_public_keys: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_execution_ms: 10_000,           // 10s
            max_fuel: 1_000_000_000,            // ~1B instructions
            allow_fs: false,
            allow_net: false,
            trusted_public_keys: Vec::new(),
        }
    }
}
