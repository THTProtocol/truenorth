/// Tool and ToolRegistry traits — the tool execution contract.
///
/// Tools are the action layer of the agent loop. Every external action goes
/// through a registered tool implementation. The registry is the single point
/// through which ALL tool execution flows, enforcing permission policy and
/// WASM sandboxing transparently.

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::types::tool::{PermissionLevel, ToolCall, ToolError, ToolResult};

/// The invocation context passed to `Tool::execute`.
///
/// Contains all information the tool needs to execute safely without
/// requiring direct access to the full orchestrator graph.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Unique identifier for this specific invocation (for audit logging).
    pub invocation_id: Uuid,
    /// The session this tool call belongs to.
    pub session_id: Uuid,
    /// The task this tool call is part of (None for ad-hoc calls).
    pub task_id: Option<Uuid>,
    /// The plan step this tool call is part of (None for ad-hoc calls).
    pub step_id: Option<Uuid>,
    /// Maximum permission level granted to this tool call by the orchestrator.
    pub granted_permission: PermissionLevel,
    /// Workspace root path: all file operations must stay within this tree.
    pub workspace_root: std::path::PathBuf,
    /// Whether this is a dry-run (log side effects but don't execute them).
    pub dry_run: bool,
}

/// A lightweight tool summary for LLM tool definition injection.
///
/// Contains exactly what the LLM needs to construct valid tool calls,
/// without exposing implementation details.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinitionSummary {
    /// The tool's canonical name.
    pub name: String,
    /// Human-readable description of what this tool does and when to use it.
    pub description: String,
    /// JSON Schema object describing the tool's input parameters.
    pub parameters: serde_json::Value,
    /// The permission level required to execute this tool.
    pub permission_level: PermissionLevel,
}

/// Errors from the tool registry itself (distinct from tool execution errors).
#[derive(Debug, Error)]
pub enum RegistryError {
    /// A tool with this name is already registered.
    #[error("Tool '{name}' is already registered")]
    DuplicateTool { name: String },

    /// No tool with this name is registered.
    #[error("Tool '{name}' not found in registry")]
    ToolNotFound { name: String },

    /// Failed to discover tools from an MCP server.
    #[error("Failed to discover MCP tools from {server_url}: {message}")]
    McpDiscoveryFailed { server_url: String, message: String },

    /// Failed to register a WASM tool module.
    #[error("Failed to register WASM tool '{name}': {message}")]
    WasmRegistrationFailed { name: String, message: String },
}

/// The core tool trait. Every tool — built-in or WASM — implements this interface.
///
/// Design rationale: tools are the action layer of the agentic loop.
/// The `Tool` trait enforces that every tool is: self-describing (schema),
/// permission-declaring (permission_level), and safely executable (execute with context).
/// The WASM host implements this trait for compiled plugins, adapting the
/// WASM ABI to the Rust trait boundary transparently.
#[async_trait]
pub trait Tool: Send + Sync + std::fmt::Debug {
    /// The canonical tool name as it appears in LLM tool call requests.
    ///
    /// Must be stable across versions — changing this is a breaking change.
    /// Convention: snake_case, action-first (e.g., `read_file`, `search_web`).
    fn name(&self) -> &str;

    /// Human-readable description of what this tool does.
    ///
    /// Shown to the LLM as the tool description in the system prompt.
    /// Good descriptions significantly improve LLM tool selection accuracy.
    /// Format: "Verb phrase describing what the tool does and when to use it."
    fn description(&self) -> &str;

    /// JSON Schema object describing the tool's input parameters.
    ///
    /// The LLM uses this to construct valid arguments.
    /// The tool validates incoming arguments against this schema before execution.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Declares the permission level required to execute this tool.
    ///
    /// The ToolRegistry uses this to enforce permission policy.
    /// Implementations must declare the MAXIMUM permission their most sensitive
    /// operation requires — not the permission of the happy path.
    fn permission_level(&self) -> PermissionLevel;

    /// Executes the tool with the given arguments and context.
    ///
    /// Contract:
    /// - Validate arguments against `parameters_schema()` before executing.
    /// - Check `context.granted_permission >= self.permission_level()` before side effects.
    /// - Never panic in production — return `Err(ToolError)` for all failure cases.
    /// - Respect `context.dry_run` — in dry-run mode, simulate and log side effects
    ///   without performing them.
    /// - Stay within `context.workspace_root` for all filesystem operations.
    async fn execute(
        &self,
        args: serde_json::Value,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError>;

    /// Returns a human-readable example of a valid invocation for documentation.
    ///
    /// Used in the `truenorth tools list` CLI output.
    fn usage_example(&self) -> Option<&str> {
        None
    }
}

/// The tool registry: a central directory of all available tools.
///
/// The registry is responsible for:
/// 1. Maintaining the list of available tools (for LLM tool definition injection)
/// 2. Routing tool call requests to the correct implementation
/// 3. Enforcing permission policy before execution
/// 4. Wrapping WASM tools transparently alongside native Rust tools
/// 5. Discovering tools from MCP-compatible external servers
#[async_trait]
pub trait ToolRegistry: Send + Sync + std::fmt::Debug {
    /// Registers a tool implementation with the registry.
    ///
    /// Returns an error if a tool with the same name is already registered.
    fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;

    /// Returns a list of all registered tool definitions.
    ///
    /// This is what gets injected into LLM system prompts and completion requests.
    fn list_tools(&self) -> Vec<ToolDefinitionSummary>;

    /// Returns the full parameter schema for a specific tool by name.
    ///
    /// Used to construct the `tools` array in `CompletionRequest`.
    fn get_schema(&self, tool_name: &str) -> Option<serde_json::Value>;

    /// Executes a tool call in a sandboxed context.
    ///
    /// This is the single point through which ALL tool execution flows.
    /// It is responsible for:
    /// - Resolving the tool name to an implementation
    /// - Checking permission policy against granted_permission
    /// - Executing in WASM sandbox if the tool is a WASM module
    /// - Logging the invocation and result to the audit trail
    /// - Emitting a `ReasoningEvent::ToolCalled` + `ReasoningEvent::ToolResult`
    async fn execute_sandboxed(
        &self,
        call: &ToolCall,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError>;

    /// Discovers tools from an MCP server and registers them as tool adapters.
    ///
    /// This is how external MCP-compatible tool servers are integrated.
    /// Returns the number of new tools discovered and registered.
    async fn discover_mcp_tools(&self, server_url: &str) -> Result<usize, RegistryError>;

    /// Returns whether a specific tool is available (registered and not in error state).
    fn is_available(&self, tool_name: &str) -> bool;

    /// Returns the number of registered tools.
    fn tool_count(&self) -> usize;

    /// Unregisters a tool by name.
    ///
    /// Used when a WASM module is evicted or an MCP server disconnects.
    fn unregister(&self, tool_name: &str) -> Result<(), RegistryError>;
}
