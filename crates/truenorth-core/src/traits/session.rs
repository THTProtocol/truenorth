/// SessionManager trait — session lifecycle contract.
///
/// Sessions are the unit of work persistence. The session manager bridges
/// the in-memory agent state and durable SQLite storage. It handles creating,
/// saving, resuming, and listing sessions, and creating handoff documents for
/// context continuation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::types::session::{HandoffDocument, SessionId, SessionState};

/// A lightweight session summary for the `truenorth resume` listing.
///
/// Contains just enough information to identify and describe a session
/// without loading the full state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Unique session identifier.
    pub session_id: SessionId,
    /// Human-readable title.
    pub title: String,
    /// When the session was first created.
    pub created_at: DateTime<Utc>,
    /// When the session was last saved.
    pub saved_at: DateTime<Utc>,
    /// The agent state at the time of save (e.g., "Executing", "Idle").
    pub agent_state: String,
    /// Why the session was saved (for display in the resume listing).
    pub save_reason: Option<String>,
    /// Token count at the time of save.
    pub context_tokens: usize,
    /// Number of reasoning events recorded this session.
    pub event_count: usize,
}

/// Errors from session management.
#[derive(Debug, Error)]
pub enum SessionError {
    /// No session with this ID exists.
    #[error("Session {id} not found")]
    NotFound { id: Uuid },

    /// The session state could not be serialized.
    #[error("Failed to serialize session state: {message}")]
    SerializationError { message: String },

    /// The session state could not be saved to storage.
    #[error("Failed to save session to storage: {message}")]
    StorageError { message: String },

    /// The session state could not be restored from storage.
    #[error("Failed to restore session {id}: {message}")]
    RestorationError { id: Uuid, message: String },

    /// The session is already active and cannot be resumed.
    #[error("Session {id} is already active")]
    AlreadyActive { id: Uuid },

    /// The session state is from an incompatible schema version.
    #[error("Session {id} schema version mismatch: file={file_version}, current={current_version}")]
    VersionMismatch {
        id: Uuid,
        file_version: String,
        current_version: String,
    },
}

/// The session manager trait: handles the lifecycle of agent sessions.
///
/// Design rationale: sessions are the unit of work persistence.
/// The session manager is the bridge between the in-memory agent state
/// and the durable SQLite storage. It is also responsible for creating
/// handoff documents — the mechanism by which context exhaustion
/// is handled gracefully without losing task progress.
#[async_trait]
pub trait SessionManager: Send + Sync + std::fmt::Debug {
    /// Creates a new session with a generated UUID.
    ///
    /// Initializes the context budget and stores the initial state in SQLite.
    /// Returns the complete initial session state.
    async fn create(
        &self,
        title: Option<String>,
        context_budget: usize,
    ) -> Result<SessionState, SessionError>;

    /// Resumes a previously saved session.
    ///
    /// Resume protocol:
    /// 1. Load session snapshot from SQLite
    /// 2. Re-query memory for changes since snapshot
    /// 3. Re-check LLM provider availability
    /// 4. Emit `ReasoningEvent::SessionResumed`
    /// 5. Return restored state for the agent loop to continue
    async fn resume(&self, session_id: SessionId) -> Result<SessionState, SessionError>;

    /// Saves the current session state to durable storage.
    ///
    /// Called: on LLM exhaustion, on user request, at context compaction,
    /// on session end, and on context budget triggers.
    async fn save(
        &self,
        state: &SessionState,
        reason: Option<String>,
    ) -> Result<(), SessionError>;

    /// Creates a handoff document from the current session state.
    ///
    /// The handoff document is the compact context transfer mechanism.
    /// It preserves task continuity while shedding irrelevant history.
    /// The new session loads this document as its initial system context.
    async fn create_handoff(
        &self,
        state: &SessionState,
    ) -> Result<HandoffDocument, SessionError>;

    /// Lists all saved sessions with summary information.
    ///
    /// Used by `truenorth resume` to show the user which sessions can be resumed.
    async fn list_sessions(&self) -> Result<Vec<SessionSummary>, SessionError>;

    /// Deletes a session and all its associated data.
    ///
    /// Removes both the SQLite row and the JSON snapshot file.
    async fn delete_session(&self, session_id: SessionId) -> Result<(), SessionError>;

    /// Returns the session summary for a specific session ID.
    async fn get_summary(
        &self,
        session_id: SessionId,
    ) -> Result<SessionSummary, SessionError>;

    /// Updates the current session state in-place (for mid-session saves).
    ///
    /// More efficient than `save()` for frequent checkpoint saves — only updates
    /// changed fields rather than rewriting the full snapshot.
    async fn update(
        &self,
        session_id: SessionId,
        state: &SessionState,
    ) -> Result<(), SessionError>;
}
