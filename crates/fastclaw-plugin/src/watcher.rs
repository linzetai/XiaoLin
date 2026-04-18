//! Filesystem watching for hot-reloading WASM plugins from a directory tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tokio::sync::RwLock;

use crate::manifest::PluginManifest;
use crate::registry::PluginRegistry;

const DEBOUNCE: Duration = Duration::from_millis(500);

/// If `path` is a `plugin.wasm` inside a plugin bundle directory, return that bundle root.
pub fn plugin_root_for_wasm_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    if !name.eq_ignore_ascii_case("plugin.wasm") {
        return None;
    }
    let parent = path.parent()?;
    if parent.join("fastclaw.plugin.json").is_file() {
        Some(parent.to_path_buf())
    } else {
        None
    }
}

async fn reload_plugin_at(registry: &RwLock<PluginRegistry>, dir: &Path) {
    let manifest_path = dir.join("fastclaw.plugin.json");
    let manifest_json = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path.display(),
                error = %e,
                "plugin hot-reload: failed to read manifest"
            );
            return;
        }
    };
    let manifest = match PluginManifest::from_json(&manifest_json) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                path = %manifest_path.display(),
                error = %e,
                "plugin hot-reload: invalid manifest"
            );
            return;
        }
    };
    let mut reg = registry.write().await;
    reg.unload(&manifest.id);
    match reg.load_from_dir(dir) {
        Ok(m) => tracing::info!(plugin_id = %m.id, path = %dir.display(), "plugin hot-reloaded"),
        Err(e) => tracing::warn!(
            path = %dir.display(),
            error = %e,
            "plugin hot-reload: load_from_dir failed"
        ),
    }
}

async fn debounced_reload_loop(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<notify::Event>,
    registry: Arc<RwLock<PluginRegistry>>,
    watched: PathBuf,
) {
    tracing::info!(dir = %watched.display(), "plugin directory watcher started");
    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut last_change: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Some(ev) => {
                        if !(ev.kind.is_create() || ev.kind.is_modify()) {
                            continue;
                        }
                        for p in &ev.paths {
                            if let Some(root) = plugin_root_for_wasm_path(p) {
                                if root.starts_with(&watched) {
                                    pending.insert(root);
                                    last_change = Some(tokio::time::Instant::now());
                                }
                            }
                        }
                    }
                    None => {
                        for r in pending.drain() {
                            reload_plugin_at(&registry, &r).await;
                        }
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                if let Some(t) = last_change {
                    if t.elapsed() >= DEBOUNCE && !pending.is_empty() {
                        let roots: Vec<PathBuf> = pending.drain().collect();
                        last_change = None;
                        for root in roots {
                            reload_plugin_at(&registry, &root).await;
                        }
                    }
                }
            }
        }
    }
}

/// Watch `plugin_dir` recursively; on `.wasm` create/modify, debounce and reload affected plugins.
///
/// Spawns a background thread that owns the [`notify`] watcher and a Tokio task that debounces
/// filesystem events and calls [`PluginRegistry::load_from_dir`].
pub fn start_watching(
    registry: Arc<RwLock<PluginRegistry>>,
    plugin_dir: PathBuf,
) -> notify::Result<()> {
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<notify::Event>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let _ = event_tx.send(ev);
        }
    })?;

    watcher.watch(&plugin_dir, RecursiveMode::Recursive)?;

    let watched = plugin_dir.clone();
    std::thread::spawn(move || {
        let _keep_alive = watcher;
        loop {
            std::thread::sleep(Duration::from_secs(86_400));
        }
    });

    let registry_task = registry.clone();
    tokio::spawn(debounced_reload_loop(event_rx, registry_task, watched));

    Ok(())
}

/// Namespaced entry point for plugin directory hot-reload (delegates to [`start_watching`]).
pub struct PluginWatcher;

impl PluginWatcher {
    /// See [`start_watching`].
    pub fn start(
        registry: Arc<RwLock<PluginRegistry>>,
        plugin_dir: PathBuf,
    ) -> notify::Result<()> {
        start_watching(registry, plugin_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plugin_root_for_wasm_path_detects_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("demo-plugin");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("fastclaw.plugin.json"), "{\"id\":\"x\",\"name\":\"x\"}").unwrap();
        let wasm = root.join("plugin.wasm");
        fs::write(&wasm, []).unwrap();

        assert_eq!(plugin_root_for_wasm_path(&wasm), Some(root));
    }

    #[test]
    fn plugin_root_for_wasm_path_ignores_non_wasm() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("readme.txt");
        fs::write(&p, "x").unwrap();
        assert!(plugin_root_for_wasm_path(&p).is_none());
    }
}
