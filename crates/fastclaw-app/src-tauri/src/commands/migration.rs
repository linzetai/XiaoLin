use crate::embedded::EmbeddedGateway;
use fastclaw_core::migration::{self, ExportOptions, ImportOptions};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptionsDto {
    pub include_sessions: bool,
    pub include_skills: bool,
    pub include_agent_workspaces: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptionsDto {
    pub merge: bool,
    pub overwrite_config: bool,
    pub overwrite_agents: bool,
    pub overwrite_sessions: bool,
    pub overwrite_skills: bool,
}

impl From<ExportOptionsDto> for ExportOptions {
    fn from(dto: ExportOptionsDto) -> Self {
        ExportOptions {
            include_sessions: dto.include_sessions,
            include_skills: dto.include_skills,
            include_agent_workspaces: dto.include_agent_workspaces,
        }
    }
}

impl From<ImportOptionsDto> for ImportOptions {
    fn from(dto: ImportOptionsDto) -> Self {
        ImportOptions {
            merge: dto.merge,
            overwrite_config: dto.overwrite_config,
            overwrite_agents: dto.overwrite_agents,
            overwrite_sessions: dto.overwrite_sessions,
            overwrite_skills: dto.overwrite_skills,
        }
    }
}

#[tauri::command]
pub async fn export_data(
    gateway: State<'_, Option<EmbeddedGateway>>,
    options: ExportOptionsDto,
) -> Result<Vec<u8>, String> {
    let _ = gateway.as_ref().as_ref().ok_or("gateway not started")?;
    // 使用默认的配置模式，从当前运行环境推断
    let mode = fastclaw_core::config::ConfigMode::from_flags(false, None);

    let export_options = ExportOptions::from(options);
    let data = migration::export_data(&mode, &export_options)
        .await
        .map_err(|e| format!("Failed to export data: {}", e))?;

    migration::serialize_migration_data(&data)
        .map_err(|e| format!("Failed to serialize migration data: {}", e))
}

#[tauri::command]
pub async fn import_data(
    gateway: State<'_, Option<EmbeddedGateway>>,
    data: Vec<u8>,
    options: ImportOptionsDto,
) -> Result<(), String> {
    let _ = gateway.as_ref().as_ref().ok_or("gateway not started")?;
    // 使用默认的配置模式，从当前运行环境推断
    let mode = fastclaw_core::config::ConfigMode::from_flags(false, None);

    let migration_data = migration::deserialize_migration_data(&data)
        .map_err(|e| format!("Failed to deserialize migration data: {}", e))?;

    let import_options = ImportOptions::from(options);

    migration::import_data(&migration_data, &mode, &import_options)
        .await
        .map_err(|e| format!("Failed to import data: {}", e))?;

    Ok(())
}
