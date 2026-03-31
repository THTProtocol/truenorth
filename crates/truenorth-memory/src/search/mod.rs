//! Search subsystem — full-text, semantic, and hybrid search over memory entries.
//!
//! The search engine is the primary interface for querying the memory layer.
//! It abstracts over three search methods:
//!
//! - **Full-text** (`TantivySearch`): BM25-ranked keyword search using a Tantivy index.
//! - **Semantic** (`SemanticSearch`): cosine similarity over pre-computed embedding vectors.
//! - **Hybrid** (`HybridSearch`): Reciprocal Rank Fusion (RRF) of the two result lists.
//!
//! ## Modules
//!
//! - [`fulltext`] — `TantivySearch`: Tantivy index wrapper.
//! - [`semantic`] — `SemanticSearch`: in-memory vector store with cosine similarity.
//! - [`hybrid`] — `HybridSearch`: RRF merge of fulltext + semantic results.
//!
//! ## `SearchEngine` facade
//!
//! The [`SearchEngine`] struct wraps all three and exposes a unified async interface
//! used by [`MemoryLayer`](crate::MemoryLayer).

pub mod fulltext;
pub mod hybrid;
pub mod semantic;

pub use fulltext::TantivySearch;
pub use hybrid::HybridSearch;
pub use semantic::SemanticSearch;

use std::path::PathBuf;
use std::sync::Arc;

use truenorth_core::traits::embedding_provider::EmbeddingProvider;
use truenorth_core::traits::memory::MemoryError;
use truenorth_core::types::memory::{MemoryEntry, MemoryScope, MemorySearchResult};
use uuid::Uuid;

/// Unified search facade for the three-tier memory layer.
///
/// Holds a `TantivySearch` (full-text) and a `SemanticSearch` (vector) engine,
/// and provides hybrid search via `HybridSearch`.
#[derive(Debug)]
pub struct SearchEngine {
    /// Tantivy full-text search.
    fulltext: Arc<TantivySearch>,
    /// In-memory semantic search.
    semantic: Arc<SemanticSearch>,
    /// Hybrid RRF combiner.
    hybrid: HybridSearch,
}

impl SearchEngine {
    /// Create a new `SearchEngine`.
    ///
    /// Opens or creates the Tantivy index at `index_dir`. Initializes the
    /// in-memory semantic vector store.
    ///
    /// # Errors
    ///
    /// Returns a `String` error if the Tantivy index cannot be opened.
    pub fn new(
        index_dir: PathBuf,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self, String> {
        let fulltext = TantivySearch::new(index_dir)?;
        let semantic = SemanticSearch::new(embedding_provider);
        let hybrid = HybridSearch::new(0.5, 0.5);
        Ok(Self {
            fulltext: Arc::new(fulltext),
            semantic: Arc::new(semantic),
            hybrid,
        })
    }

    /// Index a memory entry so it appears in future searches.
    ///
    /// Adds the entry to both the Tantivy full-text index and the semantic
    /// vector store (if it has an embedding).
    pub async fn index_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        // Full-text indexing (sync, runs on blocking thread pool).
        let ft = self.fulltext.clone();
        let entry_clone = entry.clone();
        tokio::task::spawn_blocking(move || ft.index_entry(&entry_clone))
            .await
            .map_err(|e| MemoryError::SearchIndexError {
                message: format!("Tantivy index task panic: {e}"),
            })?
            .map_err(|e| MemoryError::SearchIndexError { message: e })?;

        // Semantic indexing (only if embedding is present).
        if entry.embedding.is_some() {
            self.semantic.index_entry(entry).await;
        }

        Ok(())
    }

    /// Remove a memory entry from all search indices.
    pub async fn remove_entry(&self, id: Uuid) -> Result<(), MemoryError> {
        let ft = self.fulltext.clone();
        tokio::task::spawn_blocking(move || ft.remove_entry(id))
            .await
            .map_err(|e| MemoryError::SearchIndexError {
                message: format!("Tantivy remove task panic: {e}"),
            })?
            .map_err(|e| MemoryError::SearchIndexError { message: e })?;

        self.semantic.remove_entry(id).await;
        Ok(())
    }

    /// Perform BM25 full-text search within a scope.
    pub async fn fulltext_search(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        let ft = self.fulltext.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || ft.search(&query, scope, limit))
            .await
            .map_err(|e| MemoryError::SearchIndexError {
                message: format!("Tantivy search task panic: {e}"),
            })?
            .map_err(|e| MemoryError::SearchIndexError { message: e })
    }

    /// Perform semantic (cosine similarity) search within a scope.
    pub async fn semantic_search(
        &self,
        query: &str,
        scope: MemoryScope,
        top_k: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        self.semantic.search(query, scope, top_k).await
    }

    /// Perform hybrid search (RRF of fulltext + semantic) within a scope.
    pub async fn hybrid_search(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        let ft_results = self.fulltext_search(query, scope, limit * 2).await?;
        let sem_results = self.semantic_search(query, scope, limit * 2).await?;
        let merged = self.hybrid.merge(ft_results, sem_results, limit);
        Ok(merged)
    }

    /// Re-index all entries from a batch (used after vault sync or consolidation).
    pub async fn reindex_batch(&self, entries: &[MemoryEntry]) -> Result<(), MemoryError> {
        // Rebuild Tantivy index.
        let ft = self.fulltext.clone();
        let entries_clone = entries.to_vec();
        tokio::task::spawn_blocking(move || ft.reindex_all(&entries_clone))
            .await
            .map_err(|e| MemoryError::SearchIndexError {
                message: format!("Tantivy reindex task panic: {e}"),
            })?
            .map_err(|e| MemoryError::SearchIndexError { message: e })?;

        // Rebuild semantic index.
        self.semantic.reindex_all(entries).await;

        Ok(())
    }
}
