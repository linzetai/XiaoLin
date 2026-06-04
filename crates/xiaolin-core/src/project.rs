use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Project-level configuration stored in `.xiaolin/project.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
}

/// Generate a deterministic project ID from a canonicalized root path.
/// Returns the first 16 hex characters of SHA-256(canonical_path).
pub fn generate_project_id(root_path: &Path) -> String {
    let canonical = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.to_path_buf());
    let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex::encode(&hash[..8])
}

/// Load project configuration from `.xiaolin/project.json` at the given root.
pub fn load_project_config(root_path: &Path) -> ProjectConfig {
    let config_path = root_path.join(".xiaolin").join("project.json");
    match std::fs::read_to_string(&config_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!(
                path = %config_path.display(),
                error = %e,
                "invalid .xiaolin/project.json, using defaults"
            );
            ProjectConfig::default()
        }),
        Err(_) => ProjectConfig::default(),
    }
}

/// Write project configuration to `.xiaolin/project.json` at the given root.
pub fn write_project_config(root_path: &Path, config: &ProjectConfig) -> anyhow::Result<()> {
    let xiaolin_dir = root_path.join(".xiaolin");
    std::fs::create_dir_all(&xiaolin_dir)?;
    let config_path = xiaolin_dir.join("project.json");
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(&config_path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn generate_id_is_deterministic() {
        let path = PathBuf::from("/tmp/test-project");
        let id1 = generate_project_id(&path);
        let id2 = generate_project_id(&path);
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 16);
    }

    #[test]
    fn load_missing_config_returns_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = load_project_config(tmp.path());
        assert!(config.name.is_none());
        assert!(config.description.is_none());
    }

    #[test]
    fn write_and_load_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = ProjectConfig {
            name: Some("Test Project".into()),
            description: Some("A test".into()),
            ..Default::default()
        };
        write_project_config(tmp.path(), &config).unwrap();
        let loaded = load_project_config(tmp.path());
        assert_eq!(loaded.name, Some("Test Project".into()));
        assert_eq!(loaded.description, Some("A test".into()));
    }
}
