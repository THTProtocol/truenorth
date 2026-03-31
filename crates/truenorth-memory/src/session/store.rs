//! `SessionMemoryStore` — in-memory session tier backed by `Arc<RwLock<HashMap>>`.
//!
//! Each session is identified by a `SessionId` (Uuid). Entries are kept in
//! a `Vec<MemoryEntry>` for each session. On session close, the entries are
//! persisted to SQLite for the consolidation pipeline.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use truenorth_core::traits::embedding_provider::EmbeddingProvider;
use truenorth_core::traits::memory::{CompactionResult, MemoryError};
use truenorth_core::types::memory::{MemoryEntry, MemoryScope, MemorySearchResult, MemorySearchType};

use crate::session::compactor::ContextCompactor;

/// Type alias for a session identifier.
pub type SessionId = Uuid;

/// In-memory session memory store.
///
/// Stores all entries for active sessions in a `HashMap<SessionId, Vec<MemoryEntry>>`.
/// The entire structure is protected by an `Arc<RwLock<>>` for safe concurrent access.
/// On session close, entries are flushed to the SQLite session archive for later
/// consolidation.
#[derive(Debug, Clone)]
pub struct SessionMemoryStore {
    /// In-memory entries: session_id → list of entries (insertion-ordered).
    entries: Arc<RwLock<HashMap<SessionId, Vec<MemoryEntry>>>>,
    /// Path to the SQLite database used for session persistence on close.
    db_path: PathBuf,
    /// Optional embedding provider for semantic search within the session.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    /// Context compactor for summarization when approaching context budget.
    compactor: Arc<ContextCompactor>,
}

impl SessionMemoryStore {
    /// Create a new `SessionMemoryStore`.
    ///
    /// Opens (or creates) the SQLite database at `db_path` for session persistence.
    /// The database is opened here to validate the path; no writes happen until
    /// a session is closed.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::StorageError` if the database cannot be opened.
    pub fn new(
        db_path: PathBuf,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self, MemoryError> {
        // Validate database path is writable.
        Self::ensure_db_schema(&db_path)?;
        Ok(Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            db_path,
            embedding_provider,
            compactor: Arc::new(ContextCompactor::new()),
        })
    }

    /// Ensure the SQLite session archive schema exists.
    fn ensure_db_schema(db_path: &PathBuf) -> Result<(), MemoryError> {
        let conn = rusqlite::Connection::open(db_path).map_err(|e| MemoryError::StorageError {
            message: format!("Cannot open session DB: {e}"),
        })?;
        // Enable WAL mode for concurrent reads.
        conn.execute_batch("PRAGMA journal_mode=WAL;").map_err(|e| MemoryError::StorageError {
            message: format!("Cannot set WAL mode: {e}"),
        })?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_entries (
                id          TEXT NOT NULL PRIMARY KEY,
                session_id  TEXT NOT NULL,
                content     TEXT NOT NULL,
                metadata    TEXT NOT NULL DEFAULT '{}',
                embedding   BLOB,
                scope       TEXT NOT NULL DEFAULT 'Session',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                importance  REAL NOT NULL DEFAULT 0.5,
                retrieval_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_session_entries_session_id
                ON session_entries(session_id);
            CREATE INDEX IF NOT EXISTS idx_session_entries_created_at
                ON session_entries(session_id, created_at DESC);
            ",
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot create session schema: {e}"),
        })?;
        Ok(())
    }

    /// Add a new entry to an active session.
    ///
    /// If the session does not yet exist in the in-memory map, it is created.
    /// If an embedding provider is configured, the entry is embedded immediately.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session to add the entry to.
    /// * `content` - The text content of the entry.
    /// * `metadata` - Structured key-value metadata for filtering.
    #[instrument(skip(self, content, metadata), fields(session_id = %session_id))]
    pub async fn add_entry(
        &self,
        session_id: SessionId,
        content: String,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<MemoryEntry, MemoryError> {
        let now = Utc::now();
        let mut entry = MemoryEntry {
            id: Uuid::new_v4(),
            scope: MemoryScope::Session,
            content: content.clone(),
            metadata,
            embedding: None,
            created_at: now,
            updated_at: now,
            importance: 0.5,
            retrieval_count: 0,
        };

        // Generate embedding if provider is available.
        if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&content).await {
                Ok(vec) => entry.embedding = Some(vec),
                Err(e) => warn!("Failed to embed session entry: {e}"),
            }
        }

        let mut map = self.entries.write().await;
        map.entry(session_id).or_default().push(entry.clone());
        debug!(
            "Added session entry {} to session {}",
            entry.id, session_id
        );
        Ok(entry)
    }

    /// Write an entry to the default session scope (session_id from metadata or new).
    ///
    /// Convenience method that wraps [`add_entry`] using a session_id extracted
    /// from the metadata `"session_id"` key, or a freshly generated UUID if absent.
    pub async fn write_entry(
        &self,
        content: String,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<MemoryEntry, MemoryError> {
        let session_id = metadata
            .get("session_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);
        self.add_entry(session_id, content, metadata).await
    }

    /// Retrieve a single entry by ID from any active session.
    pub async fn get_entry(&self, id: Uuid) -> Result<MemoryEntry, MemoryError> {
        let map = self.entries.read().await;
        for entries in map.values() {
            if let Some(e) = entries.iter().find(|e| e.id == id) {
                return Ok(e.clone());
            }
        }
        Err(MemoryError::EntryNotFound { id })
    }

    /// Retrieve all entries for a specific session, ordered by creation time.
    ///
    /// Returns an empty vector if the session has no entries or doesn't exist.
    #[instrument(skip(self), fields(session_id = %session_id))]
    pub async fn get_entries(&self, session_id: SessionId) -> Vec<MemoryEntry> {
        let map = self.entries.read().await;
        map.get(&session_id).cloned().unwrap_or_default()
    }

    /// Search within a single session using keyword matching.
    ///
    /// Performs simple substring search on `content`. For full BM25 search,
    /// use the [`SearchEngine`](crate::search::SearchEngine) which also indexes
    /// session entries.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Restrict search to this session.
    /// * `query` - Substring to search for (case-insensitive).
    /// * `limit` - Maximum number of results.
    pub async fn search_within_session(
        &self,
        session_id: SessionId,
        query: &str,
        limit: usize,
    ) -> Vec<MemorySearchResult> {
        let query_lower = query.to_lowercase();
        let map = self.entries.read().await;
        let entries = match map.get(&session_id) {
            Some(v) => v,
            None => return Vec::new(),
        };

        let mut results: Vec<MemorySearchResult> = entries
            .iter()
            .filter(|e| e.content.to_lowercase().contains(&query_lower))
            .map(|e| MemorySearchResult {
                entry: e.clone(),
                score: simple_keyword_score(&e.content, query),
                search_type: MemorySearchType::FullText,
            })
            .collect();

        // Sort by score descending.
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    /// List entries created after `since`, across all active sessions.
    pub async fn list_recent(
        &self,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let map = self.entries.read().await;
        let mut all: Vec<MemoryEntry> = map
            .values()
            .flat_map(|v| v.iter().cloned())
            .filter(|e| e.created_at >= since)
            .collect();
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all.truncate(limit);
        Ok(all)
    }

    /// Persist a session's entries to SQLite and clear them from memory.
    ///
    /// Called when a session ends. The persisted entries become available to
    /// the consolidation pipeline for the next autoDream cycle.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::StorageError` if the SQLite write fails. The
    /// in-memory entries are NOT cleared if persistence fails.
    #[instrument(skip(self), fields(session_id = %session_id))]
    pub async fn close_session(&self, session_id: SessionId) -> Result<usize, MemoryError> {
        let entries = {
            let map = self.entries.read().await;
            map.get(&session_id).cloned().unwrap_or_default()
        };

        if entries.is_empty() {
            let mut map = self.entries.write().await;
            map.remove(&session_id);
            return Ok(0);
        }

        let count = entries.len();
        self.persist_to_sqlite(&entries)?;

        // Clear from memory after successful persistence.
        let mut map = self.entries.write().await;
        map.remove(&session_id);

        info!(
            "Closed session {}: persisted {} entries to SQLite",
            session_id, count
        );
        Ok(count)
    }

    /// Persist entries to SQLite session archive.
    fn persist_to_sqlite(&self, entries: &[MemoryEntry]) -> Result<(), MemoryError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| MemoryError::StorageError {
                message: format!("Cannot open session DB for persistence: {e}"),
            })?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();

        let mut stmt = conn
            .prepare(
                "INSERT OR REPLACE INTO session_entries
                 (id, session_id, content, metadata, embedding, scope, created_at, updated_at,
                  importance, retrieval_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare insert: {e}"),
            })?;

        for entry in entries {
            let metadata_json = serde_json::to_string(&entry.metadata).unwrap_or_default();
            let embedding_blob: Option<Vec<u8>> = entry.embedding.as_ref().map(|v| {
                v.iter().flat_map(|f| f.to_le_bytes()).collect()
            });
            let session_id_str = entry
                .metadata
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            stmt.execute(rusqlite::params![
                entry.id.to_string(),
                session_id_str,
                entry.content,
                metadata_json,
                embedding_blob,
                format!("{:?}", entry.scope),
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.importance,
                entry.retrieval_count,
            ])
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot insert session entry: {e}"),
            })?;
        }

        Ok(())
    }

    /// Compact session history via the [`ContextCompactor`].
    ///
    /// Summarizes the conversation when the context budget is approaching its
    /// threshold. Replaces verbose history with a compact summary prefix.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session to compact.
    /// * `budget_hint` - Approximate token budget remaining (used to size summary).
    pub async fn compact(
        &self,
        session_id: SessionId,
        budget_hint: usize,
    ) -> Result<CompactionResult, MemoryError> {
        let entries = self.get_entries(session_id).await;
        if entries.is_empty() {
            return Ok(CompactionResult {
                summary: String::new(),
                tokens_before: 0,
                tokens_after: 0,
                messages_removed: 0,
                session_id,
            });
        }

        let result = self.compactor.compact(session_id, &entries, budget_hint).await?;

        // Replace in-memory entries with a single summary entry.
        let now = Utc::now();
        let summary_entry = MemoryEntry {
            id: Uuid::new_v4(),
            scope: MemoryScope::Session,
            content: result.summary.clone(),
            metadata: {
                let mut m = HashMap::new();
                m.insert("type".into(), serde_json::json!("compaction_summary"));
                m.insert("session_id".into(), serde_json::json!(session_id.to_string()));
                m
            },
            embedding: None,
            created_at: now,
            updated_at: now,
            importance: 0.9,
            retrieval_count: 0,
        };

        let mut map = self.entries.write().await;
        map.insert(session_id, vec![summary_entry]);

        Ok(result)
    }

    /// Delete a single entry from any active session.
    pub async fn delete_entry(&self, id: Uuid) -> Result<(), MemoryError> {
        let mut map = self.entries.write().await;
        for entries in map.values_mut() {
            let before = entries.len();
            entries.retain(|e| e.id != id);
            if entries.len() < before {
                return Ok(());
            }
        }
        Err(MemoryError::EntryNotFound { id })
    }

    /// Bump retrieval count and importance for an entry.
    pub async fn record_retrieval(&self, id: Uuid) -> Result<(), MemoryError> {
        let mut map = self.entries.write().await;
        for entries in map.values_mut() {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.retrieval_count += 1;
                // Decay-based importance: each retrieval adds a small boost capped at 1.0.
                e.importance = (e.importance + 0.05).min(1.0);
                e.updated_at = Utc::now();
                return Ok(());
            }
        }
        Err(MemoryError::EntryNotFound { id })
    }

    /// Return a snapshot of all currently loaded session IDs.
    pub async fn active_sessions(&self) -> Vec<SessionId> {
        let map = self.entries.read().await;
        map.keys().copied().collect()
    }
}

/// Naive keyword relevance score: fraction of query words found in the content.
fn simple_keyword_score(content: &str, query: &str) -> f32 {
    let content_lower = content.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return 0.0;
    }
    let matches = words.iter().filter(|w| content_lower.contains(&w.to_lowercase())).count();
    matches as f32 / words.len() as f32
}
