//! Ollama provider — local inference via OpenAI-compatible API.
//!
//! Ollama exposes an OpenAI-compatible `/v1/chat/completions` endpoint
//! at `localhost:11434` by default. No API key is required.
//!
//! This provider is ideal for:
//! - Offline/air-gapped environments
//! - Development and testing without API costs
//! - Running models locally (Llama, Mistral, CodeLlama, etc.)
//!
//! ## Configuration
//!
//! ```toml
//! [llm.providers.ollama]
//! base_url = "http://localhost:11434"
//! model = "llama3.2"
//! ```

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
};
use truenorth_core::types::message::MessageRole;

use crate::providers::openai::{extract_text, parse_openai_usage};
use crate::stream::{collect_sse_data_lines, json_str, try_parse_json};

#[allow(dead_code)]
const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Ollama local inference provider.
///
/// Uses the OpenAI-compatible API that Ollama exposes at `/v1/chat/completions`.
/// No API key is needed — Ollama authenticates based on localhost access.
#[derive(Debug)]
pub struct OllamaProvider {
    base_url: String,
    model: String,
    client: Client,
    capabilities: ProviderCapabilities,
    is_available: Arc<AtomicBool>,
    is_exhausted: Arc<AtomicBool>,
    success_count: Arc<AtomicU64>,
    failure_count: Arc<AtomicU64>,
}

impl OllamaProvider {
    /// Creates a new `OllamaProvider`.
    ///
    /// `base_url` defaults to `http://localhost:11434` if empty.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let base_url_str: String = base_url.into();
        let base_url_str = if base_url_str.is_empty() {
            DEFAULT_BASE_URL.to_string()
        } else {
            base_url_str.trim_end_matches('/').to_string()
        };

        let model_str: String = model.into();

        Self {
            base_url: base_url_str.clone(),
            model: model_str.clone(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(600)) // local models can be slow
                .build()
                .expect("Failed to build HTTP client"),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calling: true,
                supports_vision: model_str.contains("vision") || model_str.contains("llava"),
                supports_thinking: false,
                max_context_tokens: 128_000, // varies by model; conservative default
                max_output_tokens: 4096,
                output_modalities: vec!["text".to_string()],
                provider_name: "ollama".to_string(),
                model_name: model_str,
            },
            is_available: Arc::new(AtomicBool::new(true)),
            is_exhausted: Arc::new(AtomicBool::new(false)),
            success_count: Arc::new(AtomicU64::new(0)),
            failure_count: Arc::new(AtomicU64::new(0)),
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url)
    }

    fn build_request_body(&self, request: &CompletionRequest) -> Value {
        let mut messages: Vec<Value> = Vec::new();

        for msg in &request.messages {
            match msg.role {
                MessageRole::System => {
                    let text = extract_text(&msg.content);
                    messages.push(json!({ "role": "system", "content": text }));
                }
                MessageRole::User => {
                    let text = extract_text(&msg.content);
                    messages.push(json!({ "role": "user", "content": text }));
                }
                MessageRole::Assistant => {
                    let text = extract_text(&msg.content);
                    messages.push(json!({ "role": "assistant", "content": text }));
                }
                MessageRole::Tool => {
                    let text = extract_text(&msg.content);
                    messages.push(json!({ "role": "tool", "content": text }));
                }
            }
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": request.parameters.max_tokens,
            "stream": request.stream,
        });

        if let Some(temp) = request.parameters.temperature {
            body["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.parameters.top_p {
            body["top_p"] = json!(top_p);
        }

        if !request.parameters.stop_sequences.is_empty() {
            body["stop"] = json!(request.parameters.stop_sequences);
        }

        // Tool definitions (Ollama supports OpenAI tool format)
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

        body
    }

    fn map_error(&self, status: u16, body: &str) -> LlmError {
        match status {
            404 => LlmError::Other {
                provider: "ollama".to_string(),
                message: format!(
                    "Model '{}' not found. Run: ollama pull {}",
                    self.model, self.model
                ),
            },
            503 | 500 => LlmError::Other {
                provider: "ollama".to_string(),
                message: format!("Ollama service error (HTTP {}). Is Ollama running?", status),
            },
            _ => LlmError::Other {
                provider: "ollama".to_string(),
                message: format!("HTTP {}: {}", status, body.chars().take(200).collect::<String>()),
            },
        }
    }

    fn parse_response(&self, body: &Value, latency_ms: u64) -> Result<CompletionResponse, LlmError> {
        use truenorth_core::types::message::ContentBlock;

        let choices = body
            .get("choices")
            .and_then(|c| c.as_array())
            .ok_or_else(|| LlmError::MalformedResponse {
                provider: "ollama".to_string(),
                detail: "No 'choices' in response".to_string(),
            })?;

        if choices.is_empty() {
            return Err(LlmError::MalformedResponse {
                provider: "ollama".to_string(),
                detail: "Empty 'choices' array".to_string(),
            });
        }

        let choice = &choices[0];
        let message = choice.get("message").ok_or_else(|| LlmError::MalformedResponse {
            provider: "ollama".to_string(),
            detail: "No 'message' in choice".to_string(),
        })?;

        let mut content_blocks: Vec<ContentBlock> = Vec::new();

        if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
            if !text.is_empty() {
                content_blocks.push(ContentBlock::Text { text: text.to_string() });
            }
        }

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
                let input: Value = serde_json::from_str(arguments_str)
                    .unwrap_or(Value::Object(Default::default()));
                content_blocks.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        let usage = body.get("usage").map(parse_openai_usage).unwrap_or_default();

        let stop_reason = match choice.get("finish_reason").and_then(|r| r.as_str()).unwrap_or("stop") {
            "stop" => StopReason::EndTurn,
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        Ok(CompletionResponse {
            content: content_blocks,
            usage,
            provider: "ollama".to_string(),
            model: self.model.clone(),
            stop_reason,
            latency_ms,
            received_at: Utc::now(),
        })
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
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
        // Ollama is local — rate limiting is unusual but we handle it anyway
        self.is_available.store(false, Ordering::SeqCst);
        let flag = Arc::clone(&self.is_available);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(retry_after_secs)).await;
            flag.store(true, Ordering::SeqCst);
        });
    }

    fn mark_exhausted(&self) {
        // For local Ollama, "exhausted" means the service is down/unreachable.
        self.is_exhausted.store(true, Ordering::SeqCst);
        warn!("Ollama provider marked exhausted — service may be unreachable");
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        let body = self.build_request_body(request);

        debug!(model = %self.model, base_url = %self.base_url, "Ollama: sending completion");

        let response = self
            .client
            .post(self.chat_url())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "ollama".to_string(),
                message: format!("Cannot reach Ollama at {}: {}", self.base_url, e),
            })?;

        let status = response.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as u64;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            self.failure_count.fetch_add(1, Ordering::SeqCst);
            let err = self.map_error(status, &error_body);
            error!(provider = "ollama", status, "Ollama completion failed");
            return Err(err);
        }

        let response_json: Value = response.json().await.map_err(|e| LlmError::MalformedResponse {
            provider: "ollama".to_string(),
            detail: format!("JSON parse error: {}", e),
        })?;

        let result = self.parse_response(&response_json, latency_ms);
        if result.is_ok() {
            self.success_count.fetch_add(1, Ordering::SeqCst);
            info!(provider = "ollama", model = %self.model, latency_ms, "Ollama completion successful");
        } else {
            self.failure_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    async fn stream(&self, request: &CompletionRequest) -> Result<StreamHandle, LlmError> {
        let mut stream_request = request.clone();
        stream_request.stream = true;
        let body = self.build_request_body(&stream_request);

        debug!(model = %self.model, "Ollama: sending streaming request");

        let response = self
            .client
            .post(self.chat_url())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "ollama".to_string(),
                message: format!("Cannot reach Ollama at {}: {}", self.base_url, e),
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(self.map_error(status, &error_body));
        }

        let data_stream = collect_sse_data_lines(response).await;
        let stream = data_stream.filter_map(|line_result| async move {
            match line_result {
                Err(e) => Some(Err(LlmError::NetworkError {
                    provider: "ollama".to_string(),
                    message: e,
                })),
                Ok(data) => parse_ollama_sse_event(&data),
            }
        });

        Ok(Box::pin(stream))
    }
}

fn parse_ollama_sse_event(data: &str) -> Option<Result<StreamEvent, LlmError>> {
    let v = try_parse_json(data)?;

    // Ollama uses OpenAI-compatible SSE format
    let choices = v.get("choices").and_then(|c| c.as_array())?;
    if choices.is_empty() {
        return None;
    }

    let choice = &choices[0];
    let delta = choice.get("delta")?;

    if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            return Some(Ok(StreamEvent::TextDelta { delta: text.to_string() }));
        }
    }

    if let Some(finish_reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
        if !finish_reason.is_empty() && finish_reason != "null" {
            let stop_reason = match finish_reason {
                "stop" => StopReason::EndTurn,
                "length" => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };
            let usage = v.get("usage").map(parse_openai_usage).unwrap_or_default();
            return Some(Ok(StreamEvent::StreamEnd { usage, stop_reason }));
        }
    }

    None
}
