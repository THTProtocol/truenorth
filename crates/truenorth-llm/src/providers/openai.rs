//! OpenAI provider — implements `LlmProvider` via the Chat Completions API.
//!
//! Supports:
//! - Synchronous completions
//! - Streaming via SSE (`stream: true`)
//! - Function calling / tool_calls
//! - o-series reasoning models (reasoning_effort parameter)
//!
//! ## API reference
//!
//! Base URL: `https://api.openai.com/v1/chat/completions`
//! Auth header: `Authorization: Bearer <key>`

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
    CompletionRequest, CompletionResponse, ProviderCapabilities, StopReason, StreamEvent,
    TokenUsage,
};
use truenorth_core::types::message::{ContentBlock, MessageRole};

use crate::stream::{collect_sse_data_lines, json_str, try_parse_json};

const OPENAI_API_BASE: &str = "https://api.openai.com/v1/chat/completions";
#[allow(dead_code)]
const DEFAULT_MODEL: &str = "gpt-4o";

/// OpenAI Chat Completions provider.
///
/// Compatible with the standard OpenAI API. For OpenAI-compatible third-party
/// providers (LM Studio, Groq, Together AI), use `OpenAiCompatProvider` instead.
#[derive(Debug)]
pub struct OpenAiProvider {
    api_key: String,
    model: String,
    client: Client,
    capabilities: ProviderCapabilities,
    is_available: Arc<AtomicBool>,
    is_exhausted: Arc<AtomicBool>,
    success_count: Arc<AtomicU64>,
    failure_count: Arc<AtomicU64>,
}

impl OpenAiProvider {
    /// Creates a new `OpenAiProvider` with the given API key and model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model_str: String = model.into();
        let is_reasoning = model_str.starts_with("o1") || model_str.starts_with("o3") || model_str.starts_with("o4");

        Self {
            api_key: api_key.into(),
            model: model_str.clone(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("Failed to build HTTP client"),
            capabilities: ProviderCapabilities {
                supports_streaming: !is_reasoning, // o-series does not support streaming
                supports_tool_calling: true,
                supports_vision: model_str.contains("vision") || model_str.contains("gpt-4o") || model_str.starts_with("gpt-4-turbo"),
                supports_thinking: is_reasoning,
                max_context_tokens: if is_reasoning { 200_000 } else { 128_000 },
                max_output_tokens: if is_reasoning { 100_000 } else { 16_384 },
                output_modalities: vec!["text".to_string()],
                provider_name: "openai".to_string(),
                model_name: model_str,
            },
            is_available: Arc::new(AtomicBool::new(true)),
            is_exhausted: Arc::new(AtomicBool::new(false)),
            success_count: Arc::new(AtomicU64::new(0)),
            failure_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Builds the JSON request body for the OpenAI Chat Completions API.
    pub(crate) fn build_request_body(&self, request: &CompletionRequest, _base_url_override: Option<&str>) -> Value {
        let mut messages: Vec<Value> = Vec::new();

        for msg in &request.messages {
            match msg.role {
                MessageRole::System => {
                    let text = extract_text(&msg.content);
                    messages.push(json!({ "role": "system", "content": text }));
                }
                MessageRole::User => {
                    messages.push(build_openai_user_message(&msg.content));
                }
                MessageRole::Assistant => {
                    messages.push(build_openai_assistant_message(&msg.content));
                }
                MessageRole::Tool => {
                    messages.push(build_openai_tool_result_message(&msg.content));
                }
            }
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
        });

        // o-series models don't accept temperature or max_tokens in the same way
        let is_reasoning = self.model.starts_with("o1") || self.model.starts_with("o3") || self.model.starts_with("o4");
        if is_reasoning {
            body["max_completion_tokens"] = json!(request.parameters.max_tokens);
            if request.parameters.enable_thinking {
                body["reasoning_effort"] = json!("high");
            }
        } else {
            body["max_tokens"] = json!(request.parameters.max_tokens);
            if let Some(temp) = request.parameters.temperature {
                body["temperature"] = json!(temp);
            }
            if let Some(top_p) = request.parameters.top_p {
                body["top_p"] = json!(top_p);
            }
        }

        if !request.parameters.stop_sequences.is_empty() {
            body["stop"] = json!(request.parameters.stop_sequences);
        }

        // Tool definitions
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                let openai_tools: Vec<Value> = tools
                    .iter()
                    .map(|t| json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        }
                    }))
                    .collect();
                body["tools"] = json!(openai_tools);
                body["tool_choice"] = json!("auto");
            }
        }

        if request.stream {
            body["stream"] = json!(true);
            body["stream_options"] = json!({ "include_usage": true });
        }

        body
    }

    fn map_http_error(&self, status: u16, body: &str) -> LlmError {
        match status {
            429 => {
                // Check if quota exceeded vs. rate limit
                if body.contains("quota") || body.contains("billing") {
                    LlmError::ApiKeyExhausted {
                        provider: "openai".to_string(),
                    }
                } else {
                    LlmError::RateLimited {
                        provider: "openai".to_string(),
                        retry_after_secs: parse_retry_after_from_json(body),
                    }
                }
            }
            401 | 403 => LlmError::ApiKeyExhausted {
                provider: "openai".to_string(),
            },
            400 => {
                if body.contains("content_policy") || body.contains("moderat") {
                    LlmError::ModelRefusal {
                        reason: extract_openai_error(body),
                    }
                } else {
                    LlmError::MalformedResponse {
                        provider: "openai".to_string(),
                        detail: extract_openai_error(body),
                    }
                }
            }
            _ => LlmError::Other {
                provider: "openai".to_string(),
                message: format!("HTTP {}: {}", status, extract_openai_error(body)),
            },
        }
    }

    pub(crate) fn parse_response(
        &self,
        body: &Value,
        latency_ms: u64,
        provider_name: &str,
    ) -> Result<CompletionResponse, LlmError> {
        let choices = body
            .get("choices")
            .and_then(|c| c.as_array())
            .ok_or_else(|| LlmError::MalformedResponse {
                provider: provider_name.to_string(),
                detail: "No 'choices' in response".to_string(),
            })?;

        if choices.is_empty() {
            return Err(LlmError::MalformedResponse {
                provider: provider_name.to_string(),
                detail: "Empty 'choices' array".to_string(),
            });
        }

        let choice = &choices[0];
        let message = choice.get("message").ok_or_else(|| LlmError::MalformedResponse {
            provider: provider_name.to_string(),
            detail: "No 'message' in choice".to_string(),
        })?;

        let mut content_blocks: Vec<ContentBlock> = Vec::new();

        // Regular text content
        if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
            if !text.is_empty() {
                content_blocks.push(ContentBlock::Text { text: text.to_string() });
            }
        }

        // o-series reasoning content
        if let Some(reasoning) = message.get("reasoning_content").and_then(|r| r.as_str()) {
            if !reasoning.is_empty() {
                content_blocks.push(ContentBlock::Thinking {
                    thinking: reasoning.to_string(),
                    signature: None,
                });
            }
        }

        // Tool calls
        if let Some(tool_calls) = message.get("tool_calls").and_then(|tc| tc.as_array()) {
            for tc in tool_calls {
                let id = json_str(tc, "id").to_string();
                let name = tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments_str = tc.get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(arguments_str).unwrap_or(Value::Object(Default::default()));
                content_blocks.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        let usage = body.get("usage").map(parse_openai_usage).unwrap_or_default();

        let stop_reason = match choice.get("finish_reason").and_then(|r| r.as_str()).unwrap_or("stop") {
            "stop" => StopReason::EndTurn,
            "tool_calls" | "function_call" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            "content_filter" => StopReason::ContentFilter,
            _ => StopReason::EndTurn,
        };

        let model_used = body.get("model").and_then(|m| m.as_str()).unwrap_or(&self.model).to_string();

        Ok(CompletionResponse {
            content: content_blocks,
            usage,
            provider: provider_name.to_string(),
            model: model_used,
            stop_reason,
            latency_ms,
            received_at: Utc::now(),
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
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
        let flag = Arc::clone(&self.is_available);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(retry_after_secs)).await;
            flag.store(true, Ordering::SeqCst);
            info!("OpenAI rate limit cleared after {}s", retry_after_secs);
        });
    }

    fn mark_exhausted(&self) {
        self.is_exhausted.store(true, Ordering::SeqCst);
        warn!("OpenAI provider marked exhausted — API key invalid or quota exceeded");
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        let body = self.build_request_body(request, None);

        debug!(model = %self.model, request_id = %request.request_id, "OpenAI: sending completion");

        let response = self
            .client
            .post(OPENAI_API_BASE)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "openai".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as u64;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            self.failure_count.fetch_add(1, Ordering::SeqCst);
            let err = self.map_http_error(status, &error_body);
            error!(provider = "openai", status = status, "OpenAI completion failed");
            return Err(err);
        }

        let response_json: Value = response.json().await.map_err(|e| LlmError::MalformedResponse {
            provider: "openai".to_string(),
            detail: format!("JSON parse error: {}", e),
        })?;

        let result = self.parse_response(&response_json, latency_ms, "openai");
        if result.is_ok() {
            self.success_count.fetch_add(1, Ordering::SeqCst);
            info!(provider = "openai", model = %self.model, latency_ms = latency_ms, "OpenAI completion successful");
        } else {
            self.failure_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    async fn stream(&self, request: &CompletionRequest) -> Result<StreamHandle, LlmError> {
        let mut stream_request = request.clone();
        stream_request.stream = true;
        let body = self.build_request_body(&stream_request, None);

        debug!(model = %self.model, "OpenAI: sending streaming request");

        let response = self
            .client
            .post(OPENAI_API_BASE)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "openai".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(self.map_http_error(status, &error_body));
        }

        let data_stream = collect_sse_data_lines(response).await;
        let provider = self.name().to_string();
        let stream = data_stream.filter_map(move |line_result| {
            let p = provider.clone();
            async move {
                match line_result {
                    Err(e) => Some(Err(LlmError::NetworkError { provider: p, message: e })),
                    Ok(data) => parse_openai_sse_event(&data, &p),
                }
            }
        });

        Ok(Box::pin(stream))
    }

    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let body = json!({
            "model": "text-embedding-3-small",
            "input": texts,
        });

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "openai".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(self.map_http_error(status, &error_body));
        }

        let json: Value = response.json().await.map_err(|e| LlmError::MalformedResponse {
            provider: "openai".to_string(),
            detail: e.to_string(),
        })?;

        let embeddings = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| LlmError::MalformedResponse {
                provider: "openai".to_string(),
                detail: "No 'data' array in embedding response".to_string(),
            })?
            .iter()
            .map(|item| {
                item.get("embedding")
                    .and_then(|e| e.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect())
                    .unwrap_or_default()
            })
            .collect();

        Ok(embeddings)
    }
}

// ─── SSE parsing ─────────────────────────────────────────────────────────────

fn parse_openai_sse_event(data: &str, _provider: &str) -> Option<Result<StreamEvent, LlmError>> {
    let v = try_parse_json(data)?;

    let choices = v.get("choices").and_then(|c| c.as_array())?;
    if choices.is_empty() {
        // This might be the final usage-only chunk
        if v.get("usage").is_some() {
            let usage = parse_openai_usage(v.get("usage")?);
            return Some(Ok(StreamEvent::StreamEnd {
                usage,
                stop_reason: StopReason::EndTurn,
            }));
        }
        return None;
    }

    let choice = &choices[0];
    let delta = choice.get("delta")?;

    // Tool calls delta
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
        for tc in tool_calls {
            let id = json_str(tc, "id").to_string();
            let name = tc.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(Ok(StreamEvent::ToolUseStart { id, name }));
            }
            if let Some(partial_args) = tc.get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
            {
                let tool_use_id = json_str(tc, "id").to_string();
                return Some(Ok(StreamEvent::ToolInputDelta {
                    tool_use_id,
                    partial_json: partial_args.to_string(),
                }));
            }
        }
    }

    // Text delta
    if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            return Some(Ok(StreamEvent::TextDelta { delta: text.to_string() }));
        }
    }

    // Finish reason
    if let Some(finish_reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
        if !finish_reason.is_empty() && finish_reason != "null" {
            let stop_reason = match finish_reason {
                "stop" => StopReason::EndTurn,
                "tool_calls" | "function_call" => StopReason::ToolUse,
                "length" => StopReason::MaxTokens,
                "content_filter" => StopReason::ContentFilter,
                _ => StopReason::EndTurn,
            };
            let usage = v.get("usage").map(parse_openai_usage).unwrap_or_default();
            return Some(Ok(StreamEvent::StreamEnd { usage, stop_reason }));
        }
    }

    None
}

// ─── Helper functions ─────────────────────────────────────────────────────────

pub(crate) fn extract_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_openai_user_message(content: &[ContentBlock]) -> Value {
    let parts: Vec<Value> = content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::Image { mime_type, data } => Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", mime_type, data) }
            })),
            ContentBlock::ToolResult { tool_use_id, content, is_error: _ } => {
                let text = content.iter()
                    .filter_map(|b| match b { ContentBlock::Text { text } => Some(text.as_str()), _ => None })
                    .collect::<Vec<_>>().join("\n");
                Some(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": text }))
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

fn build_openai_assistant_message(content: &[ContentBlock]) -> Value {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    for block in content {
        match block {
            ContentBlock::Text { text } => text_parts.push(text.clone()),
            ContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(input).unwrap_or_default(),
                    }
                }));
            }
            ContentBlock::Thinking { thinking, .. } => {
                // Include thinking as a comment-style prefix for OpenAI
                text_parts.insert(0, format!("[Reasoning: {}]", thinking));
            }
            _ => {}
        }
    }

    let mut msg = json!({ "role": "assistant" });
    if !text_parts.is_empty() {
        msg["content"] = json!(text_parts.join("\n"));
    } else {
        msg["content"] = json!(null);
    }
    if !tool_calls.is_empty() {
        msg["tool_calls"] = json!(tool_calls);
    }
    msg
}

fn build_openai_tool_result_message(content: &[ContentBlock]) -> Value {
    // Extract tool_use_id and result text from ToolResult blocks
    for block in content {
        if let ContentBlock::ToolResult { tool_use_id, content: result_content, is_error: _ } = block {
            let text = result_content.iter()
                .filter_map(|b| match b { ContentBlock::Text { text } => Some(text.as_str()), _ => None })
                .collect::<Vec<_>>()
                .join("\n");
            return json!({
                "role": "tool",
                "tool_call_id": tool_use_id,
                "content": text,
            });
        }
    }
    json!({ "role": "tool", "content": "" })
}

pub(crate) fn parse_openai_usage(usage: &Value) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        output_tokens: usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cache_read_tokens: usage.get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cache_write_tokens: 0,
        thinking_tokens: usage.get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    }
}

fn extract_openai_error(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = v.get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.to_string();
        }
    }
    body.chars().take(200).collect()
}

fn parse_retry_after_from_json(body: &str) -> u64 {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(secs) = v.get("error")
            .and_then(|e| e.get("retry_after"))
            .and_then(|r| r.as_u64())
        {
            return secs;
        }
    }
    60
}
