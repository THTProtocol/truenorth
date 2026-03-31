/// EmbeddingProvider trait — the embedding backend abstraction.
///
/// Decoupled from `LlmProvider` to allow independent configuration of the
/// inference backend vs. the embedding backend. The default is the local
/// fastembed AllMiniLML6V2 model; remote providers are config-selectable.

use async_trait::async_trait;
use thiserror::Error;

/// Errors from the embedding backend.
#[derive(Debug, Error)]
pub enum EmbeddingError {
    /// Local model failed to load or initialize.
    #[error("Embedding model failed to initialize: {message}")]
    ModelInitError { message: String },

    /// Embedding of a specific text failed.
    #[error("Failed to embed text: {message}")]
    EmbedError { message: String },

    /// The remote embedding API returned an error.
    #[error("Remote embedding API error (HTTP {status_code}): {message}")]
    ApiError { status_code: u16, message: String },

    /// A batch embed operation partially failed.
    ///
    /// Contains results for successful inputs and error messages for failed inputs.
    #[error("Partial batch failure: {success_count} succeeded, {failure_count} failed")]
    PartialBatchFailure {
        success_count: usize,
        failure_count: usize,
        /// One entry per input: Some(vec) for success, None for failure.
        results: Vec<Option<Vec<f32>>>,
    },

    /// Dimension mismatch — the returned vector has an unexpected length.
    #[error("Dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
}

/// Metadata about an embedding model's output space.
///
/// Required for verifying vector compatibility before cosine similarity operations.
/// All vectors produced by a given model must have the same dimensionality.
#[derive(Debug, Clone)]
pub struct EmbeddingModelInfo {
    /// Human-readable model identifier (e.g., "all-mini-lm-l6-v2").
    pub name: String,
    /// Dimensionality of the output vectors. All vectors from this model have this length.
    pub dimensions: usize,
    /// Maximum number of tokens this model can embed in a single input.
    pub max_input_tokens: usize,
    /// Whether this is a local (ONNX) or remote (API) model.
    pub is_local: bool,
    /// Normalized embedding: whether vectors are L2-normalized to unit length.
    pub is_normalized: bool,
}

/// The embedding provider trait. Decoupled from `LlmProvider` to allow
/// independent configuration of the inference backend vs. the embedding backend.
///
/// Design rationale for decoupling: a user may use Claude for completion
/// (via Anthropic API) but AllMiniLML6V2 for embedding (free local ONNX).
/// Bundling embedding into `LlmProvider` would force the same provider for both
/// and break the "no external dependency for core function" principle when the
/// local default is used (since the `LlmProvider` requires an API key, but
/// the default embedding backend does not).
#[async_trait]
pub trait EmbeddingProvider: Send + Sync + std::fmt::Debug {
    /// Returns metadata about the embedding model in use.
    ///
    /// Called on init to verify dimensional consistency with the vector index.
    fn model_info(&self) -> &EmbeddingModelInfo;

    /// Embeds a single text string into a vector.
    ///
    /// For the local fastembed backend, this is a synchronous ONNX inference
    /// call wrapped in an async facade (runs on a blocking thread pool via
    /// `tokio::task::spawn_blocking`).
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// Embeds a batch of text strings in a single operation.
    ///
    /// This is the preferred method for bulk operations (e.g., Obsidian vault
    /// re-indexing). The local ONNX backend processes batches more efficiently
    /// than sequential single embeds due to matrix operation batching.
    /// The remote API backend reduces round-trips and respects API rate limits.
    ///
    /// Returns one vector per input text in the same order as the input slice.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Embeds a query string optimized for retrieval (may differ from document embedding).
    ///
    /// Some models (asymmetric bi-encoders) use different instruction prefixes
    /// for query vs. document embedding. This method applies query-side instructions.
    /// For symmetric models, this is identical to `embed()`.
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed(query).await
    }

    /// Returns the dimensionality of vectors produced by this provider.
    ///
    /// Convenience method equivalent to `model_info().dimensions`.
    fn dimension(&self) -> usize {
        self.model_info().dimensions
    }

    /// Computes the cosine similarity between two embedding vectors.
    ///
    /// Provided as a default implementation here (rather than a standalone function)
    /// so that future hardware-accelerated embedding backends can override with
    /// GPU-optimized similarity computation.
    fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(
            a.len(),
            b.len(),
            "Vectors must have equal dimensions for cosine similarity"
        );
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }
}
