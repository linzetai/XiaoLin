use std::path::PathBuf;
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use xiaolin_agent::subagent_manager::SubAgentManager;

/// Watches agent definition directories and reloads definitions on file changes.
pub struct AgentDefWatcher {
    _watcher: RecommendedWatcher,
    _task: tokio::task::JoinHandle<()>,
}

impl AgentDefWatcher {
    /// Start watching the given directories for `.md` and `.json` agent definition changes.
    /// On any change, reloads all definitions and updates the `SubAgentManager`.
    pub fn start(
        dirs: Vec<PathBuf>,
        manager: Arc<SubAgentManager>,
    ) -> Result<Self, String> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);

        let notify_tx = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let dominated_by_agent_file = event.paths.iter().any(|p| {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    ext == "md" || ext == "markdown" || ext == "json"
                });
                if dominated_by_agent_file {
                    let _ = notify_tx.try_send(());
                }
            }
        })
        .map_err(|e| format!("failed to create agent-def watcher: {e}"))?;

        for dir in &dirs {
            if !dir.exists() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    tracing::warn!(dir = %dir.display(), error = %e, "failed to create agent def directory");
                    continue;
                }
            }
            if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                tracing::warn!(dir = %dir.display(), error = %e, "failed to watch agent def directory");
            } else {
                tracing::info!(dir = %dir.display(), "watching agent def directory");
            }
        }

        let reload_dirs = dirs.clone();
        let task = tokio::spawn(async move {
            while let Some(()) = rx.recv().await {
                // Debounce: wait 300ms for quiet period
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(300)) => break,
                        msg = rx.recv() => {
                            if msg.is_none() { return; }
                        }
                    }
                }

                reload_agent_defs(&reload_dirs, &manager);
            }
        });

        Ok(Self {
            _watcher: watcher,
            _task: task,
        })
    }
}

/// Reload all agent definitions from the given directories and apply to the manager.
fn reload_agent_defs(dirs: &[PathBuf], manager: &SubAgentManager) {
    use xiaolin_core::agent_config::{builtin_subagent_defs, load_subagent_defs_json, load_subagent_defs_markdown};
    use xiaolin_core::agent_markdown::merge_subagent_defs;

    let builtin = builtin_subagent_defs();
    let mut layers = vec![builtin];

    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        match load_subagent_defs_json(dir) {
            Ok(defs) if !defs.is_empty() => layers.push(defs),
            _ => {}
        }
        match load_subagent_defs_markdown(dir) {
            Ok(defs) if !defs.is_empty() => layers.push(defs),
            _ => {}
        }
    }

    let merged = merge_subagent_defs(layers);
    let count = merged.len();
    manager.set_subagent_defs(merged);
    tracing::info!(count, "hot-reloaded sub-agent definitions");
}
