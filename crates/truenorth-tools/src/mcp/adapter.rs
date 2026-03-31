//! MCP tool adapter — wraps a discovered MCP tool as a TrueNorth `Tool`.
//!
//! [`McpToolAdapter`] translates between the MCP tool schema format and the
//! TrueNorth `Tool` trait, allowing MCP server tools to be registered and
//! executed exactly like built-in tools. From the orchestrator's perspective,
//! there is no difference between a built-in tool and an MCP-backed tool.

use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;
use tracing::debug;

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, SideEffect, ToolError, ToolResult};

use crate::mcp::client::{McpClient, McpToolDefinition};

/// Wraps a remote MCP server tool as a TrueNorth `Tool` implementation.
///
/// The adapter maintains a reference to the originating server URL and the
/// tool's definition as discovered via `McpClient::list_tools`. Invocations
/// are forwarded to the server via `McpClient::invoke_tool`.
///
/// # Permission
///
/// MCP tools default to `PermissionLevel::Medium`. Individual tools can
/// override this by including a `"truenorth_permission"` key in their
/// `input_schema` extension metadata:
/// `{"truenorth_permission": "Low" | "Medium" | "High"}`.
#[derive(Debug)]
pub struct McpToolAdapter {
    /// Base URL of the originating MCP server.
    server_url: String,
    /// The tool definition as discovered from the server.
    definition: McpToolDefinition,
    /// Resolved permission level for this tool.
    permission: PermissionLevel,
    /// Lazy-constructed HTTP client (one per adapter instance).
    client: McpClient,
}

impl McpToolAdapter {
    /// Creates a new `McpToolAdapter` for the given tool definition.
    ///
    /// The permission level is extracted from the definition's `input_schema`
    /// extension field `"truenorth_permission"` if present; otherwise defaults
    /// to `PermissionLevel::Medium`.
    ///
    /// # Arguments
    /// * `server_url` — base URL of the MCP server hosting this tool.
    /// * `definition` — the tool definition returned by `McpClient::list_tools`.
    pub fn new(server_url: String, definition: McpToolDefinition) -> Self {
        let permission = Self::extract_permission(&definition.input_schema);
        let client = McpClient::new(server_url.clone());
        Self {
            server_url,
            definition,
            permission,
            client,
        }
    }

    /// Extracts a `PermissionLevel` from the schema's `"truenorth_permission"` key.
    ///
    /// Returns `PermissionLevel::Medium` if the key is absent or unrecognised.
    fn extract_permission(schema: &Value) -> PermissionLevel {
        match schema
            .get("truenorth_permission")
            .and_then(|v| v.as_str())
        {
            Some("Low") => PermissionLevel::Low,
            Some("High") => PermissionLevel::High,
            _ => PermissionLevel::Medium,
        }
    }

    /// Returns the originating server URL.
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Returns the raw MCP tool definition.
    pub fn definition(&self) -> &McpToolDefinition {
        &self.definition
    }
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters_schema(&self) -> Value {
        // Return the MCP input_schema directly.
        // Strip out any TrueNorth-specific extension fields before passing to LLM.
        let mut schema = self.definition.input_schema.clone();
        if let Some(obj) = schema.as_object_mut() {
            obj.remove("truenorth_permission");
        }
        schema
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission.clone()
    }

    fn usage_example(&self) -> Option<&str> {
        None
    }

    async fn execute(&self, args: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        debug!(
            tool_name = %self.definition.name,
            server = %self.server_url,
            "Invoking MCP tool"
        );

        // --- Invoke on MCP server ---
        let result = self
            .client
            .invoke_tool(&self.definition.name, args)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.definition.name.clone(),
                message: format!("MCP invocation failed: {e}"),
            })?;

        let execution_ms = start.elapsed().as_millis() as u64;

        let side_effects = vec![SideEffect::ExternalApiCalled {
            service: format!("mcp:{}", self.server_url),
            endpoint: format!("/tools/{}/invoke", self.definition.name),
        }];

        Ok(ToolResult {
            llm_output: result.clone(),
            display_output: Some(serde_json::json!({
                "type": "mcp_result",
                "tool": self.definition.name,
                "server": self.server_url,
                "result": result
            })),
            side_effects,
            execution_ms,
        })
    }
}
