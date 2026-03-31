//! Generic OpenAI-compatible provider.
//!
//! Many LLM backends expose an OpenAI-compatible `/v1/chat/completions` API:
//! - **LM Studio** — local inference with a GUI, `http://localhost:1234`
//! - **Groq** — cloud inference with extremely fast inference, `https://api.groq.com/openai`
//! - **Together AI** — cloud inference, `https://api.together.xyz/v1`
//! - **Fireworks AI** — `https://api.fireworks.ai/inference/v1`
//! - **Anyscale** — `https://api.endpoints.anyscale.com/v1`
//! - **DeepSeek** — `https://api.deepseek.com/v1`
//!
//! Configuration example for Groq:
//! ```toml
//! [llm.providers.groq]
//! base_url = "https://api.groq.com/openai/v1"
//! api_key = "${GROQ_API_KEY}"
//! model = "llama-3.3-70b-versatile"
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
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
use truenorth_core::types::message::MessageRole;

use crate::providers::openai::{extract_text, parse_openai_usage};
use crate::stream::collect_sse_data_lines;

/// Generic OpenAI-compatible LLM provider.
///
/// Point `base_url` at any OpenAI-compatible endpoint. The request/response
/// format is identical to `OpenAiProvider` — only the base URL and auth header differ.
#[derive(Debug)]
pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: String,
    model: String,
    provider_name: String,
    client: Client,
    capabilities: ProviderCapabilities,
    is_available: Arc<AtomicBool>,
    is_exhausted: Arc<AtomicBool>,
    success_count: Arc<AtomicU64>,
    failure_count: Arc<AtomicU64>,
}

impl OpenAiCompatProvider {
    /// Creates a new `OpenAiCompatProvider`.
    ///
    /// - `base_url`: The base URL of the OpenAI-compatible API (e.g., `https://api.groq.com/openai/v1`)
    /// - `api_key`: The API key for authentication. Pass an empty string for local providers that don't require auth.
    /// - `model`: The model identifier.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let base_url_str: String = base_url.into();
        let base_url_clean = base_url_str.trim_end_matches('/').to_string();
        let model_str: String = model.into();

        // Derive a human-readable provider name from the base URL
        let provider_name = derive_provider_name(&base_url_clean);

        Self {
            base_url: base_url_clean.clone(),
            api_key: api_key.into(),
            model: model_str.clone(),
            provider_name: provider_name.clone(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("Failed to build HTTP client"),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calling: true,
                supports_vision: false, // conservative default; override if needed
                supports_thinking: false,
                max_context_tokens: 128_000, // conservative default
                max_output_tokens: 8192,
                output_modalities: vec!["text".to_string()],
                provider_name: provider_name,
                model_name: model_str,
            },
            is_available: Arc::new(AtomicBool::new(true)),
            is_exhausted: Arc::new(AtomicBool::new(false)),
            success_count: Arc::new(AtomicU64::new(0)),
            failure_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns a mutable reference to the capabilities for customization after construction.
    pub fn capabilities_mut(&mut self) -> &mut ProviderCapabilities {
        &mut self.capabilities
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
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
            body["stream_options"] = json!({ "include_usage": true });
        }

        body
    }

    fn map_error(&self, status: u16, body: &str) -> LlmError {
        let provider = &self.provider_name;
        match status {
            429 => LlmError::RateLimited {
                provider: provider.clone(),
                retry_after_secs: 60,
            },
            401 | 403 => LlmError::ApiKeyExhausted {
                provider: provider.clone(),
            },
            _ => LlmError::Other {
                provider: provider.clone(),
                message: format!("HTTP {}: {}", status, body.chars().take(200).collect::<String>()),
            },
        }
    }

    fn parse_response(&self, body: &Value, latency_ms: u64) -> Result<CompletionResponse, LlmError> {
        use chrono::Utc;
        use truenorth_core::types::message::ContentBlock;

        let choices = body
            .get("choices")
            .and_then(|c| c.as_array())
            .ok_or_else(|| LlmError::MalformedResponse {
                provider: self.provider_name.clone(),
                detail: "No 'choices' in response".to_string(),
            })?;

        if choices.is_empty() {
            return Err(LlmError::MalformedResponse {
                provider: self.provider_name.clone(),
                detail: "Empty 'choices' array".to_string(),
            });
        }

        let choice = &choices[0];
        let message = choice.get("message").ok_or_else(|| LlmError::MalformedResponse {
            provider: self.provider_name.clone(),
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
                let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                let name = tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let args_str = tc.get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).unwrap_or_default();
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
            provider: self.provider_name.clone(),
            model: body.get("model").and_then(|m| m.as_str()).unwrap_or(&self.model).to_string(),
            stop_reason,
            latency_ms,
            received_at: Utc::now(),
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.provider_name
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
        });
    }

    fn mark_exhausted(&self) {
        self.is_exhausted.store(true, Ordering::SeqCst);
        warn!(provider = %self.provider_name, "OpenAI-compat provider marked exhausted");
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        let body = self.build_request_body(request);
        let url = self.chat_url();

        debug!(provider = %self.provider_name, model = %self.model, "OpenAI-compat: sending completion");

        let mut req = self.client.post(&url).header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: self.provider_name.clone(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as u64;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            self.failure_count.fetch_add(1, Ordering::SeqCst);
            return Err(self.map_error(status, &error_body));
        }

        let response_json: Value = response.json().await.map_err(|e| LlmError::MalformedResponse {
            provider: self.provider_name.clone(),
            detail: format!("JSON parse error: {}", e),
        })?;

        let result = self.parse_response(&response_json, latency_ms);
        if result.is_ok() {
            self.success_count.fetch_add(1, Ordering::SeqCst);
            info!(provider = %self.provider_name, latency_ms, "OpenAI-compat completion successful");
        } else {
            self.failure_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    async fn stream(&self, request: &CompletionRequest) -> Result<StreamHandle, LlmError> {
        let mut stream_request = request.clone();
        stream_request.stream = true;
        let body = self.build_request_body(&stream_request);
        let url = self.chat_url();

        let mut req = self.client.post(&url).header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: self.provider_name.clone(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(self.map_error(status, &error_body));
        }

        let provider_name = self.provider_name.clone();
        let data_stream = collect_sse_data_lines(response).await;
        let stream = data_stream.filter_map(move |line_result| {
            let p = provider_name.clone();
            async move {
                match line_result {
                    Err(e) => Some(Err(LlmError::NetworkError { provider: p, message: e })),
                    Ok(data) => parse_openai_compat_sse_event(&data, &p),
                }
            }
        });

        Ok(Box::pin(stream))
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn derive_provider_name(base_url: &str) -> String {
    let lower = base_url.to_lowercase();
    if lower.contains("groq") {
        "groq".to_string()
    } else if lower.contains("together") {
        "together_ai".to_string()
    } else if lower.contains("fireworks") {
        "fireworks".to_string()
    } else if lower.contains("deepseek") {
        "deepseek".to_string()
    } else if lower.contains("anyscale") {
        "anyscale".to_string()
    } else if lower.contains("localhost") || lower.contains("127.0.0.1") {
        "local_compat".to_string()
    } else {
        // Extract domain from URL without pulling in the url crate
        base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .map(|host| host.split(':').next().unwrap_or(host).to_string())
            .unwrap_or_else(|| "openai_compat".to_string())
    }
}

fn parse_openai_compat_sse_event(data: &str, provider: &str) -> Option<Result<StreamEvent, LlmError>> {
    use crate::stream::try_parse_json;

    let v = try_parse_json(data)?;
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
                "tool_calls" => StopReason::ToolUse,
                "length" => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };
            let usage = v.get("usage").map(parse_openai_usage).unwrap_or_default();
            return Some(Ok(StreamEvent::StreamEnd { usage, stop_reason }));
        }
    }

    None
}
