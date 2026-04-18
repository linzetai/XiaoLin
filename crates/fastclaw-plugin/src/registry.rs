use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::host::{LoadedPlugin, WasmHost};
use crate::manifest::PluginManifest;

/// Manages loaded plugins and provides lookup by plugin id or capability name.
pub struct PluginRegistry {
    host: WasmHost,
    plugins: HashMap<String, Arc<LoadedPlugin>>,
    /// Maps "plugin_id::capability_name" -> (plugin_id, export_name)
    capability_index: HashMap<String, (String, String)>,
}

impl PluginRegistry {
    pub fn new(host: WasmHost) -> Self {
        Self {
            host,
            plugins: HashMap::new(),
            capability_index: HashMap::new(),
        }
    }

    /// Load a plugin from a directory that contains `fastclaw.plugin.json` + `plugin.wasm`.
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<&PluginManifest> {
        let manifest_path = dir.join("fastclaw.plugin.json");
        let wasm_path = dir.join("plugin.wasm");

        let manifest_json = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest = PluginManifest::from_json(&manifest_json)?;

        let plugin = self.host.load(&wasm_path, manifest)?;
        let plugin_id = plugin.manifest.id.clone();

        let plugin = Arc::new(plugin);

        for cap in &plugin.manifest.capabilities {
            let key = format!("{}::{}", plugin_id, cap.name);
            self.capability_index
                .insert(key, (plugin_id.clone(), cap.export_name.clone()));
        }

        self.plugins.insert(plugin_id.clone(), plugin);

        Ok(&self.plugins[&plugin_id].manifest)
    }

    /// Load a plugin from raw bytes + manifest.
    pub fn load_bytes(&mut self, wasm_bytes: &[u8], manifest: PluginManifest) -> Result<String> {
        let plugin = self.host.load_bytes(wasm_bytes, manifest)?;
        let plugin_id = plugin.manifest.id.clone();

        let plugin = Arc::new(plugin);

        for cap in &plugin.manifest.capabilities {
            let key = format!("{}::{}", plugin_id, cap.name);
            self.capability_index
                .insert(key, (plugin_id.clone(), cap.export_name.clone()));
        }

        self.plugins.insert(plugin_id.clone(), plugin);
        Ok(plugin_id)
    }

    /// Unload a plugin by id.
    pub fn unload(&mut self, plugin_id: &str) -> bool {
        if let Some(plugin) = self.plugins.remove(plugin_id) {
            for cap in &plugin.manifest.capabilities {
                let key = format!("{}::{}", plugin_id, cap.name);
                self.capability_index.remove(&key);
            }
            true
        } else {
            false
        }
    }

    /// Invoke a plugin capability. `capability_key` is "plugin_id::capability_name".
    pub fn invoke(&self, capability_key: &str, input_json: &str) -> Result<String> {
        let (plugin_id, export_name) = self
            .capability_index
            .get(capability_key)
            .with_context(|| format!("capability `{capability_key}` not found"))?;

        let plugin = self
            .plugins
            .get(plugin_id)
            .with_context(|| format!("plugin `{plugin_id}` not loaded"))?;

        plugin.call(export_name, input_json)
    }

    /// Invoke by plugin id and capability name separately.
    pub fn invoke_by_name(
        &self,
        plugin_id: &str,
        capability_name: &str,
        input_json: &str,
    ) -> Result<String> {
        let key = format!("{plugin_id}::{capability_name}");
        self.invoke(&key, input_json)
    }

    pub fn get_plugin(&self, plugin_id: &str) -> Option<&Arc<LoadedPlugin>> {
        self.plugins.get(plugin_id)
    }

    pub fn list_plugins(&self) -> Vec<&PluginManifest> {
        self.plugins.values().map(|p| &p.manifest).collect()
    }

    pub fn list_capabilities(&self) -> Vec<String> {
        self.capability_index.keys().cloned().collect()
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}
