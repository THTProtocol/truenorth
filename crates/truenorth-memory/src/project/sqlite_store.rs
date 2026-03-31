//! `ProjectMemoryStore` — WAL-mode SQLite backend for the project memory tier.
//!
//! Schema
//! ------
//! ```sql
//! CREATE TABLE memory_entries (
//!     id            TEXT PRIMARY KEY,   -- UUID v4
//!     content       TEXT NOT NULL,
//!     metadata      TEXT NOT NULL,      -- JSON object
//!     embedding     BLOB,               -- little-endian f32 array
//!     scope         TEXT NOT NULL,      -- 'Project'
//!     created_at    TEXT NOT NULL,      -- RFC 3339
//!     updated_at    TEXT NOT NULL,
//!     importance    REAL NOT NULL,
//!     retrieval_count INTEGER NOT NULL
//! );
//! ```
//!
//! WAL mode is enabled on every connection open. Operations that mutate the
//! database use `rusqlite::Connection` obtained from a per-call open to avoid
//! shared-connection contention.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use truenorth_core::traits::embedding_provider::EmbeddingProvider;
use truenorth_core::traits::memory::MemoryError;
use truenorth_core::types::memory::{MemoryEntry, MemoryScope};

use crate::project::deduplicator::Deduplicator;
use crate::project::markdown_writer::MarkdownWriter;

/// SQLite-backed project memory store.
///
/// Opens a SQLite database in WAL mode for concurrent read access while
/// maintaining serialized writes. Each write is preceded by a semantic
/// deduplication check via the configured `EmbeddingProvider`.
#[derive(Debug, Clone)]
pub struct ProjectMemoryStore {
    /// Path to the SQLite database file.
    db_path: PathBuf,
    /// Markdown writer for Obsidian vault synchronization.
    markdown_writer: Arc<MarkdownWriter>,
    /// Semantic deduplicator.
    deduplicator: Arc<Deduplicator>,
    /// Optional embedding provider.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl ProjectMemoryStore {
    /// Create or open a `ProjectMemoryStore`.
    ///
    /// Opens the SQLite database at `db_path`, enables WAL mode, and ensures
    /// the `memory_entries` table exists. Then initializes the Markdown writer
    /// and deduplicator.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::StorageError` if the database cannot be opened or
    /// the schema cannot be created.
    pub fn new(
        db_path: PathBuf,
        vault_dir: PathBuf,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
        dedup_threshold: f32,
    ) -> Result<Self, MemoryError> {
        Self::ensure_schema(&db_path)?;
        let markdown_writer = Arc::new(MarkdownWriter::new(vault_dir.join("project")));
        let deduplicator = Arc::new(Deduplicator::new(dedup_threshold));
        Ok(Self {
            db_path,
            markdown_writer,
            deduplicator,
            embedding_provider,
        })
    }

    /// Open a SQLite connection in WAL mode.
    fn open_conn(&self) -> Result<rusqlite::Connection, MemoryError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| MemoryError::StorageError {
                message: format!("Cannot open project DB: {e}"),
            })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot set WAL: {e}"),
            })?;
        Ok(conn)
    }

    /// Ensure the database schema exists.
    fn ensure_schema(db_path: &PathBuf) -> Result<(), MemoryError> {
        let conn =
            rusqlite::Connection::open(db_path).map_err(|e| MemoryError::StorageError {
                message: format!("Cannot open project DB for schema: {e}"),
            })?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_entries (
                id              TEXT NOT NULL PRIMARY KEY,
                content         TEXT NOT NULL,
                metadata        TEXT NOT NULL DEFAULT '{}',
                embedding       BLOB,
                scope           TEXT NOT NULL DEFAULT 'Project',
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                importance      REAL NOT NULL DEFAULT 0.5,
                retrieval_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_memory_scope
                ON memory_entries(scope);
            CREATE INDEX IF NOT EXISTS idx_memory_created
                ON memory_entries(scope, created_at DESC);
            ",
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot create project schema: {e}"),
        })?;
        Ok(())
    }

    /// Write a new memory entry (with deduplication + embedding).
    ///
    /// Before inserting, checks for semantic duplicates. If a duplicate is found
    /// above the threshold, the existing entry is updated instead of creating a new one.
    ///
    /// Also writes an Obsidian-compatible Markdown file to the vault directory.
    #[instrument(skip(self, content, metadata))]
    pub async fn write_entry(
        &self,
        content: String,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<MemoryEntry, MemoryError> {
        let now = Utc::now();
        let mut entry = MemoryEntry {
            id: Uuid::new_v4(),
            scope: MemoryScope::Project,
            content: content.clone(),
            metadata,
            embedding: None,
            created_at: now,
            updated_at: now,
            importance: 0.5,
            retrieval_count: 0,
        };

        // Generate embedding.
        if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&content).await {
                Ok(vec) => entry.embedding = Some(vec),
                Err(e) => warn!("Embedding failed for project entry: {e}"),
            }
        }

        // Deduplication check.
        if entry.embedding.is_some() {
            let existing = self.load_all_with_embeddings()?;
            if let Some(dup_id) = self.deduplicator.find_duplicate(&entry, &existing) {
                debug!("Semantic duplicate found for project entry, updating {}", dup_id);
                return self.update_entry_content(dup_id, content, entry.embedding.clone()).await;
            }
        }

        // Insert into SQLite.
        self.insert_entry(&entry)?;

        // Write Obsidian Markdown file.
        if let Err(e) = self.markdown_writer.write(&entry) {
            warn!("Failed to write Obsidian Markdown for entry {}: {e}", entry.id);
        }

        info!("Wrote project memory entry {}", entry.id);
        Ok(entry)
    }

    /// Insert a new entry into SQLite.
    fn insert_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let metadata_json = serde_json::to_string(&entry.metadata).unwrap_or_default();
        let embedding_blob: Option<Vec<u8>> = entry.embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });
        conn.execute(
            "INSERT INTO memory_entries
             (id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                entry.id.to_string(),
                entry.content,
                metadata_json,
                embedding_blob,
                format!("{:?}", entry.scope),
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.importance,
                entry.retrieval_count,
            ],
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot insert project entry: {e}"),
        })?;
        Ok(())
    }

    /// Update an existing entry's content (duplicate-merge path).
    async fn update_entry_content(
        &self,
        id: Uuid,
        new_content: String,
        new_embedding: Option<Vec<f32>>,
    ) -> Result<MemoryEntry, MemoryError> {
        let now = Utc::now();
        let embedding_blob: Option<Vec<u8>> = new_embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });
        let conn = self.open_conn()?;
        conn.execute(
            "UPDATE memory_entries SET content = ?1, embedding = ?2, updated_at = ?3
             WHERE id = ?4",
            rusqlite::params![
                new_content,
                embedding_blob,
                now.to_rfc3339(),
                id.to_string(),
            ],
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot update project entry: {e}"),
        })?;
        self.get_entry(id).await
    }

    /// Retrieve a single entry by ID.
    pub async fn get_entry(&self, id: Uuid) -> Result<MemoryEntry, MemoryError> {
        let conn = self.open_conn()?;
        let entry = conn
            .query_row(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM memory_entries WHERE id = ?1",
                rusqlite::params![id.to_string()],
                row_to_entry,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => MemoryError::EntryNotFound { id },
                e => MemoryError::StorageError { message: format!("DB read error: {e}") },
            })?;
        Ok(entry)
    }

    /// List all entries with embeddings (used for deduplication scan).
    fn load_all_with_embeddings(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM memory_entries WHERE embedding IS NOT NULL AND scope = 'Project'",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare dedup query: {e}"),
            })?;
        let entries: Vec<MemoryEntry> = stmt
            .query_map([], row_to_entry)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot query for dedup: {e}"),
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// List recent entries since a given timestamp, ordered descending.
    pub async fn list_recent(
        &self,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM memory_entries
                 WHERE scope = 'Project' AND created_at >= ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare list_recent: {e}"),
            })?;
        let entries: Vec<MemoryEntry> = stmt
            .query_map(rusqlite::params![since.to_rfc3339(), limit as i64], row_to_entry)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot query recent: {e}"),
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Load all entries in the project scope (used for index rebuilds).
    pub fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM memory_entries WHERE scope = 'Project'
                 ORDER BY created_at DESC",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare load_all: {e}"),
            })?;
        let entries: Vec<MemoryEntry> = stmt
            .query_map([], row_to_entry)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot load_all: {e}"),
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Delete a memory entry by ID.
    pub async fn delete_entry(&self, id: Uuid) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let rows = conn
            .execute(
                "DELETE FROM memory_entries WHERE id = ?1",
                rusqlite::params![id.to_string()],
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot delete entry: {e}"),
            })?;
        if rows == 0 {
            return Err(MemoryError::EntryNotFound { id });
        }
        // Remove the Markdown file if it exists.
        if let Err(e) = self.markdown_writer.delete(id) {
            warn!("Failed to delete Markdown file for {}: {e}", id);
        }
        Ok(())
    }

    /// Bump retrieval count and importance for an entry.
    pub async fn record_retrieval(&self, id: Uuid) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let rows = conn
            .execute(
                "UPDATE memory_entries
                 SET retrieval_count = retrieval_count + 1,
                     importance = MIN(importance + 0.05, 1.0),
                     updated_at = ?2
                 WHERE id = ?1",
                rusqlite::params![id.to_string(), Utc::now().to_rfc3339()],
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot record retrieval: {e}"),
            })?;
        if rows == 0 {
            return Err(MemoryError::EntryNotFound { id });
        }
        Ok(())
    }

    /// Return the total number of project memory entries.
    pub fn entry_count(&self) -> Result<usize, MemoryError> {
        let conn = self.open_conn()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_entries WHERE scope = 'Project'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot count entries: {e}"),
            })?;
        Ok(count as usize)
    }

    /// Upsert an entry (used by the Obsidian reindexer for external edits).
    pub fn upsert_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let metadata_json = serde_json::to_string(&entry.metadata).unwrap_or_default();
        let embedding_blob: Option<Vec<u8>> = entry.embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });
        conn.execute(
            "INSERT OR REPLACE INTO memory_entries
             (id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                entry.id.to_string(),
                entry.content,
                metadata_json,
                embedding_blob,
                format!("{:?}", entry.scope),
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.importance,
                entry.retrieval_count,
            ],
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot upsert entry: {e}"),
        })?;
        Ok(())
    }
}

/// Map a SQLite row to a `MemoryEntry`.
fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let id_str: String = row.get(0)?;
    let id = Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4());

    let content: String = row.get(1)?;
    let metadata_json: String = row.get(2)?;
    let metadata: HashMap<String, serde_json::Value> =
        serde_json::from_str(&metadata_json).unwrap_or_default();

    let embedding_blob: Option<Vec<u8>> = row.get(3)?;
    let embedding: Option<Vec<f32>> = embedding_blob.map(|blob| {
        blob.chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    });

    let scope_str: String = row.get(4)?;
    let scope = match scope_str.as_str() {
        "Session" => MemoryScope::Session,
        "Identity" => MemoryScope::Identity,
        _ => MemoryScope::Project,
    };

    let created_str: String = row.get(5)?;
    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    let updated_str: String = row.get(6)?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    let importance: f32 = row.get(7)?;
    let retrieval_count: i64 = row.get(8)?;

    Ok(MemoryEntry {
        id,
        scope,
        content,
        metadata,
        embedding,
        created_at,
        updated_at,
        importance,
        retrieval_count: retrieval_count as u32,
    })
}

/// Public helper: convert a raw SQLite row to a `MemoryEntry` (used by other modules).
pub(crate) fn sqlite_row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    row_to_entry(row)
}
