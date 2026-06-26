use std::path::PathBuf;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::state::AppState;

/// Watches skill directories and reloads the skill registry on file changes.
pub struct SkillWatcher {
    _watcher: RecommendedWatcher,
    _task: tokio::task::JoinHandle<()>,
}

impl SkillWatcher {
    pub fn start(dirs: Vec<PathBuf>, state: AppState) -> Result<Self, String> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let is_skill_file = event.paths.iter().any(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n == "SKILL.md")
                });
                if is_skill_file {
                    let _ = tx.try_send(());
                }
            }
        })
        .map_err(|e| format!("failed to create skill watcher: {e}"))?;

        for dir in &dirs {
            if !dir.exists() {
                tracing::warn!(dir = %dir.display(), "skill directory does not exist, skipping watch");
                continue;
            }
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                tracing::warn!(dir = %dir.display(), error = %e, "failed to watch skill directory");
            } else {
                tracing::info!(dir = %dir.display(), "watching skill directory");
            }
        }

        let task = tokio::spawn(async move {
            while let Some(()) = rx.recv().await {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(300)) => break,
                        msg = rx.recv() => {
                            if msg.is_none() { return; }
                        }
                    }
                }

                if let Err(e) = state.reload_skills() {
                    tracing::warn!(error = %e, "skill watcher: failed to reload skills");
                }
                state.spawn_skill_embedding_update();
            }
        });

        Ok(Self {
            _watcher: watcher,
            _task: task,
        })
    }
}
