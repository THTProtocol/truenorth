//! MCP HTTP client — discovers and invokes tools on MCP-compatible servers.
//!
//! [`McpClient`] implements the minimal MCP HTTP/JSON transport required for
//! tool discovery (`GET /tools`) and invocation (`POST /tools/{name}/invoke`).
//! Authentication tokens are read from the `TRUENORTH_MCP_TOKEN` environment
//! variable.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tracing::{debug, info};

/// A tool definition as returned by the MCP server's `GET /tools` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    /// The canonical tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: Value,
    /// Optional hint about the version of the tool.
    #[serde(default)]
    pub version: Option<String>,
    /// Whether the tool is currently active on the server.
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

/// The JSON envelope returned by `POST /tools/{name}/invoke`.
#[derive(Debug, Deserialize)]
pub struct McpInvokeResponse {
    /// `true` if the tool succeeded.
    pub success: bool,
    /// The tool's output as a JSON value.
    pub result: Value,
    /// Human-readable error message when `success` is `false`.
    #[serde(default)]
    pub error: Option<String>,
    /// Execution time reported by the server, in milliseconds.
    #[serde(default)]
    pub execution_ms: Option<u64>,
}

/// Errors from MCP operations.
#[allow(missing_docs)]
#[derive(Debug, Error)]
pub enum McpError {
    /// An HTTP request to the MCP server failed.
    #[error("HTTP request to MCP server failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The server returned a non-success HTTP status.
    #[error("MCP server returned HTTP {status}: {body}")]
    ServerError { status: u16, body: String },

    /// The server returned a malformed JSON response.
    #[error("Failed to parse MCP server response: {0}")]
    ParseError(#[from] serde_json::Error),

    /// The server indicated tool invocation failure.
    #[error("MCP tool invocation failed: {0}")]
    ToolFailed(String),
}

/// HTTP client for MCP-compatible tool servers.
///
/// A `McpClient` is bound to a single server URL. Multiple clients can be
/// created for multiple servers.
///
/// # Authentication
///
/// If the `TRUENORTH_MCP_TOKEN` environment variable is set, its value is
/// sent as a `Bearer` token in the `Authorization` header.
#[derive(Debug, Clone)]
pub struct McpClient {
    server_url: String,
    http: reqwest::Client,
    auth_token: Option<String>,
}

impl McpClient {
    /// Creates a new `McpClient` pointing at `server_url`.
    ///
    /// # Arguments
    /// * `server_url` — base URL of the MCP server (e.g., `http://localhost:8080`).
    pub fn new(server_url: impl Into<String>) -> Self {
        let auth_token = std::env::var("TRUENORTH_MCP_TOKEN").ok();
        Self {
            server_url: server_url.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .user_agent("TrueNorth/0.1 (mcp-client)")
                .build()
                .unwrap_or_default(),
            auth_token,
        }
    }

    /// Discovers all tools available on the connected MCP server.
    ///
    /// Sends `GET {server_url}/tools` and deserializes the response as a list
    /// of [`McpToolDefinition`] objects.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the HTTP request fails or the response cannot
    /// be parsed.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpError> {
        let url = format!("{}/tools", self.server_url);
        debug!(%url, "Listing MCP tools");

        let mut req = self.http.get(&url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }

        let resp = req.send().await?;
        let status = resp.status().as_u16();

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::ServerError { status, body });
        }

        let tools: Vec<McpToolDefinition> = resp.json().await?;
        info!(server = %self.server_url, count = tools.len(), "Discovered MCP tools");
        Ok(tools)
    }

    /// Invokes a named tool on the MCP server.
    ///
    /// Sends `POST {server_url}/tools/{tool_name}/invoke` with `arguments` as
    /// the JSON body and returns the server's result.
    ///
    /// # Arguments
    /// * `tool_name` — the name of the tool to invoke.
    /// * `arguments` — the JSON arguments to pass.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the HTTP request fails, the server returns an
    /// error, or the tool itself reports failure.
    pub async fn invoke_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, McpError> {
        let url = format!("{}/tools/{}/invoke", self.server_url, tool_name);
        debug!(%url, %tool_name, "Invoking MCP tool");

        let body = serde_json::json!({ "arguments": arguments });

        let mut req = self.http.post(&url).json(&body);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }

        let resp = req.send().await?;
        let status = resp.status().as_u16();

        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(McpError::ServerError {
                status,
                body: body_text,
            });
        }

        let invoke_resp: McpInvokeResponse = resp.json().await?;

        if !invoke_resp.success {
            return Err(McpError::ToolFailed(
                invoke_resp
                    .error
                    .unwrap_or_else(|| "Unknown tool error".to_string()),
            ));
        }

        Ok(invoke_resp.result)
    }

    /// Returns the base URL of the connected MCP server.
    pub fn server_url(&self) -> &str {
        &self.server_url
    }
}
