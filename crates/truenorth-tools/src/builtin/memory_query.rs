//! Memory query tool — exposes the TrueNorth memory store to the LLM as a tool.
//!
//! [`MemoryQueryTool`] wraps the memory store's query interface so the LLM can
//! explicitly request memory lookups. Supports text (keyword), semantic
//! (embedding-based), and hybrid search modes.
//!
//! In practice this tool issues queries against an in-process store that
//! implements `MemoryStore`. In the MVP, it delegates to an HTTP endpoint
//! on the TrueNorth daemon — the same endpoint the orchestrator uses internally.

use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::debug;

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, ToolError, ToolResult};

/// Default number of results to return.
const DEFAULT_RESULT_COUNT: usize = 10;

/// A memory query result item.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MemoryResultItem {
    /// The memory entry content.
    pub content: String,
    /// Relevance score (0.0–1.0 for semantic/hybrid, None for text).
    pub score: Option<f32>,
    /// The memory scope (e.g., "session", "project", "global").
    pub scope: String,
    /// ISO-8601 timestamp when the entry was created.
    pub created_at: String,
}

/// Queries the TrueNorth memory store.
///
/// Supports three search modes:
/// - `text`: Keyword/full-text search.
/// - `semantic`: Embedding-based semantic similarity search.
/// - `hybrid`: Combined text + semantic (recommended for most queries).
///
/// # Permission
/// `Low` — read-only access to the memory store.
#[derive(Debug)]
pub struct MemoryQueryTool {
    client: reqwest::Client,
}

impl MemoryQueryTool {
    /// Creates a new `MemoryQueryTool`.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Returns the memory API base URL from the environment, defaulting to localhost.
    fn memory_api_url(&self) -> String {
        std::env::var("TRUENORTH_MEMORY_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3001".to_string())
    }
}

impl Default for MemoryQueryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> &str {
        "memory_query"
    }

    fn description(&self) -> &str {
        "Search the TrueNorth memory store for relevant past information. Use this \
         to retrieve context from previous sessions, stored facts, research notes, \
         or any previously recorded knowledge. Supports text, semantic, and hybrid \
         search modes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query text."
                },
                "mode": {
                    "type": "string",
                    "enum": ["text", "semantic", "hybrid"],
                    "description": "Search mode. 'hybrid' is recommended. Default: 'hybrid'.",
                    "default": "hybrid"
                },
                "scope": {
                    "type": "string",
                    "enum": ["session", "project", "global", "all"],
                    "description": "Memory scope to search. Default: 'all'.",
                    "default": "all"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1–50, default 10).",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Low
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"query": "Rust async patterns", "mode": "hybrid", "scope": "all", "count": 5}"#)
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let query = args["query"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'query'".to_string(),
        })?;

        let mode = args["mode"].as_str().unwrap_or("hybrid");
        let scope = args["scope"].as_str().unwrap_or("all");
        let count = args["count"]
            .as_u64()
            .map(|v| v.clamp(1, 50) as usize)
            .unwrap_or(DEFAULT_RESULT_COUNT);

        debug!(%query, mode, scope, count, "Querying memory store");

        // --- Build request payload ---
        let request_body = json!({
            "query": query,
            "mode": mode,
            "scope": scope,
            "count": count,
            "session_id": context.session_id.to_string()
        });

        // --- Call memory API ---
        let api_url = format!("{}/api/memory/search", self.memory_api_url());

        let resp = self
            .client
            .post(&api_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Memory API request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Memory API returned HTTP {status}: {body}"),
            });
        }

        let results: Vec<MemoryResultItem> = resp.json().await.unwrap_or_else(|_| vec![]);

        let execution_ms = start.elapsed().as_millis() as u64;

        let llm_output = json!({
            "query": query,
            "mode": mode,
            "scope": scope,
            "result_count": results.len(),
            "results": results
        });

        let display_output = json!({
            "type": "memory_results",
            "query": query,
            "mode": mode,
            "results": results
        });

        Ok(ToolResult {
            llm_output,
            display_output: Some(display_output),
            side_effects: vec![],
            execution_ms,
        })
    }
}
