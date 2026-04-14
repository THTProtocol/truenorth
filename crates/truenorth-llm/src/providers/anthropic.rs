//! Anthropic Claude provider — implements `LlmProvider` via the Messages API.
//!
//! Supports:
//! - Synchronous completions (non-streaming)
//! - Streaming via SSE (`stream: true`)
//! - Extended thinking (`"type": "thinking"` content blocks)
//! - Tool use (Anthropic tool definition format)
//!
//! ## API reference
//!
//! Base URL: `https://api.anthropic.com/v1/messages`
//! Auth header: `x-api-key: <key>`
//! API version: `anthropic-version: 2023-06-01`

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};
use truenorth_core::error::LlmError;
use truenorth_core::traits::llm_provider::{LlmProvider, StreamHandle};
use truenorth_core::types::llm::{
    CompletionRequest, CompletionResponse, ProviderCapabilities, StopReason,
    StreamEvent, TokenUsage,
};
use truenorth_core::types::message::{ContentBlock, MessageRole};

use crate::stream::{collect_sse_data_lines, json_str, try_parse_json};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
#[allow(dead_code)]
const DEFAULT_MODEL: &str = "claude-opus-4-5";

/// Anthropic Claude provider.
///
/// Thread-safe: availability flags use atomic booleans. The HTTP client is
/// shared via `Arc` internally by `reqwest::Client`.
#[derive(Debug)]
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    client: Client,
    capabilities: ProviderCapabilities,
    is_available: Arc<AtomicBool>,
    is_exhausted: Arc<AtomicBool>,
    success_count: Arc<AtomicU64>,
    failure_count: Arc<AtomicU64>,
}

impl AnthropicProvider {
    /// Creates a new `AnthropicProvider` with the given API key and model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let is_thinking_model = model.contains("claude-3-5") || model.contains("claude-3-7")
            || model.contains("opus") || model.contains("sonnet");

        Self {
            api_key: api_key.into(),
            model: model.clone(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("Failed to build HTTP client"),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calling: true,
                supports_vision: true,
                supports_thinking: is_thinking_model,
                max_context_tokens: 200_000,
                max_output_tokens: 8192,
                output_modalities: vec!["text".to_string()],
                provider_name: "anthropic".to_string(),
                model_name: model,
            },
            is_available: Arc::new(AtomicBool::new(true)),
            is_exhausted: Arc::new(AtomicBool::new(false)),
            success_count: Arc::new(AtomicU64::new(0)),
            failure_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Builds the JSON request body for the Anthropic Messages API.
    fn build_request_body(&self, request: &CompletionRequest) -> Value {
        // Separate system messages from conversation messages
        let mut system_content: Option<String> = None;
        let mut messages: Vec<Value> = Vec::new();

        for msg in &request.messages {
            match msg.role {
                MessageRole::System => {
                    // Anthropic takes system as a top-level field, not in messages array
                    let text = extract_text_from_blocks(&msg.content);
                    system_content = Some(text);
                }
                MessageRole::User => {
                    messages.push(build_user_message(&msg.content));
                }
                MessageRole::Assistant => {
                    messages.push(build_assistant_message(&msg.content));
                }
                MessageRole::Tool => {
                    messages.push(build_tool_result_message(&msg.content));
                }
            }
        }

        let mut body = json!({
            "model": self.model,
            "max_tokens": request.parameters.max_tokens,
            "messages": messages,
        });

        if let Some(system) = system_content {
            body["system"] = json!(system);
        }

        if let Some(temp) = request.parameters.temperature {
            body["temperature"] = json!(temp);
        }

        if !request.parameters.stop_sequences.is_empty() {
            body["stop_sequences"] = json!(request.parameters.stop_sequences);
        }

        // Extended thinking support
        if request.parameters.enable_thinking {
            let budget = request.parameters.thinking_budget.unwrap_or(8000);
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget
            });
        }

        // Tool definitions
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                let anthropic_tools: Vec<Value> = tools
                    .iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    }))
                    .collect();
                body["tools"] = json!(anthropic_tools);
            }
        }

        if request.stream {
            body["stream"] = json!(true);
        }

        body
    }

    /// Maps an Anthropic API HTTP status + body to an `LlmError`.
    fn map_http_error(&self, status: u16, body: &str) -> LlmError {
        match status {
            429 => LlmError::RateLimited {
                provider: "anthropic".to_string(),
                retry_after_secs: parse_retry_after_from_body(body),
            },
            401 | 403 => LlmError::ApiKeyExhausted {
                provider: "anthropic".to_string(),
            },
            529 => LlmError::RateLimited {
                provider: "anthropic".to_string(),
                retry_after_secs: 60,
            },
            400 => {
                if body.contains("content_policy") || body.contains("safety") {
                    LlmError::ModelRefusal {
                        reason: extract_error_message(body),
                    }
                } else {
                    LlmError::MalformedResponse {
                        provider: "anthropic".to_string(),
                        detail: extract_error_message(body),
                    }
                }
            }
            _ => LlmError::Other {
                provider: "anthropic".to_string(),
                message: format!("HTTP {}: {}", status, extract_error_message(body)),
            },
        }
    }

    /// Parses a non-streaming Anthropic response JSON into a `CompletionResponse`.
    fn parse_response(&self, body: &Value, latency_ms: u64) -> Result<CompletionResponse, LlmError> {
        let content_blocks = body
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        let mut text_parts: Vec<String> = Vec::new();
        let mut thinking_parts: Vec<String> = Vec::new();
        let mut response_blocks: Vec<ContentBlock> = Vec::new();

        for block in &content_blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    text_parts.push(text.to_string());
                    response_blocks.push(ContentBlock::Text { text: text.to_string() });
                }
                Some("thinking") => {
                    let thinking = block.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                    let signature = block.get("signature").and_then(|s| s.as_str()).map(|s| s.to_string());
                    thinking_parts.push(thinking.to_string());
                    response_blocks.push(ContentBlock::Thinking {
                        thinking: thinking.to_string(),
                        signature,
                    });
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Object(Default::default()));
                    response_blocks.push(ContentBlock::ToolUse { id, name, input });
                }
                other => {
                    warn!("Anthropic: unknown content block type: {:?}", other);
                }
            }
        }

        let usage = body.get("usage").map(parse_anthropic_usage).unwrap_or_default();

        let stop_reason = match body
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .unwrap_or("end_turn")
        {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "stop_sequence" => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        let model_used = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&self.model)
            .to_string();

        Ok(CompletionResponse {
            content: response_blocks,
            usage,
            provider: "anthropic".to_string(),
            model: model_used,
            stop_reason,
            latency_ms,
            received_at: Utc::now(),
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    fn is_available(&self) -> bool {
        self.is_available.load(Ordering::SeqCst) && !self.is_exhausted.load(Ordering::SeqCst)
    }

    fn mark_rate_limited(&self, retry_after_secs: u64) {
        self.is_available.store(false, Ordering::SeqCst);
        let available_flag = Arc::clone(&self.is_available);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(retry_after_secs)).await;
            available_flag.store(true, Ordering::SeqCst);
            info!("Anthropic rate limit cleared after {}s", retry_after_secs);
        });
    }

    fn mark_exhausted(&self) {
        self.is_exhausted.store(true, Ordering::SeqCst);
        warn!("Anthropic provider marked exhausted — API key invalid or quota gone");
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        let body = self.build_request_body(request);

        debug!(
            model = %self.model,
            request_id = %request.request_id,
            "Anthropic: sending completion request"
        );

        let response = self
            .client
            .post(ANTHROPIC_API_BASE)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "anthropic".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as u64;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            self.failure_count.fetch_add(1, Ordering::SeqCst);
            let err = self.map_http_error(status, &error_body);
            error!(
                provider = "anthropic",
                status = status,
                error = %err,
                "Anthropic completion failed"
            );
            return Err(err);
        }

        let response_json: Value = response.json().await.map_err(|e| LlmError::MalformedResponse {
            provider: "anthropic".to_string(),
            detail: format!("JSON parse error: {}", e),
        })?;

        let result = self.parse_response(&response_json, latency_ms);
        if result.is_ok() {
            self.success_count.fetch_add(1, Ordering::SeqCst);
            info!(
                provider = "anthropic",
                model = %self.model,
                latency_ms = latency_ms,
                "Anthropic completion successful"
            );
        } else {
            self.failure_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    async fn stream(&self, request: &CompletionRequest) -> Result<StreamHandle, LlmError> {
        let mut stream_request = request.clone();
        stream_request.stream = true;

        let body = self.build_request_body(&stream_request);
        let _started = Instant::now();

        debug!(
            model = %self.model,
            request_id = %request.request_id,
            "Anthropic: sending streaming request"
        );

        let response = self
            .client
            .post(ANTHROPIC_API_BASE)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "anthropic".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let err = self.map_http_error(status, &error_body);
            error!(
                provider = "anthropic",
                status = status,
                "Anthropic streaming request failed"
            );
            return Err(err);
        }

        let provider_name = self.name().to_string();

        // Convert the SSE stream into a Stream<Item = Result<StreamEvent, LlmError>>
        let data_stream = collect_sse_data_lines(response).await;
        let stream = data_stream
            .filter_map(move |line_result| {
                let provider = provider_name.clone();
                async move {
                    match line_result {
                        Err(e) => Some(Err(LlmError::NetworkError {
                            provider,
                            message: e,
                        })),
                        Ok(data) => parse_anthropic_sse_event(&data),
                    }
                }
            });

        Ok(Box::pin(stream))
    }

    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        Err(LlmError::Other {
            provider: "anthropic".to_string(),
            message: "Anthropic does not provide an embedding API. Use OpenAI or local fastembed.".to_string(),
        })
    }
}

// ─── Helper functions ────────────────────────────────────────────────────────

fn extract_text_from_blocks(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_user_message(content: &[ContentBlock]) -> Value {
    let parts: Vec<Value> = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::Image { mime_type, data } => Some(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": mime_type, "data": data }
            })),
            ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                let result_content: Vec<Value> = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
                        _ => None,
                    })
                    .collect();
                Some(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": result_content,
                    "is_error": is_error,
                }))
            }
            _ => None,
        })
        .collect();

    if parts.len() == 1 {
        if let Some(Value::Object(obj)) = parts.first() {
            if let Some(Value::String(t)) = obj.get("type") {
                if t == "text" {
                    if let Some(text) = obj.get("text") {
                        return json!({ "role": "user", "content": text });
                    }
                }
            }
        }
    }

    json!({ "role": "user", "content": parts })
}

fn build_assistant_message(content: &[ContentBlock]) -> Value {
    let parts: Vec<Value> = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::ToolUse { id, name, input } => Some(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            })),
            ContentBlock::Thinking { thinking, signature } => {
                let mut obj = json!({ "type": "thinking", "thinking": thinking });
                if let Some(sig) = signature {
                    obj["signature"] = json!(sig);
                }
                Some(obj)
            }
            _ => None,
        })
        .collect();

    json!({ "role": "assistant", "content": parts })
}

fn build_tool_result_message(content: &[ContentBlock]) -> Value {
    // Tool results in Anthropic go as user messages
    build_user_message(content)
}

fn parse_anthropic_usage(usage: &Value) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        output_tokens: usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cache_read_tokens: usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cache_write_tokens: usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        thinking_tokens: 0,
    }
}

fn parse_retry_after_from_body(body: &str) -> u64 {
    // Try to extract retry_after from error JSON
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(secs) = v.get("retry_after").and_then(|r| r.as_u64()) {
            return secs;
        }
    }
    60 // default 60 seconds
}

fn extract_error_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = v.get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.to_string();
        }
        if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
            return msg.to_string();
        }
    }
    body.chars().take(200).collect()
}

/// Parses an individual Anthropic SSE event data string into a `StreamEvent`.
fn parse_anthropic_sse_event(data: &str) -> Option<Result<StreamEvent, LlmError>> {
    let v = try_parse_json(data)?;
    let event_type = json_str(&v, "type");

    match event_type {
        "content_block_start" => {
            // Check if this is a tool_use block starting
            if let Some(content_block) = v.get("content_block") {
                let block_type = json_str(content_block, "type");
                if block_type == "tool_use" {
                    let id = json_str(content_block, "id").to_string();
                    let name = json_str(content_block, "name").to_string();
                    return Some(Ok(StreamEvent::ToolUseStart { id, name }));
                }
            }
            None
        }
        "content_block_delta" => {
            let delta = v.get("delta")?;
            let delta_type = json_str(delta, "type");
            match delta_type {
                "text_delta" => {
                    let text = json_str(delta, "text").to_string();
                    Some(Ok(StreamEvent::TextDelta { delta: text }))
                }
                "thinking_delta" => {
                    let thinking = json_str(delta, "thinking").to_string();
                    Some(Ok(StreamEvent::ThinkingDelta { delta: thinking }))
                }
                "input_json_delta" => {
                    let partial = json_str(delta, "partial_json").to_string();
                    let tool_use_id = v
                        .get("index")
                        .and_then(|i| i.as_u64())
                        .map(|i| format!("tool_{}", i))
                        .unwrap_or_default();
                    Some(Ok(StreamEvent::ToolInputDelta {
                        tool_use_id,
                        partial_json: partial,
                    }))
                }
                _ => None,
            }
        }
        "content_block_stop" => None,
        "message_delta" => {
            let delta = v.get("delta")?;
            let usage = v.get("usage").map(parse_anthropic_usage).unwrap_or_default();
            let stop_reason = match json_str(delta, "stop_reason") {
                "end_turn" => StopReason::EndTurn,
                "tool_use" => StopReason::ToolUse,
                "max_tokens" => StopReason::MaxTokens,
                "stop_sequence" => StopReason::StopSequence,
                _ => StopReason::EndTurn,
            };
            Some(Ok(StreamEvent::StreamEnd { usage, stop_reason }))
        }
        "message_stop" => None,
        "ping" => None,
        "error" => {
            let error_msg = v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            Some(Err(LlmError::Other {
                provider: "anthropic".to_string(),
                message: error_msg,
            }))
        }
        _ => {
            debug!("Anthropic: unhandled SSE event type: {}", event_type);
            None
        }
    }
}
