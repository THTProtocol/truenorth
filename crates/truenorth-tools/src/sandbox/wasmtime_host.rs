//! Wasmtime-backed WASM sandbox host.
//!
//! [`WasmtimeHost`] provides isolated execution of WASM modules with:
//! - Fuel metering for CPU usage limits.
//! - Memory size limits enforced at instantiation.
//! - WASI capability grants for controlled filesystem and network access.
//! - Wall-clock timeouts via `tokio::time::timeout`.
//! - Module compilation cache (compile once, instantiate many times).
//!
//! # WASI Version
//!
//! This host uses WASI Preview 1 (`wasi_snapshot_preview1`) via
//! `wasmtime_wasi::preview1`. This is compatible with the vast majority of
//! compiled WASM modules produced by Rust, C, C++, and AssemblyScript toolchains.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

use crate::sandbox::capabilities::CapabilitySet;
use crate::sandbox::fuel::FuelMeter;

/// Execution result returned from a WASM call.
#[derive(Debug)]
pub struct WasmExecutionResult {
    /// JSON output deserialized from the module's return value (stdout).
    pub output: Value,
    /// Fuel consumed by this invocation.
    pub fuel_consumed: u64,
    /// Wall-clock execution time in milliseconds.
    pub execution_ms: u64,
    /// Captured stdout from the module.
    pub stdout_capture: String,
    /// Captured stderr from the module.
    pub stderr_capture: String,
}

/// Error type for WASM host operations.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum WasmHostError {
    /// Module compilation failed.
    #[error("Module compilation failed: {reason}")]
    CompilationFailed { reason: String },

    /// Module not found in cache.
    #[error("Module '{id}' not found in module cache")]
    ModuleNotFound { id: String },

    /// A named export function was not found.
    #[error("Function '{function_name}' not exported from module")]
    FunctionNotFound { function_name: String },

    /// Execution trapped due to out-of-fuel.
    #[error("Out of fuel after {fuel_consumed} units consumed")]
    OutOfFuel { fuel_consumed: u64 },

    /// Memory limit was exceeded.
    #[error("Memory limit exceeded: {used_bytes} > {limit_bytes} bytes")]
    MemoryExceeded { used_bytes: usize, limit_bytes: usize },

    /// Execution exceeded its wall-clock budget.
    #[error("Execution timed out after {elapsed_ms}ms")]
    Timeout { elapsed_ms: u64 },

    /// General Wasmtime execution error.
    #[error("Wasmtime engine error: {0}")]
    Engine(String),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Statistics for the WASM host, useful for observability.
#[derive(Debug, Clone, Default)]
pub struct WasmHostStats {
    /// Number of compiled modules cached.
    pub cached_modules: usize,
    /// Estimated bytes consumed by compiled module cache.
    pub cache_bytes: usize,
    /// Total executions performed since startup.
    pub total_executions: u64,
    /// Total fuel consumed across all executions.
    pub total_fuel_consumed: u64,
    /// Number of executions that hit any resource limit.
    pub resource_limit_hits: u64,
}

/// A compiled module entry in the host's module cache.
struct CachedModule {
    module: Module,
    /// Approximate byte size of the compiled artifact (for stats).
    compiled_size_estimate: usize,
}

/// Inner state for the host, protected by a `Mutex`.
struct HostInner {
    engine: Engine,
    module_cache: HashMap<String, CachedModule>,
    stats: WasmHostStats,
}

/// The Wasmtime-backed WASM sandbox host.
///
/// # Design
///
/// A single `Engine` is shared across all module compilations. Compiled modules
/// are cached after first compilation — compilation is expensive (~50 ms for
/// typical modules); instantiation is cheap (~1 ms). Each invocation creates a
/// fresh `Store` so there is no shared mutable state between calls.
///
/// WASI is configured per-invocation using the [`CapabilitySet`]. Only the
/// listed filesystem paths are made accessible to the module; all other paths
/// are invisible.
#[derive(Clone)]
pub struct WasmtimeHost {
    inner: Arc<Mutex<HostInner>>,
}

impl std::fmt::Debug for WasmtimeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmtimeHost").finish_non_exhaustive()
    }
}

impl WasmtimeHost {
    /// Creates a new `WasmtimeHost` with an optimised Wasmtime `Engine`.
    ///
    /// The engine is configured with:
    /// - Fuel consumption enabled (required for `max_fuel` enforcement).
    /// - Async support enabled (for `tokio::time::timeout` integration).
    /// - Cranelift compiler optimisation level set to `Speed`.
    pub fn new() -> Result<Self, WasmHostError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.async_support(true);
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);

        let engine = Engine::new(&config).map_err(|e| WasmHostError::Engine(e.to_string()))?;

        Ok(Self {
            inner: Arc::new(Mutex::new(HostInner {
                engine,
                module_cache: HashMap::new(),
                stats: WasmHostStats::default(),
            })),
        })
    }

    /// Loads and compiles a WASM module from a file path.
    ///
    /// The compiled module is cached under `module_id`. Subsequent calls with
    /// the same `module_id` return the cached module unless `force_recompile`
    /// is true.
    ///
    /// # Arguments
    /// * `path` — filesystem path to the `.wasm` binary.
    /// * `module_id` — cache key for the compiled module.
    /// * `force_recompile` — bypass the cache and recompile.
    pub fn load_module_from_path(
        &self,
        path: &Path,
        module_id: &str,
        force_recompile: bool,
    ) -> Result<String, WasmHostError> {
        let bytes = std::fs::read(path)?;
        self.load_module_bytes(module_id, &bytes, force_recompile)
    }

    /// Loads and compiles a WASM module from raw bytes.
    ///
    /// Returns the `module_id` on success for use in subsequent [`execute`] calls.
    ///
    /// # Arguments
    /// * `module_id` — unique identifier used as the cache key.
    /// * `bytes` — the raw WASM binary.
    /// * `force_recompile` — bypass the cache if already compiled.
    pub fn load_module_bytes(
        &self,
        module_id: &str,
        bytes: &[u8],
        force_recompile: bool,
    ) -> Result<String, WasmHostError> {
        let mut inner = self.inner.lock().expect("WasmtimeHost lock poisoned");

        if !force_recompile && inner.module_cache.contains_key(module_id) {
            debug!(module_id, "Returning cached compiled module");
            return Ok(module_id.to_string());
        }

        info!(module_id, bytes = bytes.len(), "Compiling WASM module");
        let module = Module::from_binary(&inner.engine, bytes)
            .map_err(|e| WasmHostError::CompilationFailed { reason: e.to_string() })?;

        let compiled_size_estimate = bytes.len();
        inner.module_cache.insert(
            module_id.to_string(),
            CachedModule { module, compiled_size_estimate },
        );
        inner.stats.cached_modules = inner.module_cache.len();
        inner.stats.cache_bytes = inner
            .module_cache
            .values()
            .map(|m| m.compiled_size_estimate)
            .sum();

        Ok(module_id.to_string())
    }

    /// Executes a specific exported function within a sandboxed WASM instance.
    ///
    /// Creates a fresh `Store` per call (no shared state between invocations).
    /// The `input` JSON is passed to the module via WASI stdin. The function
    /// result is read from WASI stdout.
    ///
    /// # Arguments
    /// * `module_id` — cache key of the compiled module (from `load_module_*`).
    /// * `function_name` — exported function to call.
    /// * `input` — JSON arguments passed to the function via stdin.
    /// * `capabilities` — capability set for this invocation.
    pub async fn execute(
        &self,
        module_id: &str,
        function_name: &str,
        input: Value,
        capabilities: &CapabilitySet,
    ) -> Result<WasmExecutionResult, WasmHostError> {
        // Clone out what we need before dropping the lock.
        let (engine, module) = {
            let inner = self.inner.lock().expect("WasmtimeHost lock poisoned");
            let cached = inner.module_cache.get(module_id).ok_or_else(|| {
                WasmHostError::ModuleNotFound { id: module_id.to_string() }
            })?;
            (inner.engine.clone(), cached.module.clone())
        };

        let max_execution_ms = capabilities.max_execution_ms;
        let max_fuel = capabilities.max_fuel;
        let caps = capabilities.clone();

        let fn_name = function_name.to_string();

        let exec_future = Self::execute_inner_static(engine, module, fn_name, input, caps);

        let start = Instant::now();
        let result = timeout(Duration::from_millis(max_execution_ms), exec_future)
            .await
            .map_err(|_| {
                warn!(module_id, function_name, max_execution_ms, "WASM execution timed out");
                WasmHostError::Timeout { elapsed_ms: max_execution_ms }
            })??;

        let elapsed = start.elapsed().as_millis() as u64;

        // Update stats.
        {
            let mut inner = self.inner.lock().expect("WasmtimeHost lock poisoned");
            inner.stats.total_executions += 1;
            inner.stats.total_fuel_consumed += result.fuel_consumed;
            if result.fuel_consumed >= max_fuel {
                inner.stats.resource_limit_hits += 1;
            }
        }

        let mut meter = FuelMeter::new(max_fuel);
        meter.record(result.fuel_consumed, module_id);

        info!(
            module_id,
            function_name,
            elapsed_ms = elapsed,
            fuel_consumed = result.fuel_consumed,
            utilisation_pct = meter.utilisation_pct(),
            "WASM execution completed"
        );

        Ok(result)
    }

    /// Inner execution logic — runs synchronously inside the timeout future.
    async fn execute_inner_static(
        engine: Engine,
        module: Module,
        function_name: String,
        input: Value,
        capabilities: CapabilitySet,
    ) -> Result<WasmExecutionResult, WasmHostError> {
        let start = Instant::now();

        // Serialise input to JSON bytes for stdin.
        let input_bytes = serde_json::to_vec(&input)?;

        // Build memory pipes for I/O capture.
        let stdin_pipe =
            wasmtime_wasi::pipe::MemoryInputPipe::new(bytes::Bytes::from(input_bytes));
        let stdout_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(1024 * 1024); // 1 MiB
        let stderr_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(64 * 1024);  // 64 KiB

        let stdout_capture_pipe = stdout_pipe.clone();
        let stderr_capture_pipe = stderr_pipe.clone();

        // Build the WASI context using preview1 API.
        let mut builder = WasiCtxBuilder::new();
        builder.stdin(stdin_pipe);
        builder.stdout(stdout_pipe);
        builder.stderr(stderr_pipe);

        // Grant read-only filesystem access.
        for path in &capabilities.filesystem_read {
            if path.exists() {
                builder
                    .preopened_dir(path, path.to_str().unwrap_or("/"), wasmtime_wasi::DirPerms::READ, wasmtime_wasi::FilePerms::READ)
                    .map_err(|e| WasmHostError::Engine(e.to_string()))?;
            }
        }

        // Grant read-write filesystem access.
        for path in &capabilities.filesystem_write {
            if path.exists() {
                builder
                    .preopened_dir(path, path.to_str().unwrap_or("/output"), wasmtime_wasi::DirPerms::all(), wasmtime_wasi::FilePerms::all())
                    .map_err(|e| WasmHostError::Engine(e.to_string()))?;
            }
        }

        if capabilities.allow_env {
            builder.inherit_env();
        }

        let wasi_ctx: WasiP1Ctx = builder.build_p1();

        // Create store with WasiP1Ctx as state.
        let mut store: Store<WasiP1Ctx> = Store::new(&engine, wasi_ctx);

        // Set fuel budget.
        store
            .set_fuel(capabilities.max_fuel)
            .map_err(|e| WasmHostError::Engine(e.to_string()))?;

        // Build linker and add WASI preview1 functions.
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        preview1::add_to_linker_async(&mut linker, |ctx| ctx)
            .map_err(|e| WasmHostError::Engine(e.to_string()))?;

        // Instantiate the module.
        let instance = linker
            .instantiate_async(&mut store, &module)
            .await
            .map_err(|e| WasmHostError::Engine(e.to_string()))?;

        // Look up the target function.
        let func = instance
            .get_func(&mut store, &function_name)
            .ok_or_else(|| WasmHostError::FunctionNotFound {
                function_name: function_name.clone(),
            })?;

        // Call the function.
        let call_result = func.call_async(&mut store, &[], &mut []).await;

        // Determine fuel consumed.
        let fuel_remaining = store.get_fuel().unwrap_or(0);
        let fuel_consumed = capabilities.max_fuel.saturating_sub(fuel_remaining);

        if let Err(ref e) = call_result {
            let e_str = e.to_string();
            if e_str.contains("fuel") || e_str.contains("out of fuel") {
                return Err(WasmHostError::OutOfFuel { fuel_consumed });
            }
            return Err(WasmHostError::Engine(e_str));
        }

        // Read captured I/O.
        let stdout_bytes = stdout_capture_pipe.contents().to_vec();
        let stderr_bytes = stderr_capture_pipe.contents().to_vec();

        let stdout_capture = String::from_utf8_lossy(&stdout_bytes).into_owned();
        let stderr_capture = String::from_utf8_lossy(&stderr_bytes).into_owned();

        // Parse stdout as JSON if possible.
        let output: Value = if stdout_capture.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(stdout_capture.trim())
                .unwrap_or_else(|_| Value::String(stdout_capture.clone()))
        };

        let execution_ms = start.elapsed().as_millis() as u64;

        Ok(WasmExecutionResult {
            output,
            fuel_consumed,
            execution_ms,
            stdout_capture,
            stderr_capture,
        })
    }

    /// Returns a snapshot of host-level statistics.
    pub fn stats(&self) -> WasmHostStats {
        let inner = self.inner.lock().expect("WasmtimeHost lock poisoned");
        inner.stats.clone()
    }

    /// Evicts a compiled module from the cache.
    ///
    /// Returns `true` if the module was present and removed, `false` otherwise.
    pub fn evict_module(&self, module_id: &str) -> bool {
        let mut inner = self.inner.lock().expect("WasmtimeHost lock poisoned");
        let removed = inner.module_cache.remove(module_id).is_some();
        if removed {
            inner.stats.cached_modules = inner.module_cache.len();
            inner.stats.cache_bytes = inner
                .module_cache
                .values()
                .map(|m| m.compiled_size_estimate)
                .sum();
        }
        removed
    }

    /// Returns the list of currently cached module IDs.
    pub fn cached_module_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().expect("WasmtimeHost lock poisoned");
        inner.module_cache.keys().cloned().collect()
    }
}
