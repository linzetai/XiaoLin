use anyhow::{anyhow, Context, Result};
use std::path::Path;
use wasmtime::{
    AsContext, AsContextMut, Caller, Config, Engine, Instance, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, Trap,
};

use crate::manifest::{verify_plugin_wasm_signature, PluginManifest};
use crate::sandbox::SandboxConfig;

/// A loaded WASM plugin instance.
///
/// Uses the wasmtime "core module" API (not Component Model) for maximum
/// compatibility with existing WASM toolchains. The plugin ABI is:
///
/// - Plugin exports `malloc(size: i32) -> i32` for the host to allocate input buffers.
/// - Plugin exports `free(ptr: i32, size: i32)` for cleanup.
/// - Each capability exports `fn(ptr: i32, len: i32) -> i64` where the return
///   is `(ptr << 32) | len` packed into i64.
/// - All data exchange is JSON-encoded UTF-8 strings.
///
/// For plugins that don't follow this ABI (simpler plugins), we also support
/// a "simple" mode where the export signature is `fn() -> i32` returning a
/// pointer to a null-terminated string. The host will try the rich ABI first,
/// then fall back to simple mode.
pub struct WasmHost {
    engine: Engine,
    sandbox: SandboxConfig,
    _epoch_shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for WasmHost {
    fn drop(&mut self) {
        self._epoch_shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A loaded plugin ready for invocation.
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    module: Module,
    engine: Engine,
    sandbox: SandboxConfig,
}

impl WasmHost {
    pub fn new(sandbox: SandboxConfig) -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.wasm_memory64(false);

        let engine = Engine::new(&config)?;
        let engine_clone = engine.clone();
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        std::thread::spawn(move || {
            while !shutdown_clone.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
                engine_clone.increment_epoch();
            }
        });
        Ok(Self {
            engine,
            sandbox,
            _epoch_shutdown: shutdown,
        })
    }

    pub fn with_defaults() -> Result<Self> {
        Self::new(SandboxConfig::default())
    }

    /// Load a plugin from a .wasm file + manifest JSON.
    pub fn load(&self, wasm_path: &Path, manifest: PluginManifest) -> Result<LoadedPlugin> {
        let wasm_bytes = std::fs::read(wasm_path)
            .with_context(|| format!("failed to read WASM file: {}", wasm_path.display()))?;

        verify_plugin_wasm_signature(
            &manifest,
            &wasm_bytes,
            &self.sandbox.trusted_public_keys,
        )?;

        let module = Module::new(&self.engine, &wasm_bytes)
            .with_context(|| format!("failed to compile WASM module: {}", wasm_path.display()))?;

        Ok(LoadedPlugin {
            manifest,
            module,
            engine: self.engine.clone(),
            sandbox: self.sandbox.clone(),
        })
    }

    /// Load a plugin from raw WASM bytes + manifest.
    pub fn load_bytes(&self, wasm_bytes: &[u8], manifest: PluginManifest) -> Result<LoadedPlugin> {
        verify_plugin_wasm_signature(
            &manifest,
            wasm_bytes,
            &self.sandbox.trusted_public_keys,
        )?;

        let module = Module::new(&self.engine, wasm_bytes)?;
        Ok(LoadedPlugin {
            manifest,
            module,
            engine: self.engine.clone(),
            sandbox: self.sandbox.clone(),
        })
    }
}

fn map_interrupt_to_deadline(err: anyhow::Error, max_execution_ms: u64) -> anyhow::Error {
    if err.chain().any(|c| {
        c.downcast_ref::<Trap>()
            .is_some_and(|t| *t == Trap::Interrupt)
    }) {
        anyhow!(
            "plugin execution exceeded the configured deadline (max_execution_ms={max_execution_ms})"
        )
    } else {
        err
    }
}

impl LoadedPlugin {
    /// Invoke a capability by export name with JSON input, returns JSON output.
    pub fn call(&self, export_name: &str, input_json: &str) -> Result<String> {
        let mem_cap = self.sandbox.max_memory_bytes.min(usize::MAX as u64) as usize;
        let limits = StoreLimitsBuilder::new().memory_size(mem_cap).build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|s| s);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.sandbox.max_execution_ms);
        let fuel = if self.sandbox.max_fuel == 0 {
            u64::MAX
        } else {
            self.sandbox.max_fuel
        };
        store.set_fuel(fuel)?;

        let mut linker: Linker<StoreLimits> = Linker::new(&self.engine);

        linker.func_wrap(
            "env",
            "abort",
            |_caller: Caller<'_, StoreLimits>,
             _msg: i32,
             _file: i32,
             _line: i32,
             _col: i32|
             -> Result<()> { anyhow::bail!("guest called abort") },
        )?;

        let instance = linker.instantiate(&mut store, &self.module)?;

        let result = (|| -> Result<String> {
            if let Some(result) =
                self.try_rich_abi(&mut store, &instance, export_name, input_json)?
            {
                return Ok(result);
            }

            if let Some(result) = self.try_simple_abi(&mut store, &instance, export_name)? {
                return Ok(result);
            }

            anyhow::bail!("plugin export `{export_name}` not found or has unsupported signature")
        })();

        result.map_err(|e| map_interrupt_to_deadline(e, self.sandbox.max_execution_ms))
    }

    fn try_rich_abi(
        &self,
        store: &mut Store<StoreLimits>,
        instance: &Instance,
        export_name: &str,
        input_json: &str,
    ) -> Result<Option<String>> {
        let func =
            match instance.get_typed_func::<(i32, i32), i64>(store.as_context_mut(), export_name) {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };

        let malloc = instance
            .get_typed_func::<i32, i32>(store.as_context_mut(), "malloc")
            .context("plugin must export `malloc` for rich ABI")?;

        let memory = instance
            .get_memory(store.as_context_mut(), "memory")
            .context("plugin must export `memory`")?;

        let input_bytes = input_json.as_bytes();
        let input_len = input_bytes.len() as i32;
        let input_ptr = malloc.call(store.as_context_mut(), input_len)?;

        if input_ptr < 0 {
            anyhow::bail!(
                "plugin malloc returned negative pointer ({input_ptr}); \
                 the plugin likely failed to allocate {input_len} bytes"
            );
        }
        let mem_size = memory.data(store.as_context()).len();
        if (input_ptr as usize).saturating_add(input_len as usize) > mem_size {
            anyhow::bail!(
                "plugin malloc returned out-of-bounds pointer (ptr={input_ptr}, len={input_len}, \
                 memory_size={mem_size})"
            );
        }

        memory.data_mut(store.as_context_mut())
            [input_ptr as usize..input_ptr as usize + input_len as usize]
            .copy_from_slice(input_bytes);

        let packed = func.call(store.as_context_mut(), (input_ptr, input_len))?;
        let out_ptr = (packed >> 32) as usize;
        let out_len = (packed & 0xFFFF_FFFF) as usize;

        let data = memory.data(store.as_context());
        if out_ptr + out_len > data.len() {
            anyhow::bail!("plugin returned out-of-bounds pointer");
        }

        let output = std::str::from_utf8(&data[out_ptr..out_ptr + out_len])
            .context("plugin output is not valid UTF-8")?
            .to_string();

        Ok(Some(output))
    }

    fn try_simple_abi(
        &self,
        store: &mut Store<StoreLimits>,
        instance: &Instance,
        export_name: &str,
    ) -> Result<Option<String>> {
        let func = match instance.get_typed_func::<(), i32>(store.as_context_mut(), export_name) {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        let memory = instance
            .get_memory(store.as_context_mut(), "memory")
            .context("plugin must export `memory`")?;

        let ptr = func.call(store.as_context_mut(), ())? as usize;
        let data = memory.data(store.as_context());

        let end = data[ptr..]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(data.len() - ptr);

        let output = std::str::from_utf8(&data[ptr..ptr + end])
            .context("plugin output is not valid UTF-8")?
            .to_string();

        Ok(Some(output))
    }

    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
}
