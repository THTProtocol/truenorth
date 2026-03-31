/// LlmProvider trait — the unified LLM interface.
///
/// Every LLM backend (Anthropic, OpenAI, Ollama, mock) implements this trait.
/// The router is the only caller; no other component calls providers directly.

use async_trait::async_trait;
use std::pin::Pin;

use crate::error::LlmError;
use crate::types::llm::{
    CompletionRequest, CompletionResponse, ProviderCapabilities, StreamEvent,
};

/// A boxed async stream of streaming events.
/// Pinned because streams must be polled at a fixed memory location.
pub type StreamHandle =
    Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, LlmError>> + Send>>;

/// The core LLM provider trait. Every LLM backend implements this.
///
/// Design rationale: a single normalized interface means the router can
/// swap providers transparently. The calling code never needs to know
/// whether it is talking to Anthropic, OpenAI, a local Ollama instance,
/// or a mock during testing. Provider-specific behavior (thinking traces,
/// system prompt positioning, tool call format differences) is encapsulated
/// entirely within the implementation.
#[async_trait]
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Returns the canonical name of this provider (e.g., "anthropic", "openai").
    ///
    /// Used in logging, routing decisions, and the Visual Reasoning event stream.
    fn name(&self) -> &str;

    /// Returns the specific model identifier in use (e.g., "claude-opus-4-5").
    ///
    /// Used in routing logs and token cost estimation.
    fn model(&self) -> &str;

    /// Declares what this provider can and cannot do.
    ///
    /// The router uses this to select an appropriate provider for each request.
    /// Implementations must be accurate — wrong capabilities cause silent failures.
    fn capabilities(&self) -> &ProviderCapabilities;

    /// Returns true if this provider is currently usable.
    ///
    /// False when: API key exhausted, rate limit active, or provider manually disabled.
    /// The router calls this before every routing attempt — implementations should
    /// cache availability state rather than making network calls here.
    fn is_available(&self) -> bool;

    /// Marks this provider as temporarily unavailable for the given duration.
    ///
    /// Called by the router when a rate limit is encountered.
    /// Implementations should use a background task to restore availability after
    /// the duration expires.
    fn mark_rate_limited(&self, retry_after_secs: u64);

    /// Permanently marks this provider as unavailable for the current session.
    ///
    /// Called when an API key is confirmed exhausted or invalid.
    /// Unlike rate limits, this is not reversible without a session restart.
    fn mark_exhausted(&self);

    /// Sends a completion request and waits for the full response.
    ///
    /// Use this for tool calling (where the full response is needed to extract
    /// arguments) and for short completions where streaming overhead is not warranted.
    ///
    /// Implementations must handle all HTTP-level retries internally before
    /// returning an error — a single transient network blip should not surface
    /// as `LlmError::NetworkError`.
    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError>;

    /// Sends a completion request and returns a stream of partial events.
    ///
    /// Use this for all user-visible text generation where perceived latency matters.
    ///
    /// The stream MUST terminate with a `StreamEvent::StreamEnd` event. If the
    /// connection is interrupted, a `StreamEvent::StreamError` must be emitted
    /// as the final stream item.
    async fn stream(
        &self,
        request: &CompletionRequest,
    ) -> Result<StreamHandle, LlmError>;

    /// Generates vector embeddings for a batch of texts.
    ///
    /// This method exists on `LlmProvider` to allow routing embedding requests
    /// through the same provider being used for completion, enabling users who
    /// prefer API-based embedding to not configure a separate embedding provider.
    ///
    /// Default implementation returns `Err(LlmError::Other)` — providers that
    /// don't support embeddings do not need to override this.
    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        Err(LlmError::Other {
            provider: self.name().to_string(),
            message: "This provider does not support embeddings. Configure an EmbeddingProvider."
                .to_string(),
        })
    }

    /// Declares the capabilities this provider supports.
    ///
    /// Returns true if the provider supports all capabilities in the given list.
    /// Used by the router to filter providers for requests requiring specific features.
    fn supports_capabilities(&self, required: &[&str]) -> bool {
        let caps = self.capabilities();
        required.iter().all(|cap| match *cap {
            "streaming" => caps.supports_streaming,
            "tool_calling" => caps.supports_tool_calling,
            "vision" => caps.supports_vision,
            "thinking" => caps.supports_thinking,
            _ => false,
        })
    }
}
