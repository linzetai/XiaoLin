pub mod bridge;
pub mod host;
pub mod manifest;
pub mod registry;
pub mod sandbox;
pub mod watcher;

pub use bridge::PluginTool;
pub use host::WasmHost;
pub use manifest::{discover_plugins, DiscoveredPlugin, PluginCapability, PluginManifest};
pub use registry::PluginRegistry;
pub use sandbox::SandboxConfig;
pub use watcher::{plugin_root_for_wasm_path, start_watching, PluginWatcher};
