use std::path::{Path, PathBuf};

use crate::config::PathsConfig;

/// Resolve the FastClaw state directory from config, or fall back to `~/.fastclaw`.
pub fn resolve_state_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    if let Some(p) = cfg.and_then(|c| c.state_dir.as_deref()) {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fastclaw")
}

/// Resolve the state dir using defaults only (no config).
pub fn resolve_state_dir() -> PathBuf {
    resolve_state_dir_from(None)
}

/// Resolve the config directory: `<state>/config/`
pub fn resolve_config_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    resolve_state_dir_from(cfg).join("config")
}

/// Resolve the database file path from config or default `<state>/data/sessions.db`.
pub fn resolve_db_path_from(cfg: Option<&PathsConfig>) -> PathBuf {
    if let Some(p) = cfg.and_then(|c| c.db_path.as_deref()) {
        return PathBuf::from(p);
    }
    resolve_state_dir_from(cfg).join("data").join("sessions.db")
}

pub fn resolve_db_path() -> PathBuf {
    resolve_db_path_from(None)
}

/// Resolve the WASM plugins directory from config or default.
///
/// Priority: config `pluginsDir` > local `plugins/` > `<state>/plugins/`
pub fn resolve_plugins_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    if let Some(p) = cfg.and_then(|c| c.plugins_dir.as_deref()) {
        return PathBuf::from(p);
    }
    let local = Path::new("plugins");
    if local.exists() {
        return local.to_path_buf();
    }
    resolve_state_dir_from(cfg).join("plugins")
}

pub fn resolve_plugins_dir() -> PathBuf {
    resolve_plugins_dir_from(None)
}

/// Resolve the native extensions directory from config or default.
///
/// Priority: config `extensionsDir` > local `extensions/` > `<state>/extensions/`
pub fn resolve_extensions_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    if let Some(p) = cfg.and_then(|c| c.extensions_dir.as_deref()) {
        return PathBuf::from(p);
    }
    let local = Path::new("extensions");
    if local.exists() {
        return local.to_path_buf();
    }
    resolve_state_dir_from(cfg).join("extensions")
}

pub fn resolve_extensions_dir() -> PathBuf {
    resolve_extensions_dir_from(None)
}

/// Resolve the project-level skills directory from config or default.
///
/// Priority: config `skillsDir` > local `skills/` > `<state>/skills/`
pub fn resolve_skills_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    if let Some(p) = cfg.and_then(|c| c.skills_dir.as_deref()) {
        return PathBuf::from(p);
    }
    let local = Path::new("skills");
    if local.exists() {
        return local.to_path_buf();
    }
    resolve_state_dir_from(cfg).join("skills")
}

pub fn resolve_skills_dir() -> PathBuf {
    resolve_skills_dir_from(None)
}

/// Resolve the agent configs directory from config or default.
///
/// Priority: config `agentsDir` > local `config/agents/` > `<state>/config/agents/`
pub fn resolve_agents_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    if let Some(p) = cfg.and_then(|c| c.agents_dir.as_deref()) {
        return PathBuf::from(p);
    }
    let local = Path::new("config/agents");
    if local.exists() {
        return local.to_path_buf();
    }
    resolve_state_dir_from(cfg).join("config").join("agents")
}

pub fn resolve_agents_dir() -> PathBuf {
    resolve_agents_dir_from(None)
}

/// Resolve the logs directory: `<state>/logs/`
pub fn resolve_logs_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    resolve_state_dir_from(cfg).join("logs")
}

/// Resolve the credentials directory: `<state>/credentials/`
pub fn resolve_credentials_dir_from(cfg: Option<&PathsConfig>) -> PathBuf {
    resolve_state_dir_from(cfg).join("credentials")
}

/// Ensure the `~/.fastclaw` directory structure exists.
///
/// ```text
/// ~/.fastclaw/
/// ├── config/
/// │   ├── default.json
/// │   └── agents/
/// ├── data/
/// ├── extensions/
/// ├── plugins/
/// ├── skills/
/// ├── workspace/           # default agent workspace (main)
/// │   ├── SOUL.md
/// │   ├── USER.md
/// │   ├── AGENTS.md
/// │   └── skills/          # agent-private skills
/// ├── credentials/
/// └── logs/
/// ```
pub fn ensure_state_dir() -> anyhow::Result<PathBuf> {
    let state = resolve_state_dir();
    ensure_state_dir_at(&state)?;
    Ok(state)
}

pub fn ensure_state_dir_at(state: &Path) -> anyhow::Result<()> {
    let dirs = [
        state.join("config"),
        state.join("config").join("agents"),
        state.join("data"),
        state.join("extensions"),
        state.join("plugins"),
        state.join("skills"),
        state.join("workspace"),
        state.join("credentials"),
        state.join("logs"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            tracing::debug!(dir = %dir.display(), "created directory");
        }
    }

    let cfg_file = state.join("config").join("default.json");
    if !cfg_file.exists() {
        std::fs::write(&cfg_file, "{\n  \"gateway\": {\n    \"port\": 18789\n  }\n}\n")?;
        tracing::info!(path = %cfg_file.display(), "created default config/default.json");
    }

    tracing::info!(state_dir = %state.display(), "ensured .fastclaw directory structure");
    Ok(())
}

/// Ensure the state dir using config-driven paths.
pub fn ensure_state_dir_from(cfg: Option<&PathsConfig>) -> anyhow::Result<PathBuf> {
    let state = resolve_state_dir_from(cfg);
    ensure_state_dir_at(&state)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_dir_defaults_to_home_fastclaw() {
        let dir = resolve_state_dir();
        assert!(dir.to_string_lossy().ends_with(".fastclaw"));
    }

    #[test]
    fn state_dir_respects_config() {
        let cfg = PathsConfig {
            state_dir: Some("/tmp/test-fastclaw".to_string()),
            ..Default::default()
        };
        let dir = resolve_state_dir_from(Some(&cfg));
        assert_eq!(dir, PathBuf::from("/tmp/test-fastclaw"));
    }

    #[test]
    fn db_path_under_data() {
        let db = resolve_db_path();
        assert!(db.to_string_lossy().contains(".fastclaw/data/sessions.db"));
    }

    #[test]
    fn db_path_respects_config() {
        let cfg = PathsConfig {
            db_path: Some("/custom/db.sqlite".to_string()),
            ..Default::default()
        };
        let db = resolve_db_path_from(Some(&cfg));
        assert_eq!(db, PathBuf::from("/custom/db.sqlite"));
    }

    #[test]
    fn ensure_creates_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();

        let result = ensure_state_dir_at(tmp.path());
        assert!(result.is_ok());

        assert!(tmp.path().join("config").exists());
        assert!(tmp.path().join("data").exists());
        assert!(tmp.path().join("extensions").exists());
        assert!(tmp.path().join("plugins").exists());
        assert!(tmp.path().join("skills").exists());
        assert!(tmp.path().join("workspace").exists());
        assert!(tmp.path().join("credentials").exists());
        assert!(tmp.path().join("logs").exists());
        let cfg = tmp.path().join("config").join("default.json");
        assert!(cfg.exists());
        let content = std::fs::read_to_string(cfg).unwrap();
        assert!(content.contains("\"gateway\""));
    }
}
