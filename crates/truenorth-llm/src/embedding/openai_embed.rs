//! OpenAI remote embedding provider using text-embedding-3-small.
//!
//! Uses the OpenAI Embeddings API to generate vectors for semantic memory search.
//! This is the recommended provider for users who want maximum retrieval quality
//! at the cost of API dependency and token spend.
//!
//! ## Specs
//!
//! - **Model**: `text-embedding-3-small` (default) or `text-embedding-3-large`
//! - **Dimensions**: 1536 (text-embedding-3-small), 3072 (text-embedding-3-large)
//! - **Max input tokens**: 8191
//! - **Cost**: ~$0.00002/1K tokens (text-embedding-3-small)
//!
//! ## API reference
//!
//! Endpoint: `POST https://api.openai.com/v1/embeddings`
//! Body: `{"model": "text-embedding-3-small", "input": ["text1", "text2"]}`

use std::time::Instant;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, error, info};

use truenorth_core::traits::embedding_provider::{
    EmbeddingError, EmbeddingModelInfo, EmbeddingProvider,
};

const OPENAI_EMBEDDINGS_URL: &str = "https://api.openai.com/v1/embeddings";
const MODEL_SMALL: &str = "text-embedding-3-small";
const MODEL_LARGE: &str = "text-embedding-3-large";
const SMALL_DIMENSIONS: usize = 1536;
const LARGE_DIMENSIONS: usize = 3072;
const MAX_INPUT_TOKENS: usize = 8191;
/// Maximum texts per batch (OpenAI allows up to 2048 inputs per request)
const MAX_BATCH_SIZE: usize = 100;

/// OpenAI remote embedding provider.
///
/// Supports both `text-embedding-3-small` and `text-embedding-3-large`.
/// Large batches are split into chunks of `MAX_BATCH_SIZE` to avoid API limits.
#[derive(Debug)]
pub struct OpenAiEmbedProvider {
    api_key: String,
    model: String,
    client: Client,
    model_info: EmbeddingModelInfo,
}

impl OpenAiEmbedProvider {
    /// Creates a new `OpenAiEmbedProvider` using `text-embedding-3-small`.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_model(api_key, MODEL_SMALL)
    }

    /// Creates a new `OpenAiEmbedProvider` using the specified model.
    ///
    /// Use `text-embedding-3-large` for maximum quality, `text-embedding-3-small`
    /// for balanced quality/cost.
    pub fn with_model(api_key: impl Into<String>, model: &str) -> Self {
        let dimensions = if model == MODEL_LARGE { LARGE_DIMENSIONS } else { SMALL_DIMENSIONS };

        Self {
            api_key: api_key.into(),
            model: model.to_string(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("Failed to build HTTP client"),
            model_info: EmbeddingModelInfo {
                name: model.to_string(),
                dimensions,
                max_input_tokens: MAX_INPUT_TOKENS,
                is_local: false,
                is_normalized: false, // OpenAI does not normalize by default
            },
        }
    }

    /// Sends a batch of texts to the OpenAI embeddings API and returns the vectors.
    async fn call_api(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let started = Instant::now();

        debug!(
            model = %self.model,
            batch_size = texts.len(),
            "OpenAI embedder: calling API"
        );

        let body = json!({
            "model": self.model,
            "input": texts,
            "encoding_format": "float",
        });

        let response = self
            .client
            .post(OPENAI_EMBEDDINGS_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbeddingError::ApiError {
                status_code: 0,
                message: format!("Network error: {}", e),
            })?;

        let status = response.status().as_u16();
        let latency_ms = started.elapsed().as_millis();

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            error!(
                model = %self.model,
                status,
                "OpenAI embedder API error"
            );
            return Err(EmbeddingError::ApiError {
                status_code: status,
                message: extract_openai_error(&error_body),
            });
        }

        let json: Value = response.json().await.map_err(|e| EmbeddingError::ApiError {
            status_code: 500,
            message: format!("JSON parse error: {}", e),
        })?;

        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| EmbeddingError::ApiError {
                status_code: 500,
                message: "No 'data' array in OpenAI embedding response".to_string(),
            })?;

        // Sort by index to ensure order matches input order
        let mut indexed_embeddings: Vec<(usize, Vec<f32>)> = data
            .iter()
            .map(|item| {
                let index = item.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let embedding = item
                    .get("embedding")
                    .and_then(|e| e.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect::<Vec<f32>>()
                    })
                    .unwrap_or_default();
                (index, embedding)
            })
            .collect();

        indexed_embeddings.sort_by_key(|(i, _)| *i);

        let embeddings: Vec<Vec<f32>> = indexed_embeddings.into_iter().map(|(_, e)| e).collect();

        // Validate dimensions
        let expected_dims = self.model_info.dimensions;
        for (_i, emb) in embeddings.iter().enumerate() {
            if emb.len() != expected_dims {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: expected_dims,
                    actual: emb.len(),
                });
            }
        }

        info!(
            model = %self.model,
            batch_size = texts.len(),
            latency_ms = latency_ms,
            "OpenAI embedder: batch complete"
        );

        Ok(embeddings)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedProvider {
    fn model_info(&self) -> &EmbeddingModelInfo {
        &self.model_info
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut results = self.embed_batch(&[text]).await?;
        results.pop().ok_or_else(|| EmbeddingError::EmbedError {
            message: "API returned no embeddings for single text".to_string(),
        })
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Split into chunks to respect API limits
        if texts.len() <= MAX_BATCH_SIZE {
            return self.call_api(texts).await;
        }

        // Process in chunks
        let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(MAX_BATCH_SIZE) {
            let chunk_embeddings = self.call_api(chunk).await?;
            all_embeddings.extend(chunk_embeddings);
        }

        Ok(all_embeddings)
    }
}

fn extract_openai_error(body: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_small() {
        let provider = OpenAiEmbedProvider::new("test-key");
        assert_eq!(provider.model_info().dimensions, SMALL_DIMENSIONS);
        assert_eq!(provider.model_info().name, MODEL_SMALL);
        assert!(!provider.model_info().is_local);
    }

    #[test]
    fn test_model_info_large() {
        let provider = OpenAiEmbedProvider::with_model("test-key", MODEL_LARGE);
        assert_eq!(provider.model_info().dimensions, LARGE_DIMENSIONS);
    }

    #[test]
    fn test_cosine_similarity_same_vector() {
        let provider = OpenAiEmbedProvider::new("test-key");
        let v = vec![0.5_f32, 0.5, 0.5, 0.5];
        let sim = provider.cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001, "Same vector should have similarity 1.0");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let provider = OpenAiEmbedProvider::new("test-key");
        let a = vec![1.0_f32, 0.0, 0.0, 0.0];
        let b = vec![0.0_f32, 1.0, 0.0, 0.0];
        let sim = provider.cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001, "Orthogonal vectors should have similarity 0.0");
    }
}
