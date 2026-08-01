//! ZeroTerm Plugin - sandboxed WASM plugin runtime (wasmtime + WASI)
//!
//! Plugins are WASI commands (`wasm32-wasip1` modules): they import
//! `wasi_snapshot_preview1`, export `_start`, and communicate with the host
//! through stdio. The host feeds call input to the plugin's WASI stdin and
//! returns whatever it writes to WASI stdout. This sidesteps any linear-memory
//! pointer ABI entirely and is genuinely sandboxed - WASI is the only surface
//! a plugin can touch.
//!
//! Execution is bounded two ways: guest memory is capped at
//! [`PluginConfig::max_memory`] (via a `ResourceLimiter`), and every call runs
//! under a fixed fuel budget, so infinite loops are trapped by the engine.

use std::path::{Path, PathBuf};

use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::runtime::with_ambient_tokio_runtime;
use wasmtime_wasi::{pipe, DirPerms, FilePerms, I32Exit, WasiCtxBuilder};

/// Fuel granted per [`Plugin::call`]. Each WASM instruction costs one unit;
/// an infinite loop burns through this and is trapped by the engine.
const DEFAULT_FUEL: u64 = 50_000_000;

/// Configuration for a single plugin instance.
pub struct PluginConfig {
    /// Human-readable name, used in error messages.
    pub name: String,
    /// Maximum guest linear memory in bytes. Defaults to 16 MiB.
    pub max_memory: usize,
    /// Maximum bytes a plugin may write to its stdout per call. Defaults to 1 MiB.
    pub max_output: usize,
    /// Read-only directory preopened into the guest at `/`. `None` (the default)
    /// gives the plugin no filesystem access at all.
    pub wasi_dir: Option<PathBuf>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            name: "plugin".to_string(),
            max_memory: 16 * 1024 * 1024,
            max_output: 1024 * 1024,
            wasi_dir: None,
        }
    }
}

#[derive(Debug)]
pub enum PluginError {
    /// The module could not be compiled, linked, or instantiated.
    Load(String, String),
    /// The plugin was trapped or exited with a nonzero status while running.
    Terminated(String, String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Load(name, msg) => write!(f, "failed to load plugin `{name}`: {msg}"),
            PluginError::Terminated(name, msg) => {
                write!(f, "plugin `{name}` was terminated: {msg}")
            }
        }
    }
}

impl std::error::Error for PluginError {}

/// Owns the shared wasmtime `Engine` and loads plugin `.wasm` files into
/// isolated [`Plugin`] instances.
pub struct PluginHost {
    engine: Engine,
}

impl PluginHost {
    pub fn new() -> PluginResult<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|e| PluginError::Load("engine".to_string(), e.to_string()))?;
        Ok(Self { engine })
    }

    /// Compiles the WASM module at `path` into a sandboxed [`Plugin`].
    ///
    /// The module must be a WASI command: it imports `wasi_snapshot_preview1`
    /// and exports `_start`. See the crate README for how to build one.
    pub fn load(&self, path: impl AsRef<Path>, config: PluginConfig) -> PluginResult<Plugin> {
        let module = Module::from_file(&self.engine, path.as_ref())
            .map_err(|e| PluginError::Load(config.name.clone(), e.to_string()))?;
        Ok(Plugin {
            name: config.name.clone(),
            config,
            engine: self.engine.clone(),
            module,
        })
    }
}

/// A compiled, sandboxed plugin module.
///
/// Each [`Plugin::call`] runs in a fresh [`Store`] with its own WASI context,
/// so no guest state survives between calls.
pub struct Plugin {
    name: String,
    config: PluginConfig,
    engine: Engine,
    module: Module,
}

impl Plugin {
    /// Runs the plugin once: `input` is written to its WASI stdin and the
    /// bytes it writes to WASI stdout are returned.
    ///
    /// Bounds enforced: guest memory <= [`PluginConfig::max_memory`],
    /// execution <= [`DEFAULT_FUEL`] (traps infinite loops), and stdout
    /// <= [`PluginConfig::max_output`] (the output pipe stops accepting bytes).
    pub fn call(&mut self, input: &[u8]) -> PluginResult<Vec<u8>> {
        let stdout = pipe::MemoryOutputPipe::new(self.config.max_output);
        let mut wasi = WasiCtxBuilder::new();
        wasi.stdin(pipe::MemoryInputPipe::new(input.to_vec()))
            .stdout(stdout.clone())
            .allow_blocking_current_thread(true);
        if let Some(dir) = &self.config.wasi_dir {
            wasi.preopened_dir(dir, "/", DirPerms::READ, FilePerms::READ)
                .map_err(|e| PluginError::Load(self.name.clone(), e.to_string()))?;
        }

        let state = PluginState {
            wasi: wasi.build_p1(),
            limits: build_limits(self.config.max_memory),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);

        let mut linker = Linker::new(&self.engine);
        preview1::add_to_linker_sync(&mut linker, |s: &mut PluginState| &mut s.wasi)
            .map_err(|e| PluginError::Load(self.name.clone(), e.to_string()))?;
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| PluginError::Load(self.name.clone(), e.to_string()))?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|e| PluginError::Load(self.name.clone(), e.to_string()))?;
        store
            .set_fuel(DEFAULT_FUEL)
            .map_err(|e| PluginError::Terminated(self.name.clone(), e.to_string()))?;

        // WASI host functions need an ambient tokio runtime when called from
        // synchronous code; this installs the crate's fallback runtime if needed.
        let result = with_ambient_tokio_runtime(|| start.call(&mut store, ()));
        match result {
            Ok(()) => {}
            Err(e) => match exit_code(&e) {
                Some(0) => {}
                Some(code) => {
                    return Err(PluginError::Terminated(
                        self.name.clone(),
                        format!("exited with status {code}"),
                    ))
                }
                None => return Err(PluginError::Terminated(self.name.clone(), e.to_string())),
            },
        }
        Ok(stdout.contents().to_vec())
    }
}

/// Walks the error source chain looking for a WASI `proc_exit` code. The engine
/// wraps the `I32Exit` error in a trap with a backtrace, so a direct downcast
/// on the top-level error misses it.
fn exit_code(error: &anyhow::Error) -> Option<i32> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());
    while let Some(err) = current {
        if let Some(exit) = err.downcast_ref::<I32Exit>() {
            return Some(exit.0);
        }
        current = err.source();
    }
    None
}

struct PluginState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

type PluginResult<T> = Result<T, PluginError>;

fn build_limits(max_memory: usize) -> StoreLimits {
    StoreLimitsBuilder::new().memory_size(max_memory).build()
}

#[cfg(test)]
mod tests;
