/// Tool types — the execution contract for TrueNorth tools.
///
/// Tools are the action layer of the agent loop. Every external action
/// (file reads, web searches, shell commands) goes through a registered
/// tool implementation. These types define the inputs, outputs, and
/// security model for tool execution.

use serde::{Deserialize, Serialize};

/// Permission level declaration for a tool.
///
/// The permission system is intentionally simple: three tiers, not RBAC.
/// Tools with High permission are the dangerous ones — filesystem modification,
/// shell execution, network access to arbitrary domains. The simplicity
/// makes the security model auditable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// Read-only operations with no external side effects.
    ///
    /// Examples: read a file, search memory, render Mermaid.
    /// Executes without user confirmation in any mode.
    Low,

    /// Operations with limited, reversible side effects.
    ///
    /// Examples: web search, fetch a URL, write to a designated output directory.
    /// Executes without confirmation in autonomous mode; shown in reasoning graph.
    Medium,

    /// Operations with significant or irreversible side effects.
    ///
    /// Examples: shell command execution, writing to arbitrary filesystem paths,
    /// making HTTP POST/PUT/DELETE requests, modifying system configuration.
    /// Always requires explicit user approval (in step-wise mode) or an explicit
    /// `allow_high_permission = true` flag in autonomous mode.
    High,
}

/// The outcome of a tool invocation, structured for two audiences.
///
/// The pi-ai "split result" pattern: the LLM and the UI see different views
/// of the same tool execution result. The LLM needs machine-readable structure;
/// the user interface needs human-readable display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Machine-readable output injected into the next LLM context window.
    ///
    /// Must be JSON-serializable. For large outputs, include a summary here
    /// and store the full output separately.
    pub llm_output: serde_json::Value,

    /// Human-readable display for the Visual Reasoning Layer.
    ///
    /// If None, the UI falls back to displaying `llm_output` as formatted JSON.
    /// May include markdown, structured tables, or rich text.
    pub display_output: Option<serde_json::Value>,

    /// Side effects produced by this tool call, logged for the audit trail.
    pub side_effects: Vec<SideEffect>,

    /// Wall-clock execution time in milliseconds.
    pub execution_ms: u64,
}

/// A side effect produced by a tool call.
///
/// Side effects are logged to the immutable audit trail regardless of whether
/// the overall task succeeds or fails. They are also emitted as `ReasoningEvent`
/// variants for display in the Visual Reasoning Layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SideEffect {
    /// A file was written or created.
    FileWritten { path: String, bytes: usize },
    /// A file was deleted.
    FileDeleted { path: String },
    /// An outbound HTTP request was made.
    NetworkRequest {
        method: String,
        url: String,
        status: u16,
    },
    /// A shell command was executed.
    ShellCommandExecuted { command: String, exit_code: i32 },
    /// A memory entry was written.
    MemoryWritten { scope: String, key: String },
    /// An external API endpoint was called.
    ExternalApiCalled { service: String, endpoint: String },
    /// A file was read (tracked for audit purposes when high-permission read).
    FileRead { path: String, bytes: usize },
    /// A database record was modified.
    DatabaseModified { table: String, operation: String },
}

/// All possible failures from tool execution.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ToolError {
    /// The LLM-provided arguments don't match the tool's declared JSON Schema.
    #[error("Invalid arguments for tool '{tool_name}': {message}")]
    InvalidArguments { tool_name: String, message: String },

    /// The tool requires a higher permission than the current context grants.
    #[error("Tool '{tool_name}' requires {required:?} permission; current context grants {granted:?}")]
    PermissionDenied {
        tool_name: String,
        required: PermissionLevel,
        granted: PermissionLevel,
    },

    /// The WASM sandbox rejected a resource request.
    #[error("WASM sandbox violation for tool '{tool_name}': {violation}")]
    SandboxViolation { tool_name: String, violation: String },

    /// The tool exceeded its execution time budget.
    #[error("Tool '{tool_name}' timed out after {timeout_ms}ms")]
    ExecutionTimeout { tool_name: String, timeout_ms: u64 },

    /// The tool encountered an error in its own logic.
    #[error("Tool '{tool_name}' execution failed: {message}")]
    ExecutionFailed { tool_name: String, message: String },

    /// A filesystem path traversal was attempted outside the workspace root.
    #[error("Tool '{tool_name}' attempted path traversal: {path}")]
    PathTraversal { tool_name: String, path: String },

    /// A network request to a non-allowlisted domain was attempted.
    #[error("Tool '{tool_name}' attempted unauthorized network access to: {domain}")]
    UnauthorizedNetworkAccess { tool_name: String, domain: String },
}

/// A single tool call request from the LLM.
///
/// Emitted by `LlmProvider::complete()` when the model requests a tool call.
/// Passed to `ToolRegistry::execute_sandboxed()` for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned unique ID for correlating the result back to this call.
    pub call_id: String,
    /// The name of the tool to invoke.
    pub name: String,
    /// The arguments the model provided for this call.
    pub arguments: serde_json::Value,
}

/// The JSON Schema describing a tool's input parameters.
///
/// Used by the LLM to construct valid tool calls, and by the tool
/// implementation to validate incoming arguments before execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// The tool's canonical name.
    pub name: String,
    /// Human-readable description of what this tool does and when to use it.
    pub description: String,
    /// JSON Schema object for the tool's input parameters.
    pub parameters: serde_json::Value,
    /// The permission level required to execute this tool.
    pub permission_level: PermissionLevel,
    /// Optional usage example for documentation.
    pub example: Option<String>,
}
