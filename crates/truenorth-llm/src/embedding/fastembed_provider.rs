//! Local embedding provider using fastembed + AllMiniLML6V2 via ONNX runtime.
//!
//! This module is gated behind `#[cfg(feature = "local-embeddings")]` because
//! fastembed requires an ONNX runtime at compile time.
//!
//! ## Architecture decisions
//!
//! 1. **Lazy initialization**: The ONNX model is loaded on the first `embed()` call,
//!    not at startup. Users who never call semantic search pay zero startup cost.
//!
//! 2. **Model caching**: The model is downloaded once to `~/.truenorth/models/`
//!    and loaded from disk on subsequent boots. Network access only on first boot.
//!
//! 3. **Batch efficiency**: The ONNX runtime processes batches via matrix operations.
//!    `embed_batch()` is significantly faster than N serial `embed()` calls.
//!
//! 4. **Blocking thread pool**: ONNX inference is synchronous CPU-bound work.
//!    All inference calls are wrapped in `tokio::task::spawn_blocking` to avoid
//!    blocking the async executor.
//!
//! ## Model specs
//!
//! - **Model**: AllMiniLML6V2 (22M parameters, trained 2021)
//! - **Dimensions**: 384
//! - **Max input tokens**: 256
//! - **Storage**: ~90MB on first download
//! - **Inference**: ~5ms per embed on modern CPU hardware

#![cfg(feature = "local-embeddings")]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tracing::{debug, info, warn};

use truenorth_core::traits::embedding_provider::{
    EmbeddingError, EmbeddingModelInfo, EmbeddingProvider,
};

const MODEL_DIMENSIONS: usize = 384;
const MAX_INPUT_TOKENS: usize = 256;
const DEFAULT_CACHE_DIR: &str = ".truenorth/models";

/// Local ONNX-based embedding provider using AllMiniLML6V2.
///
/// Model initialization is deferred until the first embed call.
/// Uses `OnceLock` to ensure the model is initialized exactly once
/// across all async tasks — `OnceLock` is safe for this pattern
/// because `TextEmbedding` is `Send + Sync`.
#[derive(Debug)]
pub struct FastEmbedProvider {
    model_info: EmbeddingModelInfo,
    cache_dir: PathBuf,
    model: OnceLock<TextEmbedding>,
}

impl FastEmbedProvider {
    /// Creates a new `FastEmbedProvider` with the default model cache directory.
    ///
    /// The model is NOT initialized here — it loads on the first embed call.
    pub fn new() -> Self {
        Self::with_cache_dir(
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(DEFAULT_CACHE_DIR),
        )
    }

    /// Creates a `FastEmbedProvider` with a custom cache directory.
    ///
    /// Useful for tests that want an isolated cache location.
    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self {
            model_info: EmbeddingModelInfo {
                name: "all-mini-lm-l6-v2".to_string(),
                dimensions: MODEL_DIMENSIONS,
                max_input_tokens: MAX_INPUT_TOKENS,
                is_local: true,
                is_normalized: true,
            },
            cache_dir,
            model: OnceLock::new(),
        }
    }

    /// Ensures the model is loaded, initializing it if needed.
    ///
    /// Returns an error if the model cannot be loaded or downloaded.
    fn ensure_initialized(&self) -> Result<&TextEmbedding, EmbeddingError> {
        self.model.get_or_try_init(|| {
            info!(
                cache_dir = %self.cache_dir.display(),
                "FastEmbed: initializing AllMiniLML6V2 model (first embed call)"
            );

            std::fs::create_dir_all(&self.cache_dir).map_err(|e| EmbeddingError::ModelInitError {
                message: format!("Failed to create model cache directory: {}", e),
            })?;

            let opts = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_cache_dir(self.cache_dir.clone())
                .with_show_download_progress(true);

            TextEmbedding::try_new(opts).map_err(|e| EmbeddingError::ModelInitError {
                message: format!("Failed to initialize AllMiniLML6V2: {}", e),
            })
        })
    }

    /// Runs batch inference on the blocking thread pool.
    ///
    /// ONNX inference is CPU-bound and synchronous. We initialize the model
    /// first (which may load from disk), then run inference in a blocking task.
    async fn run_batch_inference(
        &self,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        // Ensure model is initialized before entering spawn_blocking.
        // This is safe: ensure_initialized() is synchronous and uses OnceLock.
        // The first call may block briefly loading the ONNX model from disk.
        self.ensure_initialized()?;

        let cache_dir = self.cache_dir.clone();

        let embeddings = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>, EmbeddingError> {
            // Re-create the TextEmbedding for use in the blocking thread.
            // TextEmbedding loads from the cache (model is already downloaded).
            let opts = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(false);

            let model = TextEmbedding::try_new(opts).map_err(|e| EmbeddingError::ModelInitError {
                message: format!("Failed to load AllMiniLML6V2 from cache: {}", e),
            })?;

            let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            let results = model.embed(text_refs, None).map_err(|e| EmbeddingError::EmbedError {
                message: format!("ONNX batch inference failed: {}", e),
            })?;

            debug!(
                batch_size = texts.len(),
                "FastEmbed: batch inference complete"
            );

            Ok(results)
        })
        .await
        .map_err(|e| EmbeddingError::EmbedError {
            message: format!("Blocking task panicked: {}", e),
        })??;

        Ok(embeddings)
    }
}

impl Default for FastEmbedProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    fn model_info(&self) -> &EmbeddingModelInfo {
        &self.model_info
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let texts = vec![text.to_string()];
        let mut results = self.embed_batch(&[text]).await?;
        results.pop().ok_or_else(|| EmbeddingError::EmbedError {
            message: "Batch returned empty results for single embed".to_string(),
        })
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        debug!(batch_size = texts.len(), "FastEmbed: embed_batch called");

        let text_strings: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let embeddings = self.run_batch_inference(text_strings).await?;

        // Verify dimensions
        for embedding in embeddings.iter() {
            if embedding.len() != MODEL_DIMENSIONS {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: MODEL_DIMENSIONS,
                    actual: embedding.len(),
                });
            }
        }

        Ok(embeddings)
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        // AllMiniLML6V2 is a symmetric model — query and document embeddings are identical
        self.embed(query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Integration test — only runs when ONNX model is available.
    /// Set `TRUENORTH_TEST_EMBEDDINGS=1` to enable.
    #[tokio::test]
    #[ignore = "Requires ONNX model download; run with TRUENORTH_TEST_EMBEDDINGS=1"]
    async fn test_fastembed_basic() {
        let provider = FastEmbedProvider::new();

        let embedding = provider
            .embed("Hello, semantic search!")
            .await
            .expect("Should embed successfully");

        assert_eq!(
            embedding.len(),
            MODEL_DIMENSIONS,
            "Should return 384-dimensional vector"
        );

        // Check approximately unit length
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "AllMiniLML6V2 should return normalized vectors"
        );
    }

    #[tokio::test]
    #[ignore = "Requires ONNX model download"]
    async fn test_fastembed_batch() {
        let provider = FastEmbedProvider::new();
        let texts = vec!["first text", "second text", "third text"];

        let embeddings = provider.embed_batch(&texts).await.expect("Batch embed should succeed");

        assert_eq!(embeddings.len(), 3, "Should return one embedding per input");
        for emb in &embeddings {
            assert_eq!(emb.len(), MODEL_DIMENSIONS);
        }
    }

    #[tokio::test]
    #[ignore = "Requires ONNX model download"]
    async fn test_semantic_similarity() {
        let provider = FastEmbedProvider::new();

        let dog_emb = provider.embed("A dog is a furry animal").await.unwrap();
        let wolf_emb = provider.embed("Wolves are wild canines").await.unwrap();
        let car_emb = provider.embed("A car is a motor vehicle").await.unwrap();

        let dog_wolf_sim = provider.cosine_similarity(&dog_emb, &wolf_emb);
        let dog_car_sim = provider.cosine_similarity(&dog_emb, &car_emb);

        assert!(
            dog_wolf_sim > dog_car_sim,
            "Dog-wolf similarity ({:.3}) should exceed dog-car similarity ({:.3})",
            dog_wolf_sim,
            dog_car_sim
        );
    }
}
