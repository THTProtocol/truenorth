//! `IdentityMemoryStore` — cross-project SQLite persistence for identity memory.
//!
//! Uses the same schema as `ProjectMemoryStore` but scoped to `MemoryScope::Identity`.
//! The database lives at a user-level path (not inside a project directory) so that
//! identity memory persists across project switches.
//!
//! In addition to generic `MemoryEntry` storage, this store has a dedicated
//! method for saving and loading the structured `UserProfile` as a single
//! well-known JSON entry.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use truenorth_core::traits::embedding_provider::EmbeddingProvider;
use truenorth_core::traits::memory::MemoryError;
use truenorth_core::types::memory::{MemoryEntry, MemoryScope};

use crate::identity::profile::UserProfile;
use crate::project::deduplicator::Deduplicator;

/// Well-known metadata key used to identify the profile entry.
const PROFILE_ENTRY_KEY: &str = "__user_profile__";

/// Cross-project identity memory store backed by SQLite.
///
/// Identical schema to `ProjectMemoryStore` but scoped to `MemoryScope::Identity`.
/// The store is designed for cross-project use: the database path should be in a
/// user home directory rather than inside a project workspace.
#[derive(Debug, Clone)]
pub struct IdentityMemoryStore {
    /// Path to the SQLite database file.
    db_path: PathBuf,
    /// Semantic deduplicator.
    deduplicator: Arc<Deduplicator>,
    /// Optional embedding provider.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl IdentityMemoryStore {
    /// Create or open an `IdentityMemoryStore`.
    ///
    /// Enables WAL mode and creates the `identity_entries` table if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::StorageError` if the database cannot be opened.
    pub fn new(
        db_path: PathBuf,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
        dedup_threshold: f32,
    ) -> Result<Self, MemoryError> {
        Self::ensure_schema(&db_path)?;
        Ok(Self {
            db_path,
            deduplicator: Arc::new(Deduplicator::new(dedup_threshold)),
            embedding_provider,
        })
    }

    /// Open a connection in WAL mode.
    fn open_conn(&self) -> Result<rusqlite::Connection, MemoryError> {
        let conn =
            rusqlite::Connection::open(&self.db_path).map_err(|e| MemoryError::StorageError {
                message: format!("Cannot open identity DB: {e}"),
            })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot set WAL on identity DB: {e}"),
            })?;
        Ok(conn)
    }

    /// Ensure the identity schema exists.
    fn ensure_schema(db_path: &PathBuf) -> Result<(), MemoryError> {
        let conn =
            rusqlite::Connection::open(db_path).map_err(|e| MemoryError::StorageError {
                message: format!("Cannot open identity DB for schema: {e}"),
            })?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS identity_entries (
                id              TEXT NOT NULL PRIMARY KEY,
                content         TEXT NOT NULL,
                metadata        TEXT NOT NULL DEFAULT '{}',
                embedding       BLOB,
                scope           TEXT NOT NULL DEFAULT 'Identity',
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                importance      REAL NOT NULL DEFAULT 0.5,
                retrieval_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_identity_scope
                ON identity_entries(scope);
            CREATE INDEX IF NOT EXISTS idx_identity_created
                ON identity_entries(scope, created_at DESC);
            ",
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot create identity schema: {e}"),
        })?;
        Ok(())
    }

    /// Write a new identity memory entry with deduplication.
    ///
    /// Checks for semantic duplicates before inserting. If a duplicate is found,
    /// the existing entry's content is updated.
    #[instrument(skip(self, content, metadata))]
    pub async fn write_entry(
        &self,
        content: String,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<MemoryEntry, MemoryError> {
        let now = Utc::now();
        let mut entry = MemoryEntry {
            id: Uuid::new_v4(),
            scope: MemoryScope::Identity,
            content: content.clone(),
            metadata,
            embedding: None,
            created_at: now,
            updated_at: now,
            importance: 0.7, // Identity entries start with higher importance.
            retrieval_count: 0,
        };

        // Generate embedding.
        if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&content).await {
                Ok(vec) => entry.embedding = Some(vec),
                Err(e) => warn!("Embedding failed for identity entry: {e}"),
            }
        }

        // Deduplication check.
        if entry.embedding.is_some() {
            let existing = self.load_all_with_embeddings()?;
            if let Some(dup_id) = self.deduplicator.find_duplicate(&entry, &existing) {
                debug!("Duplicate found in identity store, updating {}", dup_id);
                return self.update_entry_content(dup_id, content, entry.embedding.clone()).await;
            }
        }

        self.insert_entry(&entry)?;
        info!("Wrote identity memory entry {}", entry.id);
        Ok(entry)
    }

    /// Insert a new entry into the identity table.
    fn insert_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let metadata_json = serde_json::to_string(&entry.metadata).unwrap_or_default();
        let embedding_blob: Option<Vec<u8>> = entry.embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });
        conn.execute(
            "INSERT INTO identity_entries
             (id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                entry.id.to_string(),
                entry.content,
                metadata_json,
                embedding_blob,
                "Identity",
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.importance,
                entry.retrieval_count,
            ],
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot insert identity entry: {e}"),
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
            "UPDATE identity_entries SET content = ?1, embedding = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![new_content, embedding_blob, now.to_rfc3339(), id.to_string()],
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot update identity entry: {e}"),
        })?;
        self.get_entry(id).await
    }

    /// Retrieve a single identity entry by ID.
    pub async fn get_entry(&self, id: Uuid) -> Result<MemoryEntry, MemoryError> {
        let conn = self.open_conn()?;
        conn.query_row(
            "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
             FROM identity_entries WHERE id = ?1",
            rusqlite::params![id.to_string()],
            identity_row_to_entry,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => MemoryError::EntryNotFound { id },
            e => MemoryError::StorageError { message: format!("Identity DB read error: {e}") },
        })
    }

    /// Load all identity entries with embeddings (for deduplication scan).
    fn load_all_with_embeddings(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM identity_entries WHERE embedding IS NOT NULL",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare identity dedup query: {e}"),
            })?;
        let entries: Vec<MemoryEntry> = stmt
            .query_map([], identity_row_to_entry)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot query identity entries: {e}"),
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// List recent identity entries since a given timestamp.
    pub async fn list_recent(
        &self,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM identity_entries WHERE created_at >= ?1
                 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare identity list_recent: {e}"),
            })?;
        let entries: Vec<MemoryEntry> = stmt
            .query_map(rusqlite::params![since.to_rfc3339(), limit as i64], identity_row_to_entry)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot query identity recent: {e}"),
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Load all identity entries (used for search index rebuild).
    pub fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.open_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count
                 FROM identity_entries ORDER BY created_at DESC",
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot prepare identity load_all: {e}"),
            })?;
        let entries: Vec<MemoryEntry> = stmt
            .query_map([], identity_row_to_entry)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot load_all identity: {e}"),
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Delete an identity entry by ID.
    pub async fn delete_entry(&self, id: Uuid) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let rows = conn
            .execute(
                "DELETE FROM identity_entries WHERE id = ?1",
                rusqlite::params![id.to_string()],
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot delete identity entry: {e}"),
            })?;
        if rows == 0 {
            return Err(MemoryError::EntryNotFound { id });
        }
        Ok(())
    }

    /// Bump retrieval count and importance for an identity entry.
    pub async fn record_retrieval(&self, id: Uuid) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let rows = conn
            .execute(
                "UPDATE identity_entries
                 SET retrieval_count = retrieval_count + 1,
                     importance = MIN(importance + 0.03, 1.0),
                     updated_at = ?2
                 WHERE id = ?1",
                rusqlite::params![id.to_string(), Utc::now().to_rfc3339()],
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot record identity retrieval: {e}"),
            })?;
        if rows == 0 {
            return Err(MemoryError::EntryNotFound { id });
        }
        Ok(())
    }

    /// Save the `UserProfile` to the identity store as a special JSON entry.
    ///
    /// Uses an upsert keyed on a well-known entry metadata marker so there is
    /// always exactly one profile entry in the database.
    pub async fn save_profile(&self, profile: &UserProfile) -> Result<(), MemoryError> {
        let profile_json = profile.to_json().map_err(|e| MemoryError::WriteError {
            scope: MemoryScope::Identity,
            message: format!("Cannot serialize UserProfile: {e}"),
        })?;

        let conn = self.open_conn()?;
        let now = Utc::now();

        // Check if the profile entry already exists.
        let existing_id: Option<String> = conn
            .query_row(
                "SELECT id FROM identity_entries WHERE json_extract(metadata, '$.entry_key') = ?1",
                rusqlite::params![PROFILE_ENTRY_KEY],
                |row| row.get(0),
            )
            .ok();

        let metadata_json = serde_json::json!({"entry_key": PROFILE_ENTRY_KEY}).to_string();

        if let Some(id_str) = existing_id {
            conn.execute(
                "UPDATE identity_entries SET content = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![profile_json, now.to_rfc3339(), id_str],
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot update profile entry: {e}"),
            })?;
        } else {
            let new_id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO identity_entries
                 (id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count)
                 VALUES (?1, ?2, ?3, NULL, 'Identity', ?4, ?5, 1.0, 0)",
                rusqlite::params![
                    new_id.to_string(),
                    profile_json,
                    metadata_json,
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ],
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot insert profile entry: {e}"),
            })?;
        }

        debug!("Saved UserProfile to identity store");
        Ok(())
    }

    /// Load the `UserProfile` from the identity store.
    ///
    /// Returns `MemoryError::EntryNotFound` if no profile has been saved yet.
    pub async fn load_profile(&self) -> Result<UserProfile, MemoryError> {
        let conn = self.open_conn()?;
        let content: String = conn
            .query_row(
                "SELECT content FROM identity_entries WHERE json_extract(metadata, '$.entry_key') = ?1",
                rusqlite::params![PROFILE_ENTRY_KEY],
                |row| row.get(0),
            )
            .map_err(|_| MemoryError::EntryNotFound { id: Uuid::nil() })?;

        UserProfile::from_json(&content).map_err(|e| MemoryError::ReadError {
            scope: MemoryScope::Identity,
            message: format!("Cannot deserialize UserProfile: {e}"),
        })
    }

    /// Return the total number of identity entries.
    pub fn entry_count(&self) -> Result<usize, MemoryError> {
        let conn = self.open_conn()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM identity_entries",
                [],
                |row| row.get(0),
            )
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot count identity entries: {e}"),
            })?;
        Ok(count as usize)
    }

    /// Upsert an identity entry (used by reindexer for external edits).
    pub fn upsert_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let conn = self.open_conn()?;
        let metadata_json = serde_json::to_string(&entry.metadata).unwrap_or_default();
        let embedding_blob: Option<Vec<u8>> = entry.embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });
        conn.execute(
            "INSERT OR REPLACE INTO identity_entries
             (id, content, metadata, embedding, scope, created_at, updated_at, importance, retrieval_count)
             VALUES (?1, ?2, ?3, ?4, 'Identity', ?5, ?6, ?7, ?8)",
            rusqlite::params![
                entry.id.to_string(),
                entry.content,
                metadata_json,
                embedding_blob,
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
                entry.importance,
                entry.retrieval_count,
            ],
        )
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot upsert identity entry: {e}"),
        })?;
        Ok(())
    }
}

/// Map a SQLite identity row to a `MemoryEntry`.
fn identity_row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
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
        scope: MemoryScope::Identity,
        content,
        metadata,
        embedding,
        created_at,
        updated_at,
        importance,
        retrieval_count: retrieval_count as u32,
    })
}
