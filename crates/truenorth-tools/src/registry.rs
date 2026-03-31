//! Default `ToolRegistry` implementation backed by a thread-safe `HashMap`.
//!
//! [`DefaultToolRegistry`] is the canonical in-process tool registry for TrueNorth.
//! All tool registration, discovery, permission checking, and execution routing
//! flows through this type. A single shared instance (wrapped in `Arc`) is
//! injected into the orchestrator at startup.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use truenorth_core::traits::tool::{
    RegistryError, Tool, ToolContext, ToolDefinitionSummary, ToolRegistry,
};
use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::types::tool::{PermissionLevel, ToolCall, ToolError, ToolResult};

use crate::mcp::client::McpClient;

/// Inner state of the registry, protected by an `RwLock` for concurrent access.
#[derive(Debug)]
struct RegistryInner {
    /// The registered tools, keyed by canonical name.
    tools: HashMap<String, Arc<dyn Tool>>,
    /// Total number of tool executions since startup (for metrics).
    execution_count: u64,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            tools: HashMap::new(),
            execution_count: 0,
        }
    }
}

/// The default `ToolRegistry` implementation.
///
/// Backed by a `HashMap<String, Arc<dyn Tool>>` protected by a `RwLock`.
/// Multiple `Arc` clones may be held simultaneously; all share the same
/// underlying state.
///
/// # Thread Safety
///
/// All public methods acquire the read or write lock as needed. Long-running
/// async operations (tool execution) hold *no* lock during execution — the
/// `Arc<dyn Tool>` is cloned out before releasing the read lock.
#[derive(Debug, Clone)]
pub struct DefaultToolRegistry {
    inner: Arc<RwLock<RegistryInner>>,
}

impl DefaultToolRegistry {
    /// Creates a new, empty registry.
    ///
    /// Call [`crate::builtin::register_all_builtin_tools`] to populate with
    /// the standard built-in tool set.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner::new())),
        }
    }

    /// Returns all tools whose name or description fuzzy-matches `query`.
    ///
    /// Matching is case-insensitive substring matching against both the tool
    /// name and its description. Returns tools in insertion order.
    ///
    /// # Arguments
    /// * `query` — a keyword or phrase to search for.
    pub fn discover(&self, query: &str) -> Vec<ToolDefinitionSummary> {
        let q = query.to_lowercase();
        let inner = self.inner.read().expect("registry lock poisoned");
        inner
            .tools
            .values()
            .filter(|t| {
                t.name().to_lowercase().contains(&q)
                    || t.description().to_lowercase().contains(&q)
            })
            .map(|t| ToolDefinitionSummary {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
                permission_level: t.permission_level(),
            })
            .collect()
    }

    /// Builds a `ReasoningEvent` for a tool being called.
    fn make_tool_called_event(
        session_id: Uuid,
        step_id: Uuid,
        call: &ToolCall,
        permission_level: &PermissionLevel,
    ) -> ReasoningEvent {
        let input_summary = serde_json::to_string(&call.arguments)
            .unwrap_or_else(|_| "<non-serializable>".to_string())
            .chars()
            .take(200)
            .collect::<String>();

        ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::ToolCalled {
                step_id,
                call_id: call.call_id.clone(),
                tool_name: call.name.clone(),
                input_summary,
                permission_level: format!("{:?}", permission_level),
            },
        )
    }

    /// Builds a `ReasoningEvent` for a tool result.
    fn make_tool_result_event(
        session_id: Uuid,
        step_id: Uuid,
        call: &ToolCall,
        result: &Result<ToolResult, ToolError>,
        duration_ms: u64,
    ) -> ReasoningEvent {
        let (success, result_summary) = match result {
            Ok(r) => {
                let summary = serde_json::to_string(&r.llm_output)
                    .unwrap_or_else(|_| "<non-serializable>".to_string())
                    .chars()
                    .take(200)
                    .collect::<String>();
                (true, summary)
            }
            Err(e) => (false, e.to_string()),
        };

        ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::ToolResult {
                step_id,
                call_id: call.call_id.clone(),
                tool_name: call.name.clone(),
                success,
                result_summary,
                duration_ms,
            },
        )
    }
}

impl Default for DefaultToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolRegistry for DefaultToolRegistry {
    /// Registers a tool with the registry.
    ///
    /// Returns [`RegistryError::DuplicateTool`] if a tool with the same name
    /// is already registered. Use [`unregister`] first to replace a tool.
    fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError> {
        let name = tool.name().to_string();
        let mut inner = self.inner.write().expect("registry lock poisoned");

        if inner.tools.contains_key(&name) {
            return Err(RegistryError::DuplicateTool { name });
        }

        info!(tool_name = %name, "Registered tool");
        inner.tools.insert(name, Arc::from(tool));
        Ok(())
    }

    /// Returns a summary of all registered tools.
    ///
    /// The returned list is suitable for injection into LLM system prompts.
    fn list_tools(&self) -> Vec<ToolDefinitionSummary> {
        let inner = self.inner.read().expect("registry lock poisoned");
        inner
            .tools
            .values()
            .map(|t| ToolDefinitionSummary {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
                permission_level: t.permission_level(),
            })
            .collect()
    }

    /// Returns the full parameter schema for a named tool, or `None` if not found.
    fn get_schema(&self, tool_name: &str) -> Option<serde_json::Value> {
        let inner = self.inner.read().expect("registry lock poisoned");
        inner.tools.get(tool_name).map(|t| t.parameters_schema())
    }

    /// Executes a tool call in a sandboxed context.
    ///
    /// This is the single execution choke-point. It:
    /// 1. Resolves the tool by name.
    /// 2. Checks that `context.granted_permission >= tool.permission_level()`.
    /// 3. Executes the tool (delegating to the `Tool::execute` implementation).
    /// 4. Emits `ReasoningEvent::ToolCalled` before and `ReasoningEvent::ToolResult` after.
    ///
    /// The `ReasoningEvent`s are logged via `tracing` rather than through an injected
    /// emitter so that the registry remains decoupled from the event bus. The
    /// orchestrator layer injects a subscriber that forwards these to the bus.
    async fn execute_sandboxed(
        &self,
        call: &ToolCall,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        // --- 1. Resolve tool ---
        let tool = {
            let inner = self.inner.read().expect("registry lock poisoned");
            inner.tools.get(&call.name).cloned().ok_or_else(|| {
                ToolError::ExecutionFailed {
                    tool_name: call.name.clone(),
                    message: format!("Tool '{}' not found in registry", call.name),
                }
            })?
        };

        // --- 2. Permission check ---
        let required = tool.permission_level();
        if required > context.granted_permission {
            warn!(
                tool_name = %call.name,
                required = ?required,
                granted = ?context.granted_permission,
                "Permission denied for tool execution"
            );
            return Err(ToolError::PermissionDenied {
                tool_name: call.name.clone(),
                required,
                granted: context.granted_permission.clone(),
            });
        }

        // --- 3. Emit ToolCalled reasoning event ---
        let step_id = context.step_id.unwrap_or_else(Uuid::new_v4);
        let called_event = Self::make_tool_called_event(
            context.session_id,
            step_id,
            call,
            &tool.permission_level(),
        );
        debug!(event = ?called_event.payload, "Emitting ToolCalled reasoning event");

        // --- 4. Execute ---
        let start = Instant::now();
        debug!(tool_name = %call.name, "Executing tool");

        let result = tool.execute(call.arguments.clone(), context).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        // --- 5. Emit ToolResult reasoning event ---
        let result_event = Self::make_tool_result_event(
            context.session_id,
            step_id,
            call,
            &result,
            duration_ms,
        );
        debug!(event = ?result_event.payload, "Emitting ToolResult reasoning event");

        // --- 6. Update execution counter ---
        {
            let mut inner = self.inner.write().expect("registry lock poisoned");
            inner.execution_count += 1;
        }

        match &result {
            Ok(_) => {
                info!(
                    tool_name = %call.name,
                    duration_ms,
                    "Tool executed successfully"
                );
            }
            Err(e) => {
                error!(
                    tool_name = %call.name,
                    duration_ms,
                    error = %e,
                    "Tool execution failed"
                );
            }
        }

        result
    }

    /// Discovers tools from an MCP server and registers them as adapters.
    ///
    /// Connects to the MCP server at `server_url`, lists its available tools,
    /// and registers each as a [`crate::mcp::adapter::McpToolAdapter`].
    /// Returns the number of tools successfully discovered and registered.
    async fn discover_mcp_tools(&self, server_url: &str) -> Result<usize, RegistryError> {
        let client = McpClient::new(server_url);

        let tools = client.list_tools().await.map_err(|e| {
            RegistryError::McpDiscoveryFailed {
                server_url: server_url.to_string(),
                message: e.to_string(),
            }
        })?;

        let count = tools.len();
        info!(server_url, tool_count = count, "Discovered MCP tools");

        for tool_def in tools {
            let adapter =
                crate::mcp::adapter::McpToolAdapter::new(server_url.to_string(), tool_def);
            if let Err(e) = self.register(Box::new(adapter)) {
                warn!(error = %e, "Failed to register MCP tool adapter");
            }
        }

        Ok(count)
    }

    /// Returns whether a named tool is registered and ready.
    fn is_available(&self, tool_name: &str) -> bool {
        let inner = self.inner.read().expect("registry lock poisoned");
        inner.tools.contains_key(tool_name)
    }

    /// Returns the total number of registered tools.
    fn tool_count(&self) -> usize {
        let inner = self.inner.read().expect("registry lock poisoned");
        inner.tools.len()
    }

    /// Removes a tool from the registry by name.
    ///
    /// Used when a WASM module is evicted or an MCP server disconnects.
    fn unregister(&self, tool_name: &str) -> Result<(), RegistryError> {
        let mut inner = self.inner.write().expect("registry lock poisoned");
        if inner.tools.remove(tool_name).is_none() {
            return Err(RegistryError::ToolNotFound {
                name: tool_name.to_string(),
            });
        }
        info!(tool_name, "Unregistered tool");
        Ok(())
    }
}
