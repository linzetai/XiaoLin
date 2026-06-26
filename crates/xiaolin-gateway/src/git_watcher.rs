use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{broadcast, Mutex};

use xiaolin_tools_fs::git;

pub struct GitWatcher {
    _watcher: RecommendedWatcher,
    _task: tokio::task::JoinHandle<()>,
}

impl GitWatcher {
    pub fn new(
        project_id: String,
        work_dir: PathBuf,
        git_dir: PathBuf,
        ws_broadcast: broadcast::Sender<String>,
    ) -> Result<Self, String> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);

        let notify_tx = tx.clone();
        let mut watcher =
            notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {
                let _ = notify_tx.try_send(());
            })
            .map_err(|e| format!("failed to create watcher: {e}"))?;

        // Watch .git/HEAD, .git/index, .git/refs/heads/
        let head_path = git_dir.join("HEAD");
        let index_path = git_dir.join("index");
        let refs_path = git_dir.join("refs").join("heads");

        let _ = watcher.watch(&head_path, RecursiveMode::NonRecursive);
        let _ = watcher.watch(&index_path, RecursiveMode::NonRecursive);
        if refs_path.exists() {
            let _ = watcher.watch(&refs_path, RecursiveMode::Recursive);
        }

        let task = tokio::spawn(async move {
            while let Some(()) = rx.recv().await {
                // Debounce: wait 200ms for quiet period
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(200)) => break,
                        msg = rx.recv() => {
                            if msg.is_none() { return; }
                        }
                    }
                }

                match git::git_status(&work_dir).await {
                    Ok(status) => {
                        let event = serde_json::json!({
                            "type": "event",
                            "event": "git.status_changed",
                            "data": {
                                "projectId": &project_id,
                                "status": status,
                            }
                        });
                        let _ = ws_broadcast.send(event.to_string());
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "git status refresh failed");
                    }
                }
            }
        });

        Ok(Self {
            _watcher: watcher,
            _task: task,
        })
    }
}

pub struct GitWatcherManager {
    watchers: Mutex<HashMap<String, GitWatcher>>,
    ws_broadcast: broadcast::Sender<String>,
}

impl GitWatcherManager {
    pub fn new(ws_broadcast: broadcast::Sender<String>) -> Self {
        Self {
            watchers: Mutex::new(HashMap::new()),
            ws_broadcast,
        }
    }

    pub async fn ensure_watcher(&self, project_id: &str, work_dir: &Path) {
        let mut watchers = self.watchers.lock().await;
        if watchers.contains_key(project_id) {
            return;
        }

        if !git::is_git_repo(work_dir).await {
            return;
        }

        let git_dir = match git::resolve_git_dir(work_dir).await {
            Ok(d) => d,
            Err(_) => return,
        };

        match GitWatcher::new(
            project_id.to_string(),
            work_dir.to_path_buf(),
            git_dir,
            self.ws_broadcast.clone(),
        ) {
            Ok(w) => {
                watchers.insert(project_id.to_string(), w);
                tracing::debug!(project_id, "git watcher started");
            }
            Err(e) => {
                tracing::warn!(project_id, error = %e, "failed to start git watcher");
            }
        }
    }

    pub async fn stop_watcher(&self, project_id: &str) {
        let mut watchers = self.watchers.lock().await;
        if watchers.remove(project_id).is_some() {
            tracing::debug!(project_id, "git watcher stopped");
        }
    }

    pub async fn trigger_refresh(&self, project_id: &str, work_dir: &Path) {
        if !git::is_git_repo(work_dir).await {
            return;
        }
        match git::git_status(work_dir).await {
            Ok(status) => {
                let event = serde_json::json!({
                    "type": "event",
                    "event": "git.status_changed",
                    "data": {
                        "projectId": project_id,
                        "status": status,
                    }
                });
                let _ = self.ws_broadcast.send(event.to_string());
            }
            Err(e) => {
                tracing::debug!(project_id, error = %e, "trigger_refresh git status failed");
            }
        }
    }
}

pub type SharedGitWatcherManager = Arc<GitWatcherManager>;
