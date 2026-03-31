//! Web fetch tool — fetches the text content of a URL.
//!
//! [`WebFetchTool`] retrieves the HTML (or plaintext) content of a URL via
//! `reqwest` and performs a basic HTML-to-text extraction by stripping HTML
//! tags. The extracted text is returned to the LLM, truncated at a configurable
//! maximum character count to keep context usage manageable.

use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::debug;

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, SideEffect, ToolError, ToolResult};

/// Maximum characters to return from a fetched page (to bound LLM context usage).
const DEFAULT_MAX_CHARS: usize = 20_000;

/// Fetches and returns the text content of a URL.
///
/// Strips HTML tags to produce readable plaintext. Large pages are truncated
/// to `max_chars` (default: 20 000).
///
/// # Permission
/// `Medium` — makes outbound HTTP GET requests to arbitrary URLs.
#[derive(Debug)]
pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    /// Creates a new `WebFetchTool` with a default `reqwest` client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("TrueNorth/0.1 (web-fetch)")
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Strips HTML tags from `html` using a simple state-machine approach.
///
/// This is intentionally minimal — it removes `<...>` markup and collapses
/// repeated whitespace. For production use, consider integrating a proper
/// HTML parser.
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script_or_style = false;
    let mut tag_buf = String::new();

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' => {
                // Check if we entered or exited a script/style block.
                let tag_lower = tag_buf.to_lowercase();
                let tag_name = tag_lower.trim_start_matches('/').split_whitespace().next().unwrap_or("");
                if tag_name == "script" || tag_name == "style" {
                    in_script_or_style = !tag_lower.starts_with('/');
                } else if tag_name.starts_with("script") || tag_name.starts_with("style") {
                    in_script_or_style = !tag_lower.starts_with('/');
                }
                in_tag = false;
            }
            _ if in_tag => {
                tag_buf.push(ch);
            }
            _ if in_script_or_style => {
                // Skip script/style content entirely.
            }
            _ => {
                out.push(ch);
            }
        }
    }

    // Collapse whitespace.
    let mut result = String::with_capacity(out.len());
    let mut prev_space = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(ch);
            prev_space = false;
        }
    }

    result.trim().to_string()
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Fetch the text content of a web page or URL. Strips HTML tags and returns \
         readable plaintext. Use this to read articles, documentation, or any \
         web-accessible content. Returns up to 20 000 characters by default."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch. Must begin with http:// or https://.",
                    "format": "uri"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default 20000, max 100000).",
                    "minimum": 100,
                    "maximum": 100000,
                    "default": 20000
                }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Medium
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"url": "https://docs.rs/tokio/latest/tokio/", "max_chars": 10000}"#)
    }

    async fn execute(&self, args: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let url = args["url"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'url'".to_string(),
        })?;

        // Validate URL scheme.
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidArguments {
                tool_name: self.name().to_string(),
                message: "URL must begin with http:// or https://".to_string(),
            });
        }

        let max_chars = args["max_chars"]
            .as_u64()
            .map(|v| v.clamp(100, 100_000) as usize)
            .unwrap_or(DEFAULT_MAX_CHARS);

        debug!(%url, max_chars, "Fetching URL");

        // --- Perform request ---
        let resp = self
            .client
            .get(url)
            .header("Accept", "text/html,application/xhtml+xml,text/plain")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if !resp.status().is_success() {
            return Err(ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("HTTP {status} from {url}"),
            });
        }

        let raw_body = resp.text().await.map_err(|e| ToolError::ExecutionFailed {
            tool_name: self.name().to_string(),
            message: format!("Failed to read response body: {e}"),
        })?;

        // --- Extract text ---
        let text = if content_type.contains("html") {
            strip_html_tags(&raw_body)
        } else {
            raw_body.trim().to_string()
        };

        let truncated = text.chars().take(max_chars).collect::<String>();
        let was_truncated = text.chars().count() > max_chars;

        let execution_ms = start.elapsed().as_millis() as u64;

        let llm_output = json!({
            "url": url,
            "content": truncated,
            "char_count": truncated.len(),
            "truncated": was_truncated,
            "content_type": content_type
        });

        let display_output = json!({
            "type": "web_page",
            "url": url,
            "content": truncated,
            "truncated": was_truncated
        });

        Ok(ToolResult {
            llm_output,
            display_output: Some(display_output),
            side_effects: vec![SideEffect::NetworkRequest {
                method: "GET".to_string(),
                url: url.to_string(),
                status,
            }],
            execution_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        let html = "<html><head><title>Test</title></head><body><p>Hello <b>world</b>!</p></body></html>";
        let text = strip_html_tags(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn test_strip_removes_script() {
        let html = "<p>Before</p><script>var x = 1;</script><p>After</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("var x"));
    }
}
