/// MemoryStore trait — the unified memory layer contract.
///
/// All three memory tiers (Session, Project, Identity) implement this interface.
/// The orchestrator uses a single interface regardless of which tier it queries.
/// This enables cross-tier hybrid search and simplifies context budget management.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::types::memory::{MemoryEntry, MemoryScope, MemorySearchResult};

/// Result of a context compaction operation.
///
/// Returned by `MemoryStore::compact()` to report what was summarized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// The summarized replacement for the compacted history.
    pub summary: String,
    /// Token count before compaction.
    pub tokens_before: usize,
    /// Token count after compaction.
    pub tokens_after: usize,
    /// Number of messages removed from history.
    pub messages_removed: usize,
    /// The session ID that was compacted.
    pub session_id: Uuid,
}

/// Report from a memory consolidation cycle.
///
/// Returned by `MemoryStore::consolidate()` after an autoDream-style cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationReport {
    /// Which tier was consolidated.
    pub scope: MemoryScope,
    /// Total entries reviewed.
    pub entries_reviewed: usize,
    /// Entries that were merged with similar existing entries.
    pub entries_merged: usize,
    /// Entries that were pruned (low importance, stale).
    pub entries_pruned: usize,
    /// New summary entries created by the consolidation.
    pub entries_created: usize,
    /// Wall-clock duration of the consolidation run.
    pub duration_ms: u64,
}

/// Errors from the memory layer.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// Failed to read an entry.
    #[error("Failed to read memory ({scope:?}): {message}")]
    ReadError { scope: MemoryScope, message: String },

    /// Failed to write an entry.
    #[error("Failed to write memory ({scope:?}): {message}")]
    WriteError { scope: MemoryScope, message: String },

    /// The full-text search index encountered an error.
    #[error("Full-text search index error: {message}")]
    SearchIndexError { message: String },

    /// Embedding generation failed during a memory operation.
    #[error("Embedding generation failed during memory operation: {message}")]
    EmbeddingError { message: String },

    /// Context compaction failed.
    #[error("Memory compaction failed: {message}")]
    CompactionError { message: String },

    /// No entry with the given ID exists.
    #[error("Memory entry not found: {id}")]
    EntryNotFound { id: Uuid },

    /// The SQLite storage backend returned an error.
    #[error("SQLite storage error: {message}")]
    StorageError { message: String },

    /// Memory consolidation failed.
    #[error("Memory consolidation failed: {message}")]
    ConsolidationError { message: String },
}

/// The unified memory store trait. All three tiers implement this interface,
/// even though their underlying storage mechanisms differ.
///
/// Design rationale: the orchestrator and agent loop use a single interface
/// regardless of which tier they are querying. This allows:
/// 1. The orchestrator to issue cross-tier queries with a unified API
/// 2. Swapping storage backends (SQLite → Postgres) without changing calling code
/// 3. Testing memory operations with a mock implementation
///
/// The three-tier model (Session/Project/Identity) is preserved through the
/// `MemoryScope` parameter on every operation.
#[async_trait]
pub trait MemoryStore: Send + Sync + std::fmt::Debug {
    /// Writes a new memory entry to the specified scope.
    ///
    /// Before writing, the store performs deduplication:
    /// if a semantically similar entry exists (above the configured threshold),
    /// the existing entry is updated rather than creating a duplicate.
    ///
    /// Also generates an embedding vector for the entry if the embedding
    /// provider is configured and semantic search is enabled.
    async fn write(
        &self,
        content: String,
        scope: MemoryScope,
        metadata: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<MemoryEntry, MemoryError>;

    /// Retrieves a specific memory entry by its unique ID.
    async fn read(&self, id: Uuid) -> Result<MemoryEntry, MemoryError>;

    /// Performs full-text keyword search using Tantivy (BM25 ranking).
    ///
    /// Returns results ranked by TF-IDF relevance, filtered to the specified scope.
    async fn search_text(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError>;

    /// Performs semantic similarity search using embedding vectors.
    ///
    /// Returns results ranked by cosine similarity. Requires that entries have
    /// been embedded (embedding column is non-null). Entries without embeddings
    /// are excluded from semantic results.
    async fn search_semantic(
        &self,
        query: &str,
        scope: MemoryScope,
        top_k: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError>;

    /// Performs hybrid search combining FTS and semantic results via RRF.
    ///
    /// Reciprocal Rank Fusion (RRF) merges the two result lists into a unified
    /// ranking that handles both keyword-matched memories (FTS strength) and
    /// conceptually related memories (semantic strength). This is the preferred
    /// method for general memory queries.
    async fn search_hybrid(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError>;

    /// Returns all entries in a scope created within a time window.
    ///
    /// Used by the consolidation agent to gather recent session memories for review.
    async fn list_recent(
        &self,
        scope: MemoryScope,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Compacts the session memory by summarizing it via LLM.
    ///
    /// Called automatically when context budget approaches the 70% threshold.
    /// Returns the compacted summary and the number of tokens saved.
    async fn compact(
        &self,
        session_id: Uuid,
        budget_hint: usize,
    ) -> Result<CompactionResult, MemoryError>;

    /// Runs the autoDream-style consolidation cycle: Orient → Gather → Consolidate → Prune.
    ///
    /// Typically run between sessions as a background task. Merges similar entries,
    /// promotes important information, and prunes low-value stale content.
    async fn consolidate(
        &self,
        scope: MemoryScope,
    ) -> Result<ConsolidationReport, MemoryError>;

    /// Records a retrieval event to update the importance score of an entry.
    ///
    /// Called after each retrieval. Tracks usage patterns for consolidation pruning.
    async fn record_retrieval(&self, id: Uuid) -> Result<(), MemoryError>;

    /// Deletes a specific memory entry.
    ///
    /// Available via CLI (`truenorth memory wipe --id`) for explicit user management.
    async fn delete(&self, id: Uuid) -> Result<(), MemoryError>;

    /// Returns the total number of entries in a scope.
    async fn entry_count(&self, scope: MemoryScope) -> Result<usize, MemoryError>;
}
