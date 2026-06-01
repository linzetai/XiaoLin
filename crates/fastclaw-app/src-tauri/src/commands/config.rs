use crate::embedded::GatewayInfo;
use crate::{AppData, GatewayStartupState};

/// Get gateway connection info for the frontend.
///
/// Awaits the watch channel until the gateway reports ready or failed,
/// with a 30-second timeout. Zero-delay compared to the old polling approach.
#[tauri::command]
pub async fn get_gateway_info(state: tauri::State<'_, AppData>) -> Result<GatewayInfo, String> {
    let mut rx = state.startup_watch.clone();

    {
        let current = rx.borrow().clone();
        match current {
            GatewayStartupState::Running { info } => return Ok(info),
            GatewayStartupState::Failed { error } => return Err(error),
            GatewayStartupState::Starting => {}
        }
    }

    let timeout = tokio::time::Duration::from_secs(30);
    match tokio::time::timeout(timeout, rx.changed()).await {
        Ok(Ok(())) => {
            let current = rx.borrow().clone();
            match current {
                GatewayStartupState::Running { info } => Ok(info),
                GatewayStartupState::Failed { error } => Err(error),
                GatewayStartupState::Starting => {
                    Err("gateway still starting after watch notification".into())
                }
            }
        }
        Ok(Err(_)) => Err("gateway startup channel closed unexpectedly".into()),
        Err(_) => Err("gateway not started after 30s. Check logs for errors.".into()),
    }
}