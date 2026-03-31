//! Embedding provider implementations.
//!
//! Each embedding provider implements the
//! [`EmbeddingProvider`](truenorth_core::traits::embedding_provider::EmbeddingProvider)
//! trait. Embedding providers are independent of LLM providers — a user may
//! use Claude for completion but local AllMiniLML6V2 for embeddings.
//!
//! ## Available providers
//!
//! | Provider | Module | Feature flag | Notes |
//! |----------|--------|--------------|-------|
//! | fastembed (AllMiniLML6V2) | `fastembed_provider` | `local-embeddings` | Default, free, local ONNX |
//! | OpenAI text-embedding-3-small | `openai_embed` | none | Remote API, ~$0.00002/1K tokens |
//! | Mock | `mock_embed` | none | Deterministic, for tests |
//!
//! ## Selection
//!
//! The embedding provider is selected at startup from `config.toml`:
//! ```toml
//! [memory.embedding]
//! provider = "local"  # or "openai"
//! ```

#[cfg(feature = "local-embeddings")]
pub mod fastembed_provider;
pub mod mock_embed;
pub mod openai_embed;

#[cfg(feature = "local-embeddings")]
pub use fastembed_provider::FastEmbedProvider;
pub use mock_embed::MockEmbedder;
pub use openai_embed::OpenAiEmbedProvider;

use truenorth_core::traits::embedding_provider::{EmbeddingError, EmbeddingProvider};
use std::sync::Arc;

/// A type-erased embedding provider.
pub type ArcEmbedder = Arc<dyn EmbeddingProvider>;

/// Creates an OpenAI embedding provider.
pub fn openai_embedder(api_key: impl Into<String>) -> ArcEmbedder {
    Arc::new(OpenAiEmbedProvider::new(api_key))
}

/// Creates a mock embedder for testing.
pub fn mock_embedder() -> ArcEmbedder {
    Arc::new(MockEmbedder::new())
}

/// Creates the local fastembed provider (only available with `local-embeddings` feature).
///
/// Returns an error if the feature is not enabled.
pub fn local_embedder() -> Result<ArcEmbedder, EmbeddingError> {
    #[cfg(feature = "local-embeddings")]
    {
        Ok(Arc::new(FastEmbedProvider::new()))
    }
    #[cfg(not(feature = "local-embeddings"))]
    {
        Err(EmbeddingError::ModelInitError {
            message: "Local embedding requires the 'local-embeddings' feature flag. \
                      Rebuild with: cargo build --features local-embeddings".to_string(),
        })
    }
}
