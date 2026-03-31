/// Memory types — the three-tier memory system for TrueNorth.
///
/// TrueNorth uses three distinct memory tiers: Session (ephemeral),
/// Project (persistent, project-scoped), and Identity (persistent, user-scoped).
/// Every memory operation is scoped to one of these tiers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// The three memory tiers, each with distinct persistence and access semantics.
///
/// This is the canonical scope enum used throughout the system.
/// The scope determines which storage backend handles the operation,
/// how long the data persists, and which users can access it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MemoryScope {
    /// Ephemeral: exists only for the current session.
    ///
    /// Stored in-memory (Arc<RwLock<>>). Optionally persisted to SQLite
    /// on session end for consolidation purposes. Contents include:
    /// conversation history, tool results, scratchpad state.
    Session,

    /// Persistent: survives across sessions, scoped to a project.
    ///
    /// Stored in SQLite + Markdown files in /memory/project/.
    /// Contents include: codebase context, past decisions, error patterns,
    /// domain knowledge specific to this project.
    Project,

    /// Persistent: survives across sessions and projects.
    ///
    /// Stored in SQLite + Markdown in /memory/identity/.
    /// Contents include: user preferences, communication style,
    /// workflow patterns, long-term knowledge.
    Identity,
}

/// A single memory entry stored in the memory layer.
///
/// Memory entries are the fundamental unit of stored knowledge.
/// They carry their content, scope, metadata for filtering,
/// and an optional pre-computed embedding for semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Globally unique identifier.
    pub id: Uuid,
    /// Which tier this entry belongs to.
    pub scope: MemoryScope,
    /// The primary text content of this memory.
    pub content: String,
    /// Structured key-value metadata for filtering.
    /// Common keys: "task_id", "tool_name", "source_file", "session_id".
    pub metadata: HashMap<String, serde_json::Value>,
    /// Pre-computed embedding vector for semantic search.
    /// None until the embedding provider processes this entry.
    pub embedding: Option<Vec<f32>>,
    /// When this entry was first written.
    pub created_at: DateTime<Utc>,
    /// When this entry was last updated.
    pub updated_at: DateTime<Utc>,
    /// Importance score (0.0–1.0). Higher scores survive consolidation pruning.
    /// Updated automatically based on retrieval frequency.
    pub importance: f32,
    /// Number of times this entry has been retrieved in queries.
    pub retrieval_count: u32,
}

/// Metadata about a memory entry (lightweight view for indexing).
///
/// Used when only the metadata fields are needed, without loading the
/// full content or embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetadata {
    /// The entry's unique identifier.
    pub id: Uuid,
    /// The scope (tier) this entry belongs to.
    pub scope: MemoryScope,
    /// A short preview of the content (first 100 characters).
    pub content_preview: String,
    /// The metadata key-value pairs.
    pub metadata: HashMap<String, serde_json::Value>,
    /// When the entry was created.
    pub created_at: DateTime<Utc>,
    /// Current importance score.
    pub importance: f32,
    /// How many times retrieved.
    pub retrieval_count: u32,
    /// Whether this entry has a computed embedding.
    pub has_embedding: bool,
}

/// A query to the memory system.
///
/// Encapsulates all parameters for a memory search, supporting
/// text search, semantic search, and hybrid modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryQuery {
    /// The query text to search for.
    pub query: String,
    /// Which scope to search in.
    pub scope: MemoryScope,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Minimum relevance score threshold (0.0–1.0). Results below this are excluded.
    pub min_score: f32,
    /// Optional metadata filter: only return entries that have these key-value pairs.
    pub metadata_filter: Option<HashMap<String, serde_json::Value>>,
    /// Which search method to use.
    pub search_type: MemorySearchType,
    /// If searching within a time range, the start of the range.
    pub since: Option<DateTime<Utc>>,
    /// If searching within a time range, the end of the range.
    pub until: Option<DateTime<Utc>>,
}

/// The search method to use for a memory query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemorySearchType {
    /// Keyword-based full-text search using Tantivy (BM25 ranking).
    FullText,
    /// Vector similarity search using cosine distance on embeddings.
    Semantic,
    /// Combination of full-text and semantic search using RRF (Reciprocal Rank Fusion).
    Hybrid,
}

/// The result of a memory search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchResult {
    /// The matching memory entry.
    pub entry: MemoryEntry,
    /// Relevance score (0.0–1.0). Interpretation varies by search type:
    /// FullText = normalized BM25 score, Semantic = cosine similarity, Hybrid = RRF score.
    pub score: f32,
    /// Which search method produced this result.
    pub search_type: MemorySearchType,
}
