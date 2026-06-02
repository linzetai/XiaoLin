use serde::{Deserialize, Serialize};

use xiaolin_core::config::ChannelConfig;

const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const DEFAULT_LONG_POLL_TIMEOUT_MS: u64 = 35_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatChannelConfig {
    pub enabled: bool,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub bot_agent: Option<String>,
    #[serde(default = "default_true")]
    pub typing_enabled: bool,
    #[serde(default = "default_long_poll_timeout")]
    pub long_poll_timeout_ms: u64,
    #[serde(default = "default_cdn_base_url")]
    pub cdn_base_url: String,
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

fn default_true() -> bool {
    true
}

fn default_long_poll_timeout() -> u64 {
    DEFAULT_LONG_POLL_TIMEOUT_MS
}

const DEFAULT_CDN_BASE_URL: &str = "https://filehelper.weixin.qq.com";

fn default_cdn_base_url() -> String {
    DEFAULT_CDN_BASE_URL.to_string()
}

impl Default for WechatChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_base_url(),
            bot_agent: None,
            typing_enabled: true,
            long_poll_timeout_ms: DEFAULT_LONG_POLL_TIMEOUT_MS,
            cdn_base_url: default_cdn_base_url(),
        }
    }
}

impl WechatChannelConfig {
    /// Extract a `WechatChannelConfig` from the generic `ChannelConfig`.
    /// Uses `domain` for base_url; other wechat-specific fields use defaults.
    pub fn from_channel_config(ch: &ChannelConfig) -> Option<Self> {
        if ch.enabled == Some(false) {
            return None;
        }
        Some(Self {
            enabled: ch.enabled.unwrap_or(true),
            base_url: ch.domain.clone().unwrap_or_else(default_base_url),
            bot_agent: None,
            typing_enabled: true,
            long_poll_timeout_ms: DEFAULT_LONG_POLL_TIMEOUT_MS,
            cdn_base_url: default_cdn_base_url(),
        })
    }
}
