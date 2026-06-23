use tauri::State;
use xiaolin_network_proxy::BrowserNetworkConfig;

use crate::browser_network::BrowserNetworkState;

#[tauri::command]
pub async fn browser_get_network_config(
    state: State<'_, BrowserNetworkState>,
) -> Result<String, String> {
    state.manager().get_config_json().await
}

#[tauri::command]
pub async fn browser_save_network_config(
    state: State<'_, BrowserNetworkState>,
    config: BrowserNetworkConfig,
) -> Result<(), String> {
    state.manager().save_user_config(config).await
}

#[tauri::command]
pub async fn browser_network_confirm_resolve(
    state: State<'_, BrowserNetworkState>,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    state.manager().resolve_confirm(&request_id, approved).await
}

#[tauri::command]
pub async fn browser_webview_proxy_url(
    state: State<'_, BrowserNetworkState>,
) -> Result<Option<String>, String> {
    Ok(state.manager().webview_proxy_url().await)
}
