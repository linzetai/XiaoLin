use serde::{Deserialize, Serialize};

/// User-level OAuth material for Feishu/Lark **user-scoped** Open APIs (tasks, bitable, docx, calendar, IM media).
///
/// Configure via the main channel JSON as `userAccessToken` (see [`crate::plugin::FeishuPluginConfig`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConfig {
    /// User access token from the OAuth 2.0 authorization code flow (not the tenant/app token).
    #[serde(default)]
    pub user_access_token: Option<String>,
}

impl OAuthConfig {
    pub fn is_configured(&self) -> bool {
        self.user_access_token
            .as_ref()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false)
    }

    /// Human-oriented guidance when user-scoped tools are invoked without a token.
    pub fn missing_user_token_message() -> &'static str {
        "Feishu user OAuth is not configured. Add a non-empty `userAccessToken` to the `feishu` channel in your FastClaw config (token from the Lark/Feishu OAuth 2.0 user authorization flow with scopes for task, bitable, docx, calendar, and IM file access). Tenant (app) credentials alone are not sufficient for these user-scoped APIs."
    }
}
