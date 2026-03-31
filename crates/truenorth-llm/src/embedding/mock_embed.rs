//! Mock embedding provider for tests.
//!
//! Returns deterministic vectors based on a polynomial hash of the input text.
//! Not semantically meaningful — exists purely to allow tests to run without
//! a real embedding backend.
//!
//! ## Properties
//!
//! - **Deterministic**: Same input always produces the same output.
//! - **Unique**: Different inputs almost always produce different vectors (hash collisions are rare).
//! - **Unit-length**: All returned vectors are L2-normalized.
//! - **64-dimensional**: Higher dimensionality than 4D allows more realistic testing of
//!   similarity comparisons.
//! - **Zero latency**: Synchronous computation with no network calls.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use truenorth_core::traits::embedding_provider::{
    EmbeddingError, EmbeddingModelInfo, EmbeddingProvider,
};

const MOCK_DIMENSIONS: usize = 64;

/// Mock embedding provider that returns deterministic vectors for testing.
#[derive(Debug)]
pub struct MockEmbedder {
    model_info: EmbeddingModelInfo,
    embed_count: Arc<AtomicU64>,
}

impl MockEmbedder {
    /// Creates a new `MockEmbedder`.
    pub fn new() -> Self {
        Self {
            model_info: EmbeddingModelInfo {
                name: "mock-embedder-64d".to_string(),
                dimensions: MOCK_DIMENSIONS,
                max_input_tokens: 8192,
                is_local: true,
                is_normalized: true,
            },
            embed_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns the total number of texts embedded so far.
    pub fn embed_count(&self) -> u64 {
        self.embed_count.load(Ordering::SeqCst)
    }

    /// Resets the embed count.
    pub fn reset(&self) {
        self.embed_count.store(0, Ordering::SeqCst);
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmbeddingProvider for MockEmbedder {
    fn model_info(&self) -> &EmbeddingModelInfo {
        &self.model_info
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed_count.fetch_add(1, Ordering::SeqCst);
        debug!(text_len = text.len(), "MockEmbedder: embed called");
        Ok(hash_to_unit_vector(text, MOCK_DIMENSIONS))
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.embed_count.fetch_add(texts.len() as u64, Ordering::SeqCst);
        debug!(batch_size = texts.len(), "MockEmbedder: embed_batch called");
        let embeddings = texts
            .iter()
            .map(|text| hash_to_unit_vector(text, MOCK_DIMENSIONS))
            .collect();
        Ok(embeddings)
    }
}

/// Generates a deterministic unit-length vector for a text string.
///
/// The vector is derived from the text using a FNV-1a-inspired polynomial hash,
/// spread across `dimensions` floats and then L2-normalized.
fn hash_to_unit_vector(text: &str, dimensions: usize) -> Vec<f32> {
    // FNV-1a hash constants
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    // Generate `dimensions` values using different hash seeds
    let mut result = Vec::with_capacity(dimensions);

    for i in 0..dimensions {
        // Seed each dimension with a different value to spread the hash
        let seed = (i as u64).wrapping_mul(2654435761); // Knuth multiplicative hash

        let mut hash = FNV_OFFSET.wrapping_add(seed);
        for byte in text.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }

        // Map to [-1.0, 1.0]
        let value = (hash as f64 / u64::MAX as f64) * 2.0 - 1.0;
        result.push(value as f32);
    }

    // L2-normalize to unit length
    let norm: f32 = result.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for x in &mut result {
            *x /= norm;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_embed_deterministic() {
        let embedder = MockEmbedder::new();

        let v1 = embedder.embed("hello world").await.unwrap();
        let v2 = embedder.embed("hello world").await.unwrap();
        assert_eq!(v1, v2, "Same input must produce same output");
    }

    #[tokio::test]
    async fn test_mock_embed_different_texts() {
        let embedder = MockEmbedder::new();

        let v1 = embedder.embed("hello world").await.unwrap();
        let v2 = embedder.embed("goodbye world").await.unwrap();
        assert_ne!(v1, v2, "Different inputs should produce different embeddings");
    }

    #[tokio::test]
    async fn test_mock_embed_dimensions() {
        let embedder = MockEmbedder::new();
        let v = embedder.embed("test text").await.unwrap();
        assert_eq!(v.len(), MOCK_DIMENSIONS);
    }

    #[tokio::test]
    async fn test_mock_embed_unit_length() {
        let embedder = MockEmbedder::new();
        let v = embedder.embed("unit length test").await.unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "Vector should be approximately unit length, got norm={}",
            norm
        );
    }

    #[tokio::test]
    async fn test_mock_batch_embed() {
        let embedder = MockEmbedder::new();
        let texts = vec!["first", "second", "third"];
        let results = embedder.embed_batch(&texts).await.unwrap();
        assert_eq!(results.len(), 3);

        // Check order preserved
        let single_first = embedder.embed("first").await.unwrap();
        assert_eq!(results[0], single_first, "Batch order should match input order");
    }

    #[tokio::test]
    async fn test_mock_cosine_similarity() {
        let embedder = MockEmbedder::new();

        let v1 = embedder.embed("the quick brown fox").await.unwrap();
        let v2 = embedder.embed("the quick brown fox").await.unwrap();
        let v3 = embedder.embed("completely unrelated text about cars").await.unwrap();

        let sim_same = embedder.cosine_similarity(&v1, &v2);
        assert!(
            (sim_same - 1.0).abs() < 0.001,
            "Identical vectors should have cosine similarity 1.0, got {}",
            sim_same
        );

        // Similarity with self should be 1.0
        assert!(
            (embedder.cosine_similarity(&v3, &v3) - 1.0).abs() < 0.001
        );
    }

    #[tokio::test]
    async fn test_embed_count_tracking() {
        let embedder = MockEmbedder::new();
        assert_eq!(embedder.embed_count(), 0);

        let _ = embedder.embed("text 1").await.unwrap();
        assert_eq!(embedder.embed_count(), 1);

        let _ = embedder.embed_batch(&["a", "b", "c"]).await.unwrap();
        assert_eq!(embedder.embed_count(), 4);

        embedder.reset();
        assert_eq!(embedder.embed_count(), 0);
    }

    #[test]
    fn test_hash_to_unit_vector_zero_text() {
        // Empty string should still produce a valid (non-NaN) vector
        let v = hash_to_unit_vector("", MOCK_DIMENSIONS);
        assert_eq!(v.len(), MOCK_DIMENSIONS);
        assert!(v.iter().all(|x| x.is_finite()), "All values should be finite");
    }
}
