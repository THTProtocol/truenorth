//! `SemanticSearch` — cosine-similarity vector search over memory embeddings.
//!
//! Maintains an in-memory vector store: `Vec<(Uuid, MemoryScope, Vec<f32>)>`.
//! On each semantic query, the query text is embedded via the `EmbeddingProvider`,
//! then cosine similarity is computed against all stored vectors. The top-k
//! results are returned.
//!
//! # Performance characteristics
//!
//! This is a brute-force O(n × d) scan, where n is the number of stored vectors
//! and d is the embedding dimension. For typical project memory sizes (< 10 000
//! entries at 384 dimensions) this is fast enough without an HNSW index. An HNSW
//! integration can be added when scale demands it.

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, warn};
use uuid::Uuid;

use truenorth_core::traits::embedding_provider::EmbeddingProvider;
use truenorth_core::traits::memory::MemoryError;
use truenorth_core::types::memory::{MemoryEntry, MemoryScope, MemorySearchResult, MemorySearchType};

use crate::project::deduplicator::cosine_similarity;

/// In-memory record for the semantic vector store.
#[derive(Debug, Clone)]
struct VectorRecord {
    id: Uuid,
    scope: MemoryScope,
    embedding: Vec<f32>,
    /// Keep a copy of the content so we can return partial entries without SQLite.
    content: String,
}

/// Semantic similarity search using in-memory vector store.
///
/// The store is populated by calls to [`index_entry`] as entries are written to
/// the memory layer. It is rebuilt via [`reindex_all`] after consolidation.
#[derive(Debug)]
pub struct SemanticSearch {
    /// In-memory vector records, protected for concurrent access.
    records: Arc<RwLock<Vec<VectorRecord>>>,
    /// Embedding provider for query embedding.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl SemanticSearch {
    /// Create a new empty `SemanticSearch`.
    pub fn new(embedding_provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        Self {
            records: Arc::new(RwLock::new(Vec::new())),
            embedding_provider,
        }
    }

    /// Add or update a memory entry in the vector store.
    ///
    /// If an entry with the same UUID already exists, it is replaced.
    pub async fn index_entry(&self, entry: &MemoryEntry) {
        let embedding = match &entry.embedding {
            Some(v) => v.clone(),
            None => return, // Nothing to index without an embedding.
        };

        let record = VectorRecord {
            id: entry.id,
            scope: entry.scope,
            embedding,
            content: entry.content.clone(),
        };

        let mut records = self.records.write().await;
        // Replace existing record if present.
        if let Some(pos) = records.iter().position(|r| r.id == entry.id) {
            records[pos] = record;
        } else {
            records.push(record);
        }
        debug!("SemanticSearch indexed entry {}", entry.id);
    }

    /// Remove an entry from the vector store.
    pub async fn remove_entry(&self, id: Uuid) {
        let mut records = self.records.write().await;
        records.retain(|r| r.id != id);
    }

    /// Perform a top-k semantic similarity search within a scope.
    ///
    /// Embeds the `query` string via the configured embedding provider, then
    /// computes cosine similarity against all stored vectors for the given scope.
    /// Returns the top `top_k` results sorted by descending similarity.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::EmbeddingError` if the query cannot be embedded.
    /// Returns an empty result (not an error) if no embedding provider is configured.
    pub async fn search(
        &self,
        query: &str,
        scope: MemoryScope,
        top_k: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        let provider = match &self.embedding_provider {
            Some(p) => p.clone(),
            None => {
                debug!("SemanticSearch: no embedding provider, returning empty results");
                return Ok(Vec::new());
            }
        };

        // Embed the query.
        let query_vec = provider.embed_query(query).await.map_err(|e| {
            MemoryError::EmbeddingError {
                message: format!("Failed to embed query: {e}"),
            }
        })?;

        // Compute similarities.
        let records = self.records.read().await;
        let mut scored: Vec<(f32, &VectorRecord)> = records
            .iter()
            .filter(|r| r.scope == scope)
            .map(|r| {
                let score = cosine_similarity(&query_vec, &r.embedding);
                (score, r)
            })
            .collect();

        // Sort descending by similarity.
        scored.sort_by(|(a, _), (b, _)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        let results = scored
            .into_iter()
            .map(|(score, r)| {
                let now = chrono::Utc::now();
                MemorySearchResult {
                    entry: MemoryEntry {
                        id: r.id,
                        scope: r.scope,
                        content: r.content.clone(),
                        metadata: Default::default(),
                        embedding: Some(r.embedding.clone()),
                        created_at: now,
                        updated_at: now,
                        importance: 0.5,
                        retrieval_count: 0,
                    },
                    score,
                    search_type: MemorySearchType::Semantic,
                }
            })
            .collect();

        debug!("SemanticSearch query='{}' scope={:?} top_k={}", query, scope, top_k);
        Ok(results)
    }

    /// Rebuild the vector store from a batch of entries.
    ///
    /// Clears all existing records and re-adds entries that have embeddings.
    pub async fn reindex_all(&self, entries: &[MemoryEntry]) {
        let mut records = self.records.write().await;
        records.clear();
        for entry in entries {
            if let Some(embedding) = &entry.embedding {
                records.push(VectorRecord {
                    id: entry.id,
                    scope: entry.scope,
                    embedding: embedding.clone(),
                    content: entry.content.clone(),
                });
            }
        }
        tracing::info!("SemanticSearch: reindexed {} entries with embeddings", records.len());
    }

    /// Return the current number of indexed vectors.
    pub async fn vector_count(&self) -> usize {
        self.records.read().await.len()
    }

    /// Return the number of vectors for a specific scope.
    pub async fn vector_count_for_scope(&self, scope: MemoryScope) -> usize {
        self.records.read().await.iter().filter(|r| r.scope == scope).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_entry_with_emb(id: Uuid, content: &str, scope: MemoryScope, emb: Vec<f32>) -> MemoryEntry {
        let now = chrono::Utc::now();
        MemoryEntry {
            id,
            scope,
            content: content.to_string(),
            metadata: HashMap::new(),
            embedding: Some(emb),
            created_at: now,
            updated_at: now,
            importance: 0.5,
            retrieval_count: 0,
        }
    }

    #[tokio::test]
    async fn test_index_and_vector_count() {
        let ss = SemanticSearch::new(None);
        let e1 = make_entry_with_emb(Uuid::new_v4(), "hello", MemoryScope::Project, vec![1.0, 0.0]);
        let e2 = make_entry_with_emb(Uuid::new_v4(), "world", MemoryScope::Project, vec![0.0, 1.0]);
        ss.index_entry(&e1).await;
        ss.index_entry(&e2).await;
        assert_eq!(ss.vector_count().await, 2);
    }

    #[tokio::test]
    async fn test_remove_entry() {
        let ss = SemanticSearch::new(None);
        let id = Uuid::new_v4();
        let e = make_entry_with_emb(id, "test", MemoryScope::Project, vec![1.0, 0.0, 0.0]);
        ss.index_entry(&e).await;
        assert_eq!(ss.vector_count().await, 1);
        ss.remove_entry(id).await;
        assert_eq!(ss.vector_count().await, 0);
    }

    #[tokio::test]
    async fn test_no_provider_returns_empty() {
        let ss = SemanticSearch::new(None);
        let results = ss.search("anything", MemoryScope::Project, 5).await.unwrap();
        assert!(results.is_empty());
    }
}
