/// WasmHost trait — the plugin sandbox contract.
///
/// TrueNorth's security model for third-party tools: every untrusted tool
/// executes in a Wasmtime sandbox with explicit capability grants, memory limits,
/// CPU fuel metering, and wall-clock timeouts. A malicious or buggy tool cannot
/// escape the sandbox boundary regardless of what it attempts.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

/// The capability grants for a WASM module instance.
///
/// Capability-based security: a module can only access exactly what is
/// explicitly granted. No grant = no access. This is structurally different
/// from permission flags that can be accidentally set too broadly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmCapabilities {
    /// Filesystem paths the module may read from.
    pub filesystem_read: Vec<PathBuf>,
    /// Filesystem paths the module may write to.
    pub filesystem_write: Vec<PathBuf>,
    /// Allowlisted hostnames the module may make HTTP requests to.
    /// An empty list means no network access.
    pub network_allow: Vec<String>,
    /// Whether the module may read environment variables.
    pub allow_env: bool,
    /// Explicit environment variable key=value pairs the module receives.
    /// Only meaningful when `allow_env` is false (controlled injection).
    pub env_vars: HashMap<String, String>,
    /// Whether the module may spawn child processes.
    pub allow_subprocess: bool,
    /// Whether the module may access the system clock.
    pub allow_clock: bool,
    /// Whether the module may generate random numbers.
    pub allow_random: bool,
}

impl WasmCapabilities {
    /// Returns a capabilities set with zero permissions — the most restrictive.
    ///
    /// All capability grants must be explicitly added from this baseline.
    pub fn none() -> Self {
        Self {
            filesystem_read: vec![],
            filesystem_write: vec![],
            network_allow: vec![],
            allow_env: false,
            env_vars: HashMap::new(),
            allow_subprocess: false,
            allow_clock: true,  // Reading the clock is almost always safe.
            allow_random: true, // RNG is almost always safe.
        }
    }

    /// Standard sandbox for untrusted third-party tools.
    ///
    /// Grants read access to the workspace and write access to the outputs directory.
    /// Network access must be explicitly added per-tool via `network_allow`.
    pub fn sandboxed(workspace_read: PathBuf, outputs_write: PathBuf) -> Self {
        Self {
            filesystem_read: vec![workspace_read],
            filesystem_write: vec![outputs_write],
            network_allow: vec![],
            allow_env: false,
            env_vars: HashMap::new(),
            allow_subprocess: false,
            allow_clock: true,
            allow_random: true,
        }
    }
}

/// Resource limits for a WASM module instance.
///
/// Fuel metering is Wasmtime's mechanism for bounding CPU consumption.
/// Each Wasm instruction consumes a configurable amount of fuel. When the
/// tank is empty, execution traps with an out-of-fuel error — preventing
/// both infinite loops and excessive CPU usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmResourceLimits {
    /// Maximum linear memory in bytes. Default: 64 MiB.
    pub max_memory_bytes: usize,
    /// CPU fuel units. ~1 per Wasm instruction.
    /// 10_000_000 ≈ 10 million simple operations.
    pub max_fuel: u64,
    /// Wall-clock timeout for the entire module execution.
    pub max_execution_ms: u64,
    /// Maximum number of table elements (indirect function calls).
    pub max_table_elements: u32,
    /// Maximum size of the Wasm call stack in bytes.
    pub max_stack_bytes: usize,
}

impl Default for WasmResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MiB
            max_fuel: 10_000_000,
            max_execution_ms: 30_000,        // 30 seconds
            max_table_elements: 10_000,
            max_stack_bytes: 1 * 1024 * 1024, // 1 MiB
        }
    }
}

/// Complete sandbox configuration for a WASM module instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSandboxConfig {
    /// The capability grants for this instance.
    pub capabilities: WasmCapabilities,
    /// The resource limits for this instance.
    pub limits: WasmResourceLimits,
    /// The working directory for filesystem operations.
    pub working_dir: PathBuf,
}

/// A handle to a loaded WASM module, ready for instantiation and execution.
///
/// The module binary has been validated and compiled but not yet instantiated.
/// Module handles are cached — compilation is expensive (~50ms), instantiation is cheap (~1ms).
#[derive(Debug, Clone)]
pub struct WasmModuleHandle {
    /// Unique identifier for this module (used as cache key).
    pub id: String,
    /// Human-readable name of the module (from its metadata).
    pub name: String,
    /// The version string from the module's metadata.
    pub version: String,
    /// Exported functions available for invocation.
    pub exported_functions: Vec<WasmExport>,
}

/// Describes an exported function from a WASM module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmExport {
    /// The function name as exported from the WASM module.
    pub name: String,
    /// JSON Schema describing the expected input parameters.
    pub input_schema: serde_json::Value,
    /// JSON Schema describing the output.
    pub output_schema: serde_json::Value,
    /// Human-readable description of the function's purpose.
    pub description: String,
}

/// The result of executing a WASM module function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmExecutionResult {
    /// The JSON output from the WASM function.
    pub output: serde_json::Value,
    /// Fuel units consumed during execution (for monitoring and tuning).
    pub fuel_consumed: u64,
    /// Wall-clock time the execution took in milliseconds.
    pub execution_ms: u64,
    /// Captured stdout from the module (not propagated to the user).
    pub stdout_capture: String,
    /// Captured stderr from the module (not propagated to the user).
    pub stderr_capture: String,
}

/// Memory usage statistics for the WASM host engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmMemoryStats {
    /// Number of compiled modules in the cache.
    pub cached_modules: usize,
    /// Estimated bytes consumed by the compiled module cache.
    pub cache_bytes: usize,
    /// Total executions performed since startup.
    pub total_executions: u64,
    /// Total fuel consumed across all executions.
    pub total_fuel_consumed: u64,
    /// Number of executions that hit resource limits (any limit type).
    pub resource_limit_hits: u64,
    /// Number of executions that resulted in sandbox violations.
    pub sandbox_violations: u64,
}

/// Errors from WASM module loading and execution.
#[derive(Debug, Error)]
pub enum WasmError {
    /// The module binary could not be compiled.
    #[error("Module compilation failed: {reason}")]
    CompilationFailed { reason: String },

    /// The module binary failed validation (malformed or unsafe).
    #[error("Module validation failed: {reason}")]
    ValidationFailed { reason: String },

    /// No module with this ID is in the cache.
    #[error("Module not found: {id}")]
    ModuleNotFound { id: String },

    /// The requested function is not exported by the module.
    #[error("Function not found in module: {function_name}")]
    FunctionNotFound { function_name: String },

    /// The module attempted an operation not permitted by its sandbox config.
    #[error("Sandbox policy violation: {violation}")]
    SandboxViolation { violation: String },

    /// The module's execution trapped (runtime error, assertion failure, etc.).
    #[error("Execution trapped: {reason}")]
    ExecutionTrapped { reason: String },

    /// The module exhausted its fuel allocation.
    #[error("Out of fuel after {fuel_consumed} units")]
    OutOfFuel { fuel_consumed: u64 },

    /// The module exceeded its memory allocation.
    #[error("Memory limit exceeded: {used_bytes} > {limit_bytes}")]
    MemoryExceeded { used_bytes: usize, limit_bytes: usize },

    /// The module exceeded its wall-clock time limit.
    #[error("Execution timed out after {elapsed_ms}ms")]
    Timeout { elapsed_ms: u64 },

    /// Failed to serialize input to the module.
    #[error("Input serialization failed: {0}")]
    InputSerialization(#[from] serde_json::Error),

    /// An I/O error while loading the module file.
    #[error("Wasm I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// An internal Wasmtime engine error.
    #[error("Wasmtime engine error: {0}")]
    Engine(String),
}

/// The WASM plugin host — TrueNorth's sandbox runtime for third-party tools.
///
/// Design rationale: the security gap identified in Phase 1 (Section 16 of the
/// research paper) is structural — no amount of code review can make arbitrary
/// tool execution safe if execution is unbounded. The WasmHost provides the
/// structural solution: every third-party tool executes in a Wasmtime sandbox
/// with explicit capability grants, memory limits, CPU fuel metering, and
/// wall-clock timeouts. A malicious or buggy tool cannot escape the sandbox
/// boundary regardless of what it attempts.
///
/// The host maintains a module registry: compiled modules are cached after
/// first load (compilation is expensive; ~50ms for typical tool modules).
/// Each invocation creates a fresh instance from the compiled module
/// (instantiation is cheap; ~1ms). This means shared state between invocations
/// is impossible — each call is fully isolated, matching the stateless tool contract.
#[async_trait]
pub trait WasmHost: Send + Sync + std::fmt::Debug {
    /// Loads and compiles a WASM module from a file path.
    ///
    /// Compilation is performed once and the result is cached. Subsequent
    /// calls with the same path return the cached handle without recompiling.
    /// The `force_recompile` flag bypasses the cache (use after module updates).
    ///
    /// Validation includes:
    /// - Wasm binary format validation
    /// - Component Model interface compliance check
    /// - Declared capabilities matching the module's actual import requirements
    async fn load_module(
        &self,
        path: &std::path::Path,
        force_recompile: bool,
    ) -> Result<WasmModuleHandle, WasmError>;

    /// Loads a WASM module from raw bytes.
    ///
    /// Used when the module bytes come from a registry or network source
    /// rather than a local file. The `module_id` is used as the cache key.
    async fn load_module_bytes(
        &self,
        module_id: &str,
        bytes: &[u8],
    ) -> Result<WasmModuleHandle, WasmError>;

    /// Executes a specific exported function within a sandboxed WASM instance.
    ///
    /// Each call creates a fresh module instance — there is no shared mutable
    /// state between calls. The `input` JSON is serialized to the WASM
    /// Component Model's type system, the function is called, and the output
    /// is deserialized back to JSON.
    ///
    /// The `sandbox` parameter specifies the exact capabilities and resource
    /// limits for this invocation. The same module can be called with different
    /// sandboxes for different callers (e.g., a trusted internal tool gets
    /// more filesystem access than an untrusted third-party plugin).
    async fn execute(
        &self,
        module_id: &str,
        function_name: &str,
        input: serde_json::Value,
        sandbox: WasmSandboxConfig,
    ) -> Result<WasmExecutionResult, WasmError>;

    /// Returns the sandbox configuration that should be applied for a named tool.
    ///
    /// The host consults the `config.toml` `[tools.sandbox.*]` tables
    /// to build the appropriate `WasmSandboxConfig`. If no specific config
    /// exists for the tool, the default sandbox is returned.
    fn sandbox_config(&self, tool_name: &str) -> WasmSandboxConfig;

    /// Returns a list of all currently loaded module handles.
    async fn list_modules(&self) -> Vec<WasmModuleHandle>;

    /// Evicts a module from the cache, freeing compiled binary memory.
    ///
    /// The module will be recompiled on the next `load_module` call.
    async fn evict_module(&self, module_id: &str) -> Result<(), WasmError>;

    /// Returns memory usage statistics for the WASM engine.
    async fn memory_stats(&self) -> WasmMemoryStats;

    /// Returns whether a module with the given ID is in the cache.
    fn is_cached(&self, module_id: &str) -> bool;
}
