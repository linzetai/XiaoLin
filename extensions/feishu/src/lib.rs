mod client;
mod oauth;
mod plugin;
mod webhook;

pub mod channel;
pub mod commands;
pub mod core;
pub mod messaging;
pub mod tools;
pub mod ws;

pub use channel::{FeishuPlugin, FeishuPluginConfig};
#[deprecated(note = "Use FeishuPlugin / FeishuPluginConfig instead")]
pub use channel::{FeishuChannel, FeishuChannelConfig};
pub use client::FeishuClient;
pub use oauth::OAuthConfig;
#[deprecated(note = "Use FeishuPlugin::llm_tools() instead")]
pub use tools::{FeishuGetChatMessagesTool, FeishuReplyMessageTool, FeishuSendMessageTool};
#[deprecated(note = "Use FeishuPlugin as ChannelPlugin instead")]
pub use webhook::{feishu_webhook_handler, FeishuWebhookConfig};
