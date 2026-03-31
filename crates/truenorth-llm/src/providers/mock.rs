//! Mock LLM provider for testing.
//!
//! The `MockProvider` allows router unit tests and integration tests to run
//! without real API credentials. It supports:
//!
//! - Configurable deterministic responses
//! - Simulated rate limit failures (after N calls)
//! - Simulated exhaustion
//! - Simulated network errors
//! - Configurable latency simulation
//! - Recording of all calls for assertion in tests
//!
//! ## Example usage
//!
//! ```rust
//! use truenorth_llm::providers::MockProvider;
//!
//! let mut mock = MockProvider::new();
//! mock.set_response("Hello from mock!");
//! mock.simulate_rate_limit_after(2); // fail with RateLimited after 2 calls
//! ```

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::stream;
use tracing::{debug, info};
use uuid::Uuid;

use truenorth_core::error::LlmError;
use truenorth_core::traits::llm_provider::{LlmProvider, StreamHandle};
use truenorth_core::types::llm::{
    CompletionRequest, CompletionResponse, ProviderCapabilities, StopReason, StreamEvent,
    TokenUsage,
};
use truenorth_core::types::message::ContentBlock;

/// The failure mode that the mock provider should simulate.
#[derive(Debug, Clone)]
pub enum MockFailureMode {
    /// No failure — always succeed.
    None,
    /// Return `LlmError::RateLimited` with the given retry-after seconds.
    RateLimited { retry_after_secs: u64 },
    /// Return `LlmError::ApiKeyExhausted`.
    ApiKeyExhausted,
    /// Return `LlmError::NetworkError` with the given message.
    NetworkError { message: String },
    /// Return `LlmError::ModelRefusal` with the given reason.
    ModelRefusal { reason: String },
    /// Return `LlmError::Other` after a specified number of successful calls.
    FailAfterN { successes_before_fail: u64, error: Box<MockFailureMode> },
}

/// A recorded call to the mock provider — useful for assertions in tests.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    /// The request that was made.
    pub request_id: Uuid,
    /// Whether the call succeeded.
    pub succeeded: bool,
    /// Simulated latency.
    pub latency_ms: u64,
}

/// Internal state for the mock provider (behind Mutex for thread safety).
#[derive(Debug)]
struct MockState {
    /// The response text to return on successful calls.
    response_text: String,
    /// The failure mode to simulate.
    failure_mode: MockFailureMode,
    /// All calls recorded so far.
    recorded_calls: Vec<RecordedCall>,
    /// Simulated latency in milliseconds.
    latency_ms: u64,
    /// Number of successful calls made so far.
    success_count: u64,
    /// Whether the provider is rate-limited.
    is_rate_limited: bool,
    /// Whether the provider is exhausted.
    is_exhausted: bool,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            response_text: "Mock response: I am a test LLM provider.".to_string(),
            failure_mode: MockFailureMode::None,
            recorded_calls: Vec::new(),
            latency_ms: 0,
            success_count: 0,
            is_rate_limited: false,
            is_exhausted: false,
        }
    }
}

/// Mock LLM provider for testing.
#[derive(Debug)]
pub struct MockProvider {
    name: String,
    model: String,
    capabilities: ProviderCapabilities,
    state: Arc<Mutex<MockState>>,
    is_available_override: Arc<AtomicBool>,
}

impl MockProvider {
    /// Creates a new mock provider with default settings.
    pub fn new() -> Self {
        Self::with_name("mock")
    }

    /// Creates a mock provider with a custom name (useful for testing multi-provider routing).
    pub fn with_name(name: &str) -> Self {
        Self {
            name: name.to_string(),
            model: "mock-model-1.0".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calling: true,
                supports_vision: true,
                supports_thinking: true,
                max_context_tokens: 1_000_000,
                max_output_tokens: 100_000,
                output_modalities: vec!["text".to_string()],
                provider_name: name.to_string(),
                model_name: "mock-model-1.0".to_string(),
            },
            state: Arc::new(Mutex::new(MockState::default())),
            is_available_override: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Sets the response text that will be returned by successful calls.
    pub fn set_response(&self, text: &str) {
        self.state.lock().unwrap().response_text = text.to_string();
    }

    /// Configures the provider to simulate rate limiting on every call.
    pub fn simulate_rate_limited(&self, retry_after_secs: u64) {
        self.state.lock().unwrap().failure_mode =
            MockFailureMode::RateLimited { retry_after_secs };
    }

    /// Configures the provider to fail after `n` successful calls.
    pub fn simulate_rate_limit_after(&self, n: u64) {
        self.state.lock().unwrap().failure_mode = MockFailureMode::FailAfterN {
            successes_before_fail: n,
            error: Box::new(MockFailureMode::RateLimited { retry_after_secs: 30 }),
        };
    }

    /// Configures the provider to simulate API key exhaustion.
    pub fn simulate_exhausted(&self) {
        self.state.lock().unwrap().failure_mode = MockFailureMode::ApiKeyExhausted;
    }

    /// Configures the provider to simulate a network error.
    pub fn simulate_network_error(&self, message: &str) {
        self.state.lock().unwrap().failure_mode =
            MockFailureMode::NetworkError { message: message.to_string() };
    }

    /// Adds simulated latency to each response.
    pub fn set_latency_ms(&self, ms: u64) {
        self.state.lock().unwrap().latency_ms = ms;
    }

    /// Resets the mock to its default state.
    pub fn reset(&self) {
        *self.state.lock().unwrap() = MockState::default();
        self.is_available_override.store(true, Ordering::SeqCst);
    }

    /// Returns all calls recorded by this mock.
    pub fn recorded_calls(&self) -> Vec<RecordedCall> {
        self.state.lock().unwrap().recorded_calls.clone()
    }

    /// Returns the number of successful calls made.
    pub fn success_count(&self) -> u64 {
        self.state.lock().unwrap().success_count
    }

    /// Directly overrides availability (bypasses failure mode logic).
    pub fn set_available(&self, available: bool) {
        self.is_available_override.store(available, Ordering::SeqCst);
    }

    /// Builds the simulated `CompletionResponse`.
    fn build_success_response(&self, request: &CompletionRequest, latency_ms: u64) -> CompletionResponse {
        let mut state = self.state.lock().unwrap();
        state.success_count += 1;
        state.recorded_calls.push(RecordedCall {
            request_id: request.request_id,
            succeeded: true,
            latency_ms,
        });

        let text = state.response_text.clone();

        CompletionResponse {
            content: vec![ContentBlock::Text { text }],
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                thinking_tokens: 0,
            },
            provider: self.name.clone(),
            model: self.model.clone(),
            stop_reason: StopReason::EndTurn,
            latency_ms,
            received_at: Utc::now(),
        }
    }

    /// Determines whether the current call should fail based on the failure mode.
    fn check_failure(&self) -> Option<LlmError> {
        let state = self.state.lock().unwrap();
        match &state.failure_mode {
            MockFailureMode::None => None,
            MockFailureMode::RateLimited { retry_after_secs } => {
                Some(LlmError::RateLimited {
                    provider: self.name.clone(),
                    retry_after_secs: *retry_after_secs,
                })
            }
            MockFailureMode::ApiKeyExhausted => {
                Some(LlmError::ApiKeyExhausted { provider: self.name.clone() })
            }
            MockFailureMode::NetworkError { message } => {
                Some(LlmError::NetworkError {
                    provider: self.name.clone(),
                    message: message.clone(),
                })
            }
            MockFailureMode::ModelRefusal { reason } => {
                Some(LlmError::ModelRefusal { reason: reason.clone() })
            }
            MockFailureMode::FailAfterN { successes_before_fail, error } => {
                if state.success_count >= *successes_before_fail {
                    // Clone inner mode before releasing lock to avoid borrow conflict
                    let inner_error = error.clone();
                    drop(state);
                    self.simulate_failure_from_mode(&inner_error)
                } else {
                    None
                }
            }
        }
    }

    fn simulate_failure_from_mode(&self, mode: &MockFailureMode) -> Option<LlmError> {
        match mode {
            MockFailureMode::RateLimited { retry_after_secs } => {
                Some(LlmError::RateLimited {
                    provider: self.name.clone(),
                    retry_after_secs: *retry_after_secs,
                })
            }
            MockFailureMode::ApiKeyExhausted => {
                Some(LlmError::ApiKeyExhausted { provider: self.name.clone() })
            }
            MockFailureMode::NetworkError { message } => {
                Some(LlmError::NetworkError {
                    provider: self.name.clone(),
                    message: message.clone(),
                })
            }
            _ => Some(LlmError::Other {
                provider: self.name.clone(),
                message: "Mock failure".to_string(),
            }),
        }
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    fn is_available(&self) -> bool {
        if !self.is_available_override.load(Ordering::SeqCst) {
            return false;
        }
        let state = self.state.lock().unwrap();
        !state.is_rate_limited && !state.is_exhausted
    }

    fn mark_rate_limited(&self, retry_after_secs: u64) {
        {
            let mut state = self.state.lock().unwrap();
            state.is_rate_limited = true;
        }
        let state_clone = Arc::clone(&self.state);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(retry_after_secs)).await;
            state_clone.lock().unwrap().is_rate_limited = false;
        });
    }

    fn mark_exhausted(&self) {
        self.state.lock().unwrap().is_exhausted = true;
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let latency_ms = {
            let state = self.state.lock().unwrap();
            state.latency_ms
        };

        if latency_ms > 0 {
            tokio::time::sleep(Duration::from_millis(latency_ms)).await;
        }

        debug!(
            provider = %self.name,
            request_id = %request.request_id,
            "MockProvider: processing completion request"
        );

        if let Some(err) = self.check_failure() {
            let mut state = self.state.lock().unwrap();
            state.recorded_calls.push(RecordedCall {
                request_id: request.request_id,
                succeeded: false,
                latency_ms,
            });
            return Err(err);
        }

        let response = self.build_success_response(request, latency_ms);
        info!(provider = %self.name, "MockProvider: returning success response");
        Ok(response)
    }

    async fn stream(&self, request: &CompletionRequest) -> Result<StreamHandle, LlmError> {
        let latency_ms = {
            let state = self.state.lock().unwrap();
            state.latency_ms
        };

        if latency_ms > 0 {
            tokio::time::sleep(Duration::from_millis(latency_ms / 2)).await;
        }

        if let Some(err) = self.check_failure() {
            return Err(err);
        }

        let response_text = self.state.lock().unwrap().response_text.clone();

        // Split the response into 5-character chunks to simulate streaming
        let chunks: Vec<String> = response_text
            .chars()
            .collect::<Vec<_>>()
            .chunks(5)
            .map(|c| c.iter().collect())
            .collect();

        let chunk_delay = if latency_ms > 0 { latency_ms / (chunks.len().max(1) as u64) } else { 0 };

        let mut events: Vec<Result<StreamEvent, LlmError>> = chunks
            .into_iter()
            .map(|chunk| Ok(StreamEvent::TextDelta { delta: chunk }))
            .collect();

        events.push(Ok(StreamEvent::StreamEnd {
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                thinking_tokens: 0,
            },
            stop_reason: StopReason::EndTurn,
        }));

        let stream = stream::iter(events);
        Ok(Box::pin(stream))
    }

    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        // Return deterministic 4-dimensional mock embeddings based on text hash
        Ok(texts.iter().map(|text| mock_embed_text(text)).collect())
    }
}

/// Generates a deterministic mock embedding for a text string.
///
/// Uses a simple polynomial hash to produce a 4-dimensional unit vector.
/// Not semantically meaningful — only for testing that embedding round-trips work.
fn mock_embed_text(text: &str) -> Vec<f32> {
    let hash: u64 = text.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as u64)
    });

    let a = ((hash & 0xFF) as f32) / 255.0 - 0.5;
    let b = (((hash >> 8) & 0xFF) as f32) / 255.0 - 0.5;
    let c = (((hash >> 16) & 0xFF) as f32) / 255.0 - 0.5;
    let d = (((hash >> 24) & 0xFF) as f32) / 255.0 - 0.5;

    let norm = (a * a + b * b + c * c + d * d).sqrt().max(1e-8);
    vec![a / norm, b / norm, c / norm, d / norm]
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::llm::CompletionParameters;

    fn make_request() -> CompletionRequest {
        CompletionRequest {
            request_id: Uuid::new_v4(),
            messages: vec![],
            tools: None,
            parameters: CompletionParameters::default(),
            session_id: Uuid::new_v4(),
            stream: false,
            required_capabilities: vec![],
        }
    }

    #[tokio::test]
    async fn test_mock_success() {
        let mock = MockProvider::new();
        mock.set_response("Test response");
        let result = mock.complete(&make_request()).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text == "Test response")));
    }

    #[tokio::test]
    async fn test_mock_rate_limited() {
        let mock = MockProvider::new();
        mock.simulate_rate_limited(30);
        let result = mock.complete(&make_request()).await;
        assert!(matches!(result, Err(LlmError::RateLimited { retry_after_secs: 30, .. })));
    }

    #[tokio::test]
    async fn test_mock_fail_after_n() {
        let mock = MockProvider::new();
        mock.simulate_rate_limit_after(2);

        let r1 = mock.complete(&make_request()).await;
        assert!(r1.is_ok(), "First call should succeed");

        let r2 = mock.complete(&make_request()).await;
        assert!(r2.is_ok(), "Second call should succeed");

        let r3 = mock.complete(&make_request()).await;
        assert!(r3.is_err(), "Third call should fail");
        assert!(matches!(r3, Err(LlmError::RateLimited { .. })));
    }

    #[tokio::test]
    async fn test_mock_exhausted() {
        let mock = MockProvider::new();
        mock.simulate_exhausted();
        let result = mock.complete(&make_request()).await;
        assert!(matches!(result, Err(LlmError::ApiKeyExhausted { .. })));
    }

    #[tokio::test]
    async fn test_mock_recorded_calls() {
        let mock = MockProvider::new();
        let _ = mock.complete(&make_request()).await;
        let _ = mock.complete(&make_request()).await;
        assert_eq!(mock.recorded_calls().len(), 2);
        assert_eq!(mock.success_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_stream() {
        use futures::StreamExt;
        let mock = MockProvider::new();
        mock.set_response("Hello stream");
        let result = mock.stream(&make_request()).await;
        assert!(result.is_ok());

        let events: Vec<_> = result.unwrap().collect().await;
        // Should have text deltas + final StreamEnd
        assert!(!events.is_empty());
        assert!(events.iter().any(|e| matches!(e, Ok(StreamEvent::StreamEnd { .. }))));
    }

    #[test]
    fn test_mock_embed_deterministic() {
        let v1 = mock_embed_text("hello world");
        let v2 = mock_embed_text("hello world");
        let v3 = mock_embed_text("different text");

        assert_eq!(v1, v2, "Same input should produce same embedding");
        assert_ne!(v1, v3, "Different input should produce different embedding");

        // Check unit length (approximately)
        let norm: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001, "Should be approximately unit length");
    }
}
