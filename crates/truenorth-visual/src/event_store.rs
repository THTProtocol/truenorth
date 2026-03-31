/// SQLite-backed persistent event store for the Visual Reasoning Layer.
///
/// All reasoning events are persisted here in append-only fashion. The store
/// uses WAL (Write-Ahead Logging) mode for better concurrent read performance.
/// Events are stored as JSON payloads with metadata columns for efficient
/// querying by session, task, and time range.
///
/// The store is the source of truth for event replay — live subscribers use
/// the `EventBus` broadcast channel, but the store guarantees completeness:
/// even if a subscriber is slow or offline, it can catch up via replay.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info};
use uuid::Uuid;

use truenorth_core::traits::reasoning::ReasoningError;
use truenorth_core::types::event::{EventId, ReasoningEvent};

use crate::types::StoredEvent;

/// The SQLite-backed event store for reasoning events.
///
/// Thread-safe via an internal `Arc<Mutex<Connection>>`. All mutations are
/// serialised through the mutex; reads acquire the same lock. For higher
/// read concurrency, a connection pool can be introduced in a future version
/// without changing the public API.
#[derive(Debug, Clone)]
pub struct ReasoningEventStore {
    conn: Arc<Mutex<Connection>>,
}

impl ReasoningEventStore {
    /// Opens (or creates) a `ReasoningEventStore` at the given file path.
    ///
    /// Runs the DDL migration to ensure the `reasoning_events` table exists,
    /// enables WAL mode for better concurrent performance, and sets pragmas
    /// for durability and speed.
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened or the schema
    /// migration fails.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ReasoningError> {
        let conn = Connection::open(path).map_err(|e| ReasoningError::PersistenceError {
            message: format!("Failed to open SQLite database: {e}"),
        })?;

        // WAL mode: readers don't block writers, writers don't block readers.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to set WAL pragmas: {e}"),
            })?;

        Self::migrate(&conn)?;

        info!("ReasoningEventStore opened successfully");

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Opens an in-memory store suitable for tests.
    ///
    /// All data is lost when the store is dropped.
    pub fn open_in_memory() -> Result<Self, ReasoningError> {
        let conn = Connection::open_in_memory().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Failed to open in-memory SQLite: {e}"),
        })?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to set pragmas: {e}"),
            })?;

        Self::migrate(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs DDL migrations to ensure the schema is up to date.
    fn migrate(conn: &Connection) -> Result<(), ReasoningError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS reasoning_events (
                id           TEXT    NOT NULL PRIMARY KEY,  -- UUID as text
                session_id   TEXT    NOT NULL,              -- UUID
                task_id      TEXT,                          -- UUID, nullable
                event_type   TEXT    NOT NULL,              -- discriminant tag
                payload      TEXT    NOT NULL,              -- full event JSON
                timestamp    TEXT    NOT NULL               -- ISO-8601 UTC
            );

            CREATE INDEX IF NOT EXISTS idx_reasoning_events_session_id
                ON reasoning_events (session_id, timestamp);

            CREATE INDEX IF NOT EXISTS idx_reasoning_events_task_id
                ON reasoning_events (task_id, timestamp)
                WHERE task_id IS NOT NULL;

            CREATE INDEX IF NOT EXISTS idx_reasoning_events_timestamp
                ON reasoning_events (timestamp);
            "#,
        )
        .map_err(|e| ReasoningError::PersistenceError {
            message: format!("Schema migration failed: {e}"),
        })
    }

    /// Persists a `ReasoningEvent` to the store and returns its assigned `EventId`.
    ///
    /// The event's own `id` is used as the primary key — if the same event is
    /// stored twice (e.g. due to a retry), the second insert is silently ignored
    /// via `INSERT OR IGNORE`.
    ///
    /// The `task_id` is extracted from the event payload where present so that
    /// task-scoped queries remain efficient.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` if serialisation or the
    /// SQL insert fails.
    pub fn store(&self, event: &ReasoningEvent) -> Result<EventId, ReasoningError> {
        let payload_json =
            serde_json::to_string(event).map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to serialise event: {e}"),
            })?;

        let event_type = event_type_tag(event);
        let task_id: Option<String> = extract_task_id(event).map(|u| u.to_string());
        let timestamp_str = event.timestamp.to_rfc3339();
        let id_str = event.id.to_string();
        let session_str = event.session_id.to_string();

        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        conn.execute(
            r#"
            INSERT OR IGNORE INTO reasoning_events
                (id, session_id, task_id, event_type, payload, timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![id_str, session_str, task_id, event_type, payload_json, timestamp_str],
        )
        .map_err(|e| ReasoningError::PersistenceError {
            message: format!("Failed to insert event: {e}"),
        })?;

        debug!(event_id = %event.id, event_type, "Event persisted to store");

        Ok(event.id)
    }

    /// Returns all events for a session, ordered by `sequence_num` ascending.
    ///
    /// This is the primary path for replaying a full session history.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database or deserialisation errors.
    pub fn query_by_session(&self, session_id: Uuid) -> Result<Vec<StoredEvent>, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, payload, timestamp, rowid
                FROM reasoning_events
                WHERE session_id = ?1
                ORDER BY rowid ASC
                "#,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to prepare query: {e}"),
            })?;

        let rows = stmt
            .query_map(params![session_id.to_string()], row_to_stored_event)
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Query execution failed: {e}"),
            })?;

        collect_rows(rows)
    }

    /// Returns all events for a specific task, ordered by `sequence_num` ascending.
    ///
    /// Requires that `task_id` was present in the event payload; events without
    /// a task context (e.g. `FatalError` scoped only to a session) are excluded.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database or deserialisation errors.
    pub fn query_by_task(&self, task_id: Uuid) -> Result<Vec<StoredEvent>, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, payload, timestamp, rowid
                FROM reasoning_events
                WHERE task_id = ?1
                ORDER BY rowid ASC
                "#,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to prepare query: {e}"),
            })?;

        let rows = stmt
            .query_map(params![task_id.to_string()], row_to_stored_event)
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Query execution failed: {e}"),
            })?;

        collect_rows(rows)
    }

    /// Returns all events whose `timestamp` falls within `[from, to]` (inclusive).
    ///
    /// Timestamps are compared as ISO-8601 strings, which sort correctly when
    /// zero-padded (as `chrono` produces).
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database or deserialisation errors.
    pub fn query_range(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredEvent>, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, payload, timestamp, rowid
                FROM reasoning_events
                WHERE timestamp >= ?1 AND timestamp <= ?2
                ORDER BY rowid ASC
                "#,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to prepare range query: {e}"),
            })?;

        let rows = stmt
            .query_map(
                params![from.to_rfc3339(), to.to_rfc3339()],
                row_to_stored_event,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Range query execution failed: {e}"),
            })?;

        collect_rows(rows)
    }

    /// Returns all events for a session in strict `sequence_num` order for replay.
    ///
    /// Identical to `query_by_session` but returns an empty `Vec` on error rather
    /// than propagating it — suitable for best-effort replay where a partial result
    /// is preferable to a hard failure.
    pub fn replay(&self, session_id: Uuid) -> Vec<StoredEvent> {
        match self.query_by_session(session_id) {
            Ok(events) => events,
            Err(e) => {
                error!(%session_id, error = %e, "Replay failed; returning empty event list");
                Vec::new()
            }
        }
    }

    /// Returns the most recent `count` events for a session, ordered newest-first.
    ///
    /// Used by the frontend "recent events" panel to quickly populate the view
    /// without loading the entire history.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub fn recent_events(
        &self,
        session_id: Uuid,
        count: usize,
    ) -> Result<Vec<StoredEvent>, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, payload, timestamp, rowid
                FROM reasoning_events
                WHERE session_id = ?1
                ORDER BY rowid DESC
                LIMIT ?2
                "#,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to prepare recent-events query: {e}"),
            })?;

        let rows = stmt
            .query_map(
                params![session_id.to_string(), count as i64],
                row_to_stored_event,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("recent_events query failed: {e}"),
            })?;

        // Rows come back newest-first; reverse to chronological order.
        let mut events = collect_rows(rows)?;
        events.reverse();
        Ok(events)
    }

    /// Returns the total number of events stored for a session.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub fn event_count(&self, session_id: Uuid) -> Result<usize, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reasoning_events WHERE session_id = ?1",
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("event_count query failed: {e}"),
            })?;

        Ok(count as usize)
    }

    /// Deletes all events stored for a session.
    ///
    /// Called when a session is permanently removed. This operation is
    /// irreversible — the event store is otherwise append-only.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub fn delete_session_events(&self, session_id: Uuid) -> Result<(), ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let deleted = conn
            .execute(
                "DELETE FROM reasoning_events WHERE session_id = ?1",
                params![session_id.to_string()],
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("delete_session_events failed: {e}"),
            })?;

        info!(%session_id, deleted_rows = deleted, "Session events deleted");
        Ok(())
    }

    /// Returns all events for a session that occurred at or after `since`.
    ///
    /// Used by `ReasoningEventEmitter::replay` to support incremental replay
    /// from a known checkpoint.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub fn query_since(
        &self,
        session_id: Uuid,
        since: DateTime<Utc>,
    ) -> Result<Vec<StoredEvent>, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, payload, timestamp, rowid
                FROM reasoning_events
                WHERE session_id = ?1 AND timestamp >= ?2
                ORDER BY rowid ASC
                "#,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("Failed to prepare query_since statement: {e}"),
            })?;

        let rows = stmt
            .query_map(
                params![session_id.to_string(), since.to_rfc3339()],
                row_to_stored_event,
            )
            .map_err(|e| ReasoningError::PersistenceError {
                message: format!("query_since execution failed: {e}"),
            })?;

        collect_rows(rows)
    }

    /// Returns the most recent Mermaid diagram string stored for a task.
    ///
    /// Searches backwards through `PlanCreated` events for the task to find the
    /// latest diagram string. Returns `None` if no `PlanCreated` event has been
    /// stored for this task.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub fn latest_diagram(&self, task_id: Uuid) -> Result<Option<String>, ReasoningError> {
        let conn = self.conn.lock().map_err(|e| ReasoningError::PersistenceError {
            message: format!("Mutex poisoned: {e}"),
        })?;

        let result: rusqlite::Result<String> = conn.query_row(
            r#"
            SELECT payload FROM reasoning_events
            WHERE task_id = ?1 AND event_type = 'plan_created'
            ORDER BY rowid DESC
            LIMIT 1
            "#,
            params![task_id.to_string()],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(payload_json) => {
                let event: ReasoningEvent =
                    serde_json::from_str(&payload_json).map_err(|e| {
                        ReasoningError::PersistenceError {
                            message: format!("Failed to deserialise PlanCreated event: {e}"),
                        }
                    })?;

                let diagram = match &event.payload {
                    truenorth_core::types::event::ReasoningEventPayload::PlanCreated {
                        mermaid_diagram,
                        ..
                    } => Some(mermaid_diagram.clone()),
                    _ => None,
                };

                Ok(diagram)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(ReasoningError::PersistenceError {
                message: format!("latest_diagram query failed: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extracts the discriminant tag string from a `ReasoningEvent` for the
/// `event_type` column.  Matches the `serde` tag defined in `ReasoningEventPayload`.
fn event_type_tag(event: &ReasoningEvent) -> &'static str {
    use truenorth_core::types::event::ReasoningEventPayload::*;
    match &event.payload {
        TaskReceived { .. } => "task_received",
        PlanCreated { .. } => "plan_created",
        PlanApproved { .. } => "plan_approved",
        StepStarted { .. } => "step_started",
        StepCompleted { .. } => "step_completed",
        StepFailed { .. } => "step_failed",
        ToolCalled { .. } => "tool_called",
        ToolResult { .. } => "tool_result",
        LlmRouted { .. } => "llm_routed",
        LlmFallback { .. } => "llm_fallback",
        LlmExhausted { .. } => "llm_exhausted",
        RcsActivated { .. } => "rcs_activated",
        ReasonCompleted { .. } => "reason_completed",
        CriticCompleted { .. } => "critic_completed",
        SynthesisCompleted { .. } => "synthesis_completed",
        ContextCompacted { .. } => "context_compacted",
        MemoryWritten { .. } => "memory_written",
        MemoryConsolidated { .. } => "memory_consolidated",
        MemoryQueried { .. } => "memory_queried",
        DeviationDetected { .. } => "deviation_detected",
        ChecklistVerified { .. } => "checklist_verified",
        SessionSaved { .. } => "session_saved",
        SessionResumed { .. } => "session_resumed",
        HeartbeatFired { .. } => "heartbeat_fired",
        SkillActivated { .. } => "skill_activated",
        TaskCompleted { .. } => "task_completed",
        TaskFailed { .. } => "task_failed",
        FatalError { .. } => "fatal_error",
    }
}

/// Extracts the `task_id` from payloads that carry one, for the indexed
/// `task_id` column.  Returns `None` for session-scoped events.
fn extract_task_id(event: &ReasoningEvent) -> Option<Uuid> {
    use truenorth_core::types::event::ReasoningEventPayload::*;
    match &event.payload {
        TaskReceived { task_id, .. } => Some(*task_id),
        PlanCreated { task_id, .. } => Some(*task_id),
        StepStarted { task_id, .. } => Some(*task_id),
        StepCompleted { task_id, .. } => Some(*task_id),
        StepFailed { task_id, .. } => Some(*task_id),
        ToolCalled { step_id, .. } => Some(*step_id), // step_id is used here; no task_id
        ToolResult { step_id, .. } => Some(*step_id),
        RcsActivated { task_id, .. } => Some(*task_id),
        ReasonCompleted { task_id, .. } => Some(*task_id),
        CriticCompleted { task_id, .. } => Some(*task_id),
        SynthesisCompleted { task_id, .. } => Some(*task_id),
        DeviationDetected { task_id, .. } => Some(*task_id),
        TaskCompleted { task_id, .. } => Some(*task_id),
        TaskFailed { task_id, .. } => Some(*task_id),
        _ => None,
    }
}

/// Maps a SQLite row to a `StoredEvent`.
fn row_to_stored_event(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<StoredEvent> {
    let id_str: String = row.get(0)?;
    let session_id_str: String = row.get(1)?;
    let payload_json: String = row.get(2)?;
    let timestamp_str: String = row.get(3)?;
    let sequence_num: i64 = row.get(4)?;

    let id = Uuid::parse_str(&id_str)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;

    let session_id = Uuid::parse_str(&session_id_str)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e)))?;

    let event: ReasoningEvent = serde_json::from_str(&payload_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?;

    let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?;

    Ok(StoredEvent {
        id,
        session_id,
        event,
        timestamp,
        sequence_num,
    })
}

/// Collects a `MappedRows` iterator into a `Vec`, converting errors.
fn collect_rows(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<StoredEvent>>,
) -> Result<Vec<StoredEvent>, ReasoningError> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| ReasoningError::PersistenceError {
            message: format!("Failed to collect query rows: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};

    fn make_event(session_id: Uuid) -> ReasoningEvent {
        ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::HeartbeatFired {
                registration_id: "test-agent".to_string(),
                tick_count: 1,
                next_tick_in_secs: 60,
            },
        )
    }

    #[test]
    fn store_and_replay() {
        let store = ReasoningEventStore::open_in_memory().unwrap();
        let session_id = Uuid::new_v4();
        let event = make_event(session_id);
        let event_id = store.store(&event).unwrap();
        assert_eq!(event_id, event.id);

        let events = store.replay(session_id);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event_id);
    }

    #[test]
    fn idempotent_store() {
        let store = ReasoningEventStore::open_in_memory().unwrap();
        let session_id = Uuid::new_v4();
        let event = make_event(session_id);
        store.store(&event).unwrap();
        store.store(&event).unwrap(); // second insert is ignored
        assert_eq!(store.event_count(session_id).unwrap(), 1);
    }

    #[test]
    fn delete_session_events() {
        let store = ReasoningEventStore::open_in_memory().unwrap();
        let session_id = Uuid::new_v4();
        store.store(&make_event(session_id)).unwrap();
        store.store(&make_event(session_id)).unwrap();
        assert_eq!(store.event_count(session_id).unwrap(), 2);
        store.delete_session_events(session_id).unwrap();
        assert_eq!(store.event_count(session_id).unwrap(), 0);
    }
}
