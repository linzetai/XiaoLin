use xiaolin_core::migration::{self, ExportOptions, ImportOptions};
use serde::{Deserialize, Serialize};

/// Maximum import blob size accepted via IPC (512 MiB).
const MAX_IMPORT_BYTES: usize = 512 * 1024 * 1024;

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

fn config_mode() -> xiaolin_core::config::ConfigMode {
    crate::resolve_config_mode()
}

/// Export all data (sessions, skills, agent workspaces) to a binary blob.
///
/// This is a local file operation - reads from local state directory.
#[tauri::command]
pub async fn export_data(options: ExportOptionsDto) -> Result<Vec<u8>, String> {
    let mode = config_mode();
    let export_options = ExportOptions::from(options);
    let data = migration::export_data(&mode, &export_options)
        .await
        .map_err(|e| format!("Failed to export data: {}", e))?;

    migration::serialize_migration_data(&data)
        .map_err(|e| format!("Failed to serialize migration data: {}", e))
}

/// Import data from a binary blob.
///
/// This is a local file operation - writes to local state directory.
#[tauri::command]
pub async fn import_data(data: Vec<u8>, options: ImportOptionsDto) -> Result<(), String> {
    if data.len() > MAX_IMPORT_BYTES {
        return Err("Import file too large (max 512MB)".to_string());
    }
    let mode = config_mode();
    let migration_data = migration::deserialize_migration_data(&data)
        .map_err(|e| format!("Failed to deserialize migration data: {}", e))?;

    let import_options = ImportOptions::from(options);

    migration::import_data(&migration_data, &mode, &import_options)
        .await
        .map_err(|e| format!("Failed to import data: {}", e))?;

    Ok(())
}