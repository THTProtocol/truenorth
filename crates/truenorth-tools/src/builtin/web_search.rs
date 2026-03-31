//! Web search tool — performs internet searches via a configurable search API.
//!
//! [`WebSearchTool`] sends a query to the Brave Search API (or a compatible
//! endpoint configured via the `TRUENORTH_SEARCH_API_URL` and
//! `TRUENORTH_SEARCH_API_KEY` environment variables) and returns structured
//! results with title, URL, and snippet.

use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, warn};

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, SideEffect, ToolError, ToolResult};

/// Default search API endpoint (Brave Search compatible).
const DEFAULT_SEARCH_API_URL: &str = "https://api.search.brave.com/res/v1/web/search";

/// A single search result item.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResultItem {
    /// Page title.
    pub title: String,
    /// Page URL.
    pub url: String,
    /// Short text snippet from the page.
    pub snippet: String,
}

/// Brave Search API response shape (partial).
#[derive(Debug, Deserialize)]
struct BraveSearchResponse {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: Option<String>,
}

/// Performs web searches via the Brave Search API.
///
/// Requires the `TRUENORTH_SEARCH_API_KEY` environment variable to be set.
/// Optionally, `TRUENORTH_SEARCH_API_URL` overrides the endpoint.
///
/// # Permission
/// `Medium` — makes outbound HTTP requests to the search API.
#[derive(Debug)]
pub struct WebSearchTool {
    client: reqwest::Client,
}

impl WebSearchTool {
    /// Creates a new `WebSearchTool` with a default `reqwest` client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "search_web"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Use this when you need up-to-date facts, \
         news, or information not in your training data. Returns a list of results with \
         titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to send to the search engine."
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1–10, default 5).",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Medium
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"query": "Rust async programming best practices", "count": 5}"#)
    }

    async fn execute(&self, args: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let query = args["query"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'query'".to_string(),
        })?;

        let count = args["count"].as_u64().unwrap_or(5).clamp(1, 10) as usize;

        debug!(query, count, "Executing web search");

        // --- Read config from environment ---
        let api_url = std::env::var("TRUENORTH_SEARCH_API_URL")
            .unwrap_or_else(|_| DEFAULT_SEARCH_API_URL.to_string());

        let api_key = std::env::var("TRUENORTH_SEARCH_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            warn!("TRUENORTH_SEARCH_API_KEY not set; search results may be empty or rate-limited");
        }

        // --- Build request ---
        let resp = self
            .client
            .get(&api_url)
            .query(&[("q", query), ("count", &count.to_string())])
            .header("Accept", "application/json")
            .header("Accept-Encoding", "gzip")
            .header("X-Subscription-Token", &api_key)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = resp.status().as_u16();

        let results: Vec<SearchResultItem> = if resp.status().is_success() {
            let body: BraveSearchResponse = resp.json().await.map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to parse search response: {e}"),
            })?;

            body.web
                .map(|w| {
                    w.results
                        .into_iter()
                        .take(count)
                        .map(|r| SearchResultItem {
                            title: r.title,
                            url: r.url,
                            snippet: r.description.unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let body = resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Search API returned HTTP {status}: {body}"),
            });
        };

        let execution_ms = start.elapsed().as_millis() as u64;

        let llm_output = json!({
            "query": query,
            "count": results.len(),
            "results": results
        });

        let display_output = json!({
            "type": "search_results",
            "query": query,
            "results": results
        });

        Ok(ToolResult {
            llm_output,
            display_output: Some(display_output),
            side_effects: vec![SideEffect::NetworkRequest {
                method: "GET".to_string(),
                url: api_url,
                status,
            }],
            execution_ms,
        })
    }
}
