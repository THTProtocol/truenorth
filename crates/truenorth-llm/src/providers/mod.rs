//! LLM provider implementations.
//!
//! Each provider module implements the [`LlmProvider`](truenorth_core::traits::llm_provider::LlmProvider)
//! trait for a specific LLM backend. Provider modules are responsible for:
//!
//! 1. Translating [`CompletionRequest`](truenorth_core::types::llm::CompletionRequest) into
//!    the provider's native API request format.
//! 2. Making HTTP requests with proper authentication headers.
//! 3. Parsing responses back into [`CompletionResponse`](truenorth_core::types::llm::CompletionResponse).
//! 4. Mapping provider-specific HTTP error codes to [`LlmError`](truenorth_core::error::LlmError) variants.
//! 5. Implementing streaming via SSE parsing.
//!
//! ## Adding a new provider
//!
//! 1. Create `src/providers/my_provider.rs`
//! 2. Implement the `LlmProvider` trait
//! 3. Add `pub mod my_provider;` below
//! 4. Optionally add a convenience constructor to `ProviderRegistry`

pub mod anthropic;
pub mod google;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod openai_compat;

// Re-export the concrete types for convenience
pub use anthropic::AnthropicProvider;
pub use google::GoogleProvider;
pub use mock::MockProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;
pub use openai_compat::OpenAiCompatProvider;

use std::sync::Arc;
use truenorth_core::traits::llm_provider::LlmProvider;

/// A boxed, type-erased LLM provider.
///
/// The router stores providers as `Arc<dyn LlmProvider>` so it can hold
/// a heterogeneous list of backends without requiring generics.
pub type ArcProvider = Arc<dyn LlmProvider>;

/// Builds an `ArcProvider` from an [`AnthropicProvider`].
pub fn anthropic(api_key: impl Into<String>, model: impl Into<String>) -> ArcProvider {
    Arc::new(AnthropicProvider::new(api_key, model))
}

/// Builds an `ArcProvider` from an [`OpenAiProvider`].
pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> ArcProvider {
    Arc::new(OpenAiProvider::new(api_key, model))
}

/// Builds an `ArcProvider` from a [`GoogleProvider`].
pub fn google(api_key: impl Into<String>, model: impl Into<String>) -> ArcProvider {
    Arc::new(GoogleProvider::new(api_key, model))
}

/// Builds an `ArcProvider` from an [`OllamaProvider`].
pub fn ollama(base_url: impl Into<String>, model: impl Into<String>) -> ArcProvider {
    Arc::new(OllamaProvider::new(base_url, model))
}

/// Builds an `ArcProvider` from an [`OpenAiCompatProvider`].
pub fn openai_compat(
    base_url: impl Into<String>,
    api_key: impl Into<String>,
    model: impl Into<String>,
) -> ArcProvider {
    Arc::new(OpenAiCompatProvider::new(base_url, api_key, model))
}

/// Builds a [`MockProvider`] for testing.
pub fn mock() -> ArcProvider {
    Arc::new(MockProvider::new())
}
