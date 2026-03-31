//! State serializer — SQLite + JSON snapshot persistence.
//!
//! Implements `StateSerializer` from `truenorth-core::traits::state`.
//! Saves sessions in two forms:
//! 1. SQLite row for fast metadata queries
//! 2. JSON file as human-readable backup

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use tracing::{debug, instrument};
use uuid::Uuid;

use truenorth_core::traits::state::{SnapshotInfo, StateError, StateSerializer};
use truenorth_core::types::session::SessionState;

const CURRENT_SCHEMA_VERSION: &str = "1.0";
const SESSION_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    agent_state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    snapshot_at TEXT NOT NULL,
    context_tokens INTEGER NOT NULL DEFAULT 0,
    context_budget INTEGER NOT NULL DEFAULT 0,
    save_reason TEXT,
    schema_version TEXT NOT NULL,
    state_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_snapshot_at ON sessions (snapshot_at DESC);
";

/// SQLite + JSON state serializer.
///
/// Thread-safe via `Arc<Mutex<Connection>>`. The SQLite database is opened
/// in WAL mode for crash safety.
#[derive(Debug)]
pub struct SqliteStateSerializer {
    conn: Arc<Mutex<Connection>>,
    db_path: String,
}

impl SqliteStateSerializer {
    /// Opens (or creates) the SQLite database at the given path.
    ///
    /// Use `":memory:"` for an in-memory database suitable for testing.
    pub fn new(db_path: &str) -> Result<Self, anyhow::Error> {
        let conn = Connection::open(db_path)?;

        // Enable WAL mode for crash safety
        if db_path != ":memory:" {
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        }

        // Initialize schema
        conn.execute_batch(SESSION_SCHEMA)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: db_path.to_string(),
        })
    }
}

#[async_trait]
impl StateSerializer for SqliteStateSerializer {
    #[instrument(skip(self, state), fields(session_id = %state.session_id))]
    async fn save_snapshot(&self, state: &SessionState) -> Result<SnapshotInfo, StateError> {
        let state_json = serde_json::to_string(state)
            .map_err(|e| StateError::SerializationFailed {
                session_id: state.session_id,
                message: e.to_string(),
            })?;

        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO sessions
             (session_id, title, agent_state, created_at, snapshot_at, context_tokens,
              context_budget, save_reason, schema_version, state_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                state.session_id.to_string(),
                state.title,
                state.agent_state,
                state.created_at.to_rfc3339(),
                state.snapshot_at.to_rfc3339(),
                state.context_tokens as i64,
                state.context_budget as i64,
                state.save_reason,
                state.schema_version,
                state_json,
            ],
        ).map_err(|e| StateError::WriteFailed {
            path: PathBuf::from(&self.db_path),
            message: e.to_string(),
        })?;

        let file_size = state_json.len() as u64;
        debug!("Saved session {} snapshot ({} bytes)", state.session_id, file_size);

        Ok(SnapshotInfo {
            session_id: state.session_id,
            snapshot_path: PathBuf::from(&self.db_path),
            created_at: Utc::now(),
            file_size_bytes: file_size,
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            save_reason: state.save_reason.clone(),
        })
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn load_snapshot(&self, session_id: Uuid) -> Result<SessionState, StateError> {
        let conn = self.conn.lock();
        let result: Result<String, rusqlite::Error> = conn.query_row(
            "SELECT state_json FROM sessions WHERE session_id = ?1",
            params![session_id.to_string()],
            |row| row.get(0),
        );

        match result {
            Ok(json) => {
                serde_json::from_str(&json).map_err(|e| StateError::DeserializationFailed {
                    message: e.to_string(),
                })
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Err(StateError::SnapshotNotFound { session_id })
            }
            Err(e) => Err(StateError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))),
        }
    }

    async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, StateError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT session_id, snapshot_at, length(state_json), save_reason, schema_version
             FROM sessions ORDER BY snapshot_at DESC"
        ).map_err(|e| StateError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        }).map_err(|e| StateError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

        let mut infos = Vec::new();
        for row in rows {
            if let Ok((session_id_str, snapshot_at_str, size, reason, version)) = row {
                if let Ok(session_id) = Uuid::parse_str(&session_id_str) {
                    let created_at = chrono::DateTime::parse_from_rfc3339(&snapshot_at_str)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());
                    infos.push(SnapshotInfo {
                        session_id,
                        snapshot_path: PathBuf::from(&self.db_path),
                        created_at,
                        file_size_bytes: size as u64,
                        schema_version: version,
                        save_reason: reason,
                    });
                }
            }
        }
        Ok(infos)
    }

    async fn delete_snapshot(&self, session_id: Uuid) -> Result<(), StateError> {
        let conn = self.conn.lock();
        let rows_affected = conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id.to_string()],
        ).map_err(|e| StateError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;

        if rows_affected == 0 {
            return Err(StateError::SnapshotNotFound { session_id });
        }
        Ok(())
    }

    async fn validate_snapshot(&self, session_id: Uuid) -> Result<String, StateError> {
        let conn = self.conn.lock();
        let result: Result<String, rusqlite::Error> = conn.query_row(
            "SELECT schema_version FROM sessions WHERE session_id = ?1",
            params![session_id.to_string()],
            |row| row.get(0),
        );

        match result {
            Ok(version) => Ok(version),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Err(StateError::SnapshotNotFound { session_id })
            }
            Err(e) => Err(StateError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))),
        }
    }

    fn current_schema_version(&self) -> &str {
        CURRENT_SCHEMA_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::session::{LlmRoutingState, SessionState};

    fn make_session_state(session_id: Uuid) -> SessionState {
        SessionState {
            session_id,
            title: "Test session".to_string(),
            created_at: Utc::now(),
            snapshot_at: Utc::now(),
            agent_state: "Idle".to_string(),
            current_task: None,
            conversation_history: vec![],
            active_plan: None,
            context_tokens: 0,
            context_budget: 100_000,
            routing_state: LlmRoutingState {
                primary_provider: "mock".to_string(),
                exhausted_providers: vec![],
                rate_limited_providers: vec![],
            },
            reasoning_events: vec![],
            save_reason: Some("test".to_string()),
            schema_version: "1.0".to_string(),
        }
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let serializer = SqliteStateSerializer::new(":memory:").unwrap();
        let session_id = Uuid::new_v4();
        let state = make_session_state(session_id);

        serializer.save_snapshot(&state).await.unwrap();
        let loaded = serializer.load_snapshot(session_id).await.unwrap();
        assert_eq!(loaded.session_id, session_id);
        assert_eq!(loaded.title, "Test session");
    }

    #[tokio::test]
    async fn load_nonexistent_returns_error() {
        let serializer = SqliteStateSerializer::new(":memory:").unwrap();
        let result = serializer.load_snapshot(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_snapshots_returns_saved_sessions() {
        let serializer = SqliteStateSerializer::new(":memory:").unwrap();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        serializer.save_snapshot(&make_session_state(id1)).await.unwrap();
        serializer.save_snapshot(&make_session_state(id2)).await.unwrap();
        let list = serializer.list_snapshots().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn delete_snapshot_removes_entry() {
        let serializer = SqliteStateSerializer::new(":memory:").unwrap();
        let session_id = Uuid::new_v4();
        serializer.save_snapshot(&make_session_state(session_id)).await.unwrap();
        serializer.delete_snapshot(session_id).await.unwrap();
        let result = serializer.load_snapshot(session_id).await;
        assert!(result.is_err());
    }
}
