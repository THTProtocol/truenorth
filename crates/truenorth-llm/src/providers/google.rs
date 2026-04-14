//! Google Gemini provider — implements `LlmProvider` via the GenerateContent API.
//!
//! Supports:
//! - Synchronous completions (non-streaming)
//! - Streaming via the streamGenerateContent endpoint
//! - Tool use via function declarations
//! - Vision inputs (multimodal content parts)
//!
//! ## API reference
//!
//! Base URL: `https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent`
//! Auth: `key=<api_key>` query parameter
//! Streaming: `:streamGenerateContent?alt=sse&key=<api_key>`

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

use crate::stream::{collect_sse_data_lines, try_parse_json};

const GOOGLE_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
#[allow(dead_code)]
const DEFAULT_MODEL: &str = "gemini-2.0-flash-exp";

/// Google Gemini provider.
#[derive(Debug)]
pub struct GoogleProvider {
    api_key: String,
    model: String,
    client: Client,
    capabilities: ProviderCapabilities,
    is_available: Arc<AtomicBool>,
    is_exhausted: Arc<AtomicBool>,
    success_count: Arc<AtomicU64>,
    failure_count: Arc<AtomicU64>,
}

impl GoogleProvider {
    /// Creates a new `GoogleProvider` with the given API key and model name.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model_str: String = model.into();
        let supports_thinking = model_str.contains("thinking") || model_str.contains("2.5");

        Self {
            api_key: api_key.into(),
            model: model_str.clone(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("Failed to build HTTP client"),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calling: true,
                supports_vision: true,
                supports_thinking,
                max_context_tokens: 1_000_000,
                max_output_tokens: 8192,
                output_modalities: vec!["text".to_string()],
                provider_name: "google".to_string(),
                model_name: model_str,
            },
            is_available: Arc::new(AtomicBool::new(true)),
            is_exhausted: Arc::new(AtomicBool::new(false)),
            success_count: Arc::new(AtomicU64::new(0)),
            failure_count: Arc::new(AtomicU64::new(0)),
        }
    }

    fn generate_url(&self, streaming: bool) -> String {
        let endpoint = if streaming {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let query = if streaming {
            format!("?alt=sse&key={}", self.api_key)
        } else {
            format!("?key={}", self.api_key)
        };
        format!("{}/{}/:{}{}", GOOGLE_API_BASE, self.model, endpoint, query)
    }

    fn build_request_body(&self, request: &CompletionRequest) -> Value {
        let mut system_instruction: Option<String> = None;
        let mut contents: Vec<Value> = Vec::new();

        for msg in &request.messages {
            match msg.role {
                MessageRole::System => {
                    let text = extract_text_content(&msg.content);
                    system_instruction = Some(text);
                }
                MessageRole::User => {
                    let parts = build_google_parts(&msg.content);
                    contents.push(json!({ "role": "user", "parts": parts }));
                }
                MessageRole::Assistant => {
                    let parts = build_google_assistant_parts(&msg.content);
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
                MessageRole::Tool => {
                    // Tool results go as user messages with function_response parts
                    let parts = build_google_tool_result_parts(&msg.content);
                    contents.push(json!({ "role": "user", "parts": parts }));
                }
            }
        }

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": request.parameters.max_tokens,
            }
        });

        if let Some(sys) = system_instruction {
            body["systemInstruction"] = json!({
                "parts": [{ "text": sys }]
            });
        }

        if let Some(temp) = request.parameters.temperature {
            body["generationConfig"]["temperature"] = json!(temp);
        }

        if let Some(top_p) = request.parameters.top_p {
            body["generationConfig"]["topP"] = json!(top_p);
        }

        if !request.parameters.stop_sequences.is_empty() {
            body["generationConfig"]["stopSequences"] = json!(request.parameters.stop_sequences);
        }

        // Tool definitions
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                let function_declarations: Vec<Value> = tools
                    .iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }))
                    .collect();
                body["tools"] = json!([{ "functionDeclarations": function_declarations }]);
            }
        }

        body
    }

    fn map_http_error(&self, status: u16, body: &str) -> LlmError {
        match status {
            429 => LlmError::RateLimited {
                provider: "google".to_string(),
                retry_after_secs: 60,
            },
            400 if body.contains("API_KEY") || body.contains("invalid") => {
                LlmError::ApiKeyExhausted {
                    provider: "google".to_string(),
                }
            }
            400 => {
                if body.contains("SAFETY") || body.contains("safety") {
                    LlmError::ModelRefusal {
                        reason: extract_google_error(body),
                    }
                } else {
                    LlmError::MalformedResponse {
                        provider: "google".to_string(),
                        detail: extract_google_error(body),
                    }
                }
            }
            403 => LlmError::ApiKeyExhausted {
                provider: "google".to_string(),
            },
            _ => LlmError::Other {
                provider: "google".to_string(),
                message: format!("HTTP {}: {}", status, extract_google_error(body)),
            },
        }
    }

    fn parse_response(&self, body: &Value, latency_ms: u64) -> Result<CompletionResponse, LlmError> {
        let candidates = body
            .get("candidates")
            .and_then(|c| c.as_array())
            .ok_or_else(|| LlmError::MalformedResponse {
                provider: "google".to_string(),
                detail: "No 'candidates' in response".to_string(),
            })?;

        if candidates.is_empty() {
            // Check for safety block
            if let Some(prompt_feedback) = body.get("promptFeedback") {
                if let Some(block_reason) = prompt_feedback.get("blockReason").and_then(|r| r.as_str()) {
                    return Err(LlmError::ModelRefusal {
                        reason: format!("Google safety block: {}", block_reason),
                    });
                }
            }
            return Err(LlmError::MalformedResponse {
                provider: "google".to_string(),
                detail: "Empty 'candidates' array".to_string(),
            });
        }

        let candidate = &candidates[0];
        let content_blocks = parse_google_candidate_content(candidate);

        let usage_metadata = body.get("usageMetadata");
        let usage = usage_metadata.map(parse_google_usage).unwrap_or_default();

        let stop_reason = match candidate
            .get("finishReason")
            .and_then(|r| r.as_str())
            .unwrap_or("STOP")
        {
            "STOP" => StopReason::EndTurn,
            "MAX_TOKENS" => StopReason::MaxTokens,
            "SAFETY" => StopReason::ContentFilter,
            "TOOL_USE" | "FUNCTION_CALL" => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };

        Ok(CompletionResponse {
            content: content_blocks,
            usage,
            provider: "google".to_string(),
            model: self.model.clone(),
            stop_reason,
            latency_ms,
            received_at: Utc::now(),
        })
    }
}

#[async_trait]
impl LlmProvider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
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
            info!("Google rate limit cleared after {}s", retry_after_secs);
        });
    }

    fn mark_exhausted(&self) {
        self.is_exhausted.store(true, Ordering::SeqCst);
        warn!("Google provider marked exhausted");
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        let url = self.generate_url(false);
        let body = self.build_request_body(request);

        debug!(model = %self.model, request_id = %request.request_id, "Google: sending completion");

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "google".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as u64;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            self.failure_count.fetch_add(1, Ordering::SeqCst);
            let err = self.map_http_error(status, &error_body);
            error!(provider = "google", status = status, "Google completion failed");
            return Err(err);
        }

        let response_json: Value = response.json().await.map_err(|e| LlmError::MalformedResponse {
            provider: "google".to_string(),
            detail: format!("JSON parse error: {}", e),
        })?;

        let result = self.parse_response(&response_json, latency_ms);
        if result.is_ok() {
            self.success_count.fetch_add(1, Ordering::SeqCst);
            info!(provider = "google", model = %self.model, latency_ms, "Google completion successful");
        } else {
            self.failure_count.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    async fn stream(&self, request: &CompletionRequest) -> Result<StreamHandle, LlmError> {
        let url = self.generate_url(true);
        let body = self.build_request_body(request);

        debug!(model = %self.model, "Google: sending streaming request");

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "google".to_string(),
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(self.map_http_error(status, &error_body));
        }

        let data_stream = collect_sse_data_lines(response).await;
        let model = self.model.clone();

        let stream = data_stream.filter_map(move |line_result| {
            let m = model.clone();
            async move {
                match line_result {
                    Err(e) => Some(Err(LlmError::NetworkError { provider: "google".to_string(), message: e })),
                    Ok(data) => parse_google_sse_event(&data, &m),
                }
            }
        });

        Ok(Box::pin(stream))
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn extract_text_content(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_google_parts(content: &[ContentBlock]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(json!({ "text": text })),
            ContentBlock::Image { mime_type, data } => Some(json!({
                "inlineData": { "mimeType": mime_type, "data": data }
            })),
            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                let text = content.iter()
                    .filter_map(|b| match b { ContentBlock::Text { text } => Some(text.as_str()), _ => None })
                    .collect::<Vec<_>>().join("\n");
                Some(json!({
                    "functionResponse": {
                        "name": tool_use_id,
                        "response": { "result": text }
                    }
                }))
            }
            _ => None,
        })
        .collect()
}

fn build_google_assistant_parts(content: &[ContentBlock]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(json!({ "text": text })),
            ContentBlock::ToolUse { id: _, name, input } => Some(json!({
                "functionCall": { "name": name, "args": input }
            })),
            ContentBlock::Thinking { thinking, .. } => {
                // Graceful degradation: include thinking as text prefix
                Some(json!({ "text": format!("[Reasoning: {}]", thinking) }))
            }
            _ => None,
        })
        .collect()
}

fn build_google_tool_result_parts(content: &[ContentBlock]) -> Vec<Value> {
    build_google_parts(content)
}

fn parse_google_candidate_content(candidate: &Value) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    let parts = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());

    if let Some(parts) = parts {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                blocks.push(ContentBlock::Text { text: text.to_string() });
            } else if let Some(fc) = part.get("functionCall") {
                let id = fc.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let name = id.clone();
                let input = fc.get("args").cloned().unwrap_or_default();
                blocks.push(ContentBlock::ToolUse { id, name, input });
            }
        }
    }
    blocks
}

fn parse_google_usage(usage: &Value) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        output_tokens: usage.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cache_read_tokens: usage.get("cachedContentTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cache_write_tokens: 0,
        thinking_tokens: usage.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    }
}

fn extract_google_error(body: &str) -> String {
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

fn parse_google_sse_event(data: &str, _model: &str) -> Option<Result<StreamEvent, LlmError>> {
    let v = try_parse_json(data)?;

    let candidates = v.get("candidates").and_then(|c| c.as_array())?;
    if candidates.is_empty() {
        // Check usageMetadata for stream end
        if v.get("usageMetadata").is_some() {
            let usage = v.get("usageMetadata").map(parse_google_usage).unwrap_or_default();
            return Some(Ok(StreamEvent::StreamEnd {
                usage,
                stop_reason: StopReason::EndTurn,
            }));
        }
        return None;
    }

    let candidate = &candidates[0];

    // Check for finish
    if let Some(finish_reason) = candidate.get("finishReason").and_then(|r| r.as_str()) {
        if finish_reason != "FINISH_REASON_UNSPECIFIED" && !finish_reason.is_empty() {
            let stop_reason = match finish_reason {
                "STOP" => StopReason::EndTurn,
                "MAX_TOKENS" => StopReason::MaxTokens,
                "SAFETY" => StopReason::ContentFilter,
                _ => StopReason::EndTurn,
            };
            let usage = v.get("usageMetadata").map(parse_google_usage).unwrap_or_default();
            return Some(Ok(StreamEvent::StreamEnd { usage, stop_reason }));
        }
    }

    let parts = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())?;

    for part in parts {
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            if !text.is_empty() {
                return Some(Ok(StreamEvent::TextDelta { delta: text.to_string() }));
            }
        }
        if let Some(fc) = part.get("functionCall") {
            let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let id = name.clone();
            return Some(Ok(StreamEvent::ToolUseStart { id, name }));
        }
    }

    None
}
