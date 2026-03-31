//! Session manager implementation.
//!
//! Implements `SessionManager` from `truenorth-core::traits::session`.
//! Handles the full session lifecycle: create, save, resume, list, delete.
//! Bridges in-memory agent state with durable SQLite storage.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use truenorth_core::traits::session::{SessionError, SessionManager, SessionSummary};
use truenorth_core::traits::state::StateSerializer;
use truenorth_core::types::session::{
    HandoffDocument, LlmRoutingState, SessionId, SessionState,
};

use crate::session::handoff::HandoffGenerator;
use crate::session::serializer::SqliteStateSerializer;

/// Default session manager.
///
/// Wraps the `SqliteStateSerializer` for persistence and adds
/// session lifecycle logic (create, update, handoff generation).
#[derive(Debug, Clone)]
pub struct DefaultSessionManager {
    serializer: Arc<SqliteStateSerializer>,
}

impl DefaultSessionManager {
    /// Creates a new session manager with the given serializer.
    pub fn new(serializer: Arc<SqliteStateSerializer>) -> Self {
        Self { serializer }
    }

    /// Creates an empty initial session state.
    fn new_session_state(session_id: Uuid, title: Option<String>, context_budget: usize) -> SessionState {
        SessionState {
            session_id,
            title: title.unwrap_or_else(|| format!("Session {}", &session_id.to_string()[..8])),
            created_at: Utc::now(),
            snapshot_at: Utc::now(),
            agent_state: "Idle".to_string(),
            current_task: None,
            conversation_history: vec![],
            active_plan: None,
            context_tokens: 0,
            context_budget,
            routing_state: LlmRoutingState {
                primary_provider: "default".to_string(),
                exhausted_providers: vec![],
                rate_limited_providers: vec![],
            },
            reasoning_events: vec![],
            save_reason: None,
            schema_version: "1.0".to_string(),
        }
    }
}

#[async_trait]
impl SessionManager for DefaultSessionManager {
    /// Creates a new session with a generated UUID.
    #[instrument(skip(self))]
    async fn create(
        &self,
        title: Option<String>,
        context_budget: usize,
    ) -> Result<SessionState, SessionError> {
        let session_id = Uuid::new_v4();
        let state = Self::new_session_state(session_id, title, context_budget);

        self.serializer.save_snapshot(&state).await
            .map_err(|e| SessionError::StorageError { message: e.to_string() })?;

        info!("Created session {}", session_id);
        Ok(state)
    }

    /// Resumes a previously saved session.
    ///
    /// Loads from SQLite, re-queries memory for changes since snapshot,
    /// and returns the restored state.
    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn resume(&self, session_id: SessionId) -> Result<SessionState, SessionError> {
        let mut state = self.serializer.load_snapshot(session_id).await
            .map_err(|e| SessionError::RestorationError {
                id: session_id,
                message: e.to_string(),
            })?;

        // Update the snapshot time on resume
        state.snapshot_at = Utc::now();

        info!("Resumed session {} (state: {})", session_id, state.agent_state);
        Ok(state)
    }

    /// Saves the current session state to durable storage.
    #[instrument(skip(self, state), fields(session_id = %state.session_id))]
    async fn save(
        &self,
        state: &SessionState,
        reason: Option<String>,
    ) -> Result<(), SessionError> {
        let mut state = state.clone();
        state.snapshot_at = Utc::now();
        if reason.is_some() {
            state.save_reason = reason;
        }

        self.serializer.save_snapshot(&state).await
            .map_err(|e| SessionError::StorageError { message: e.to_string() })?;

        debug!("Saved session {}", state.session_id);
        Ok(())
    }

    /// Creates a handoff document from the current session state.
    async fn create_handoff(
        &self,
        state: &SessionState,
    ) -> Result<HandoffDocument, SessionError> {
        Ok(HandoffGenerator::generate(state))
    }

    /// Lists all saved sessions with summary information.
    async fn list_sessions(&self) -> Result<Vec<SessionSummary>, SessionError> {
        let snapshots = self.serializer.list_snapshots().await
            .map_err(|e| SessionError::StorageError { message: e.to_string() })?;

        let summaries = snapshots.into_iter().map(|info| SessionSummary {
            session_id: info.session_id,
            title: format!("Session {}", &info.session_id.to_string()[..8]),
            created_at: info.created_at,
            saved_at: info.created_at,
            agent_state: "Unknown".to_string(),
            save_reason: info.save_reason,
            context_tokens: 0,
            event_count: 0,
        }).collect();

        Ok(summaries)
    }

    /// Deletes a session and all associated data.
    async fn delete_session(&self, session_id: SessionId) -> Result<(), SessionError> {
        self.serializer.delete_snapshot(session_id).await
            .map_err(|e| SessionError::StorageError { message: e.to_string() })?;
        info!("Deleted session {}", session_id);
        Ok(())
    }

    /// Returns the session summary for a specific session ID.
    async fn get_summary(
        &self,
        session_id: SessionId,
    ) -> Result<SessionSummary, SessionError> {
        let snapshots = self.serializer.list_snapshots().await
            .map_err(|e| SessionError::StorageError { message: e.to_string() })?;

        snapshots.into_iter()
            .find(|s| s.session_id == session_id)
            .map(|info| SessionSummary {
                session_id: info.session_id,
                title: format!("Session {}", &info.session_id.to_string()[..8]),
                created_at: info.created_at,
                saved_at: info.created_at,
                agent_state: "Unknown".to_string(),
                save_reason: info.save_reason,
                context_tokens: 0,
                event_count: 0,
            })
            .ok_or(SessionError::NotFound { id: session_id })
    }

    /// Updates the current session state in-place.
    async fn update(
        &self,
        _session_id: SessionId,
        state: &SessionState,
    ) -> Result<(), SessionError> {
        self.save(state, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::serializer::SqliteStateSerializer;

    fn make_manager() -> DefaultSessionManager {
        let serializer = Arc::new(SqliteStateSerializer::new(":memory:").unwrap());
        DefaultSessionManager::new(serializer)
    }

    #[tokio::test]
    async fn create_returns_valid_session() {
        let mgr = make_manager();
        let state = mgr.create(Some("Test".to_string()), 100_000).await.unwrap();
        assert_eq!(state.title, "Test");
        assert_eq!(state.context_budget, 100_000);
        assert_eq!(state.agent_state, "Idle");
    }

    #[tokio::test]
    async fn save_and_resume_roundtrip() {
        let mgr = make_manager();
        let state = mgr.create(None, 50_000).await.unwrap();
        let session_id = state.session_id;

        let mut updated = state.clone();
        updated.agent_state = "Executing".to_string();
        mgr.save(&updated, Some("checkpoint".to_string())).await.unwrap();

        let resumed = mgr.resume(session_id).await.unwrap();
        assert_eq!(resumed.agent_state, "Executing");
    }

    #[tokio::test]
    async fn list_sessions_returns_created() {
        let mgr = make_manager();
        mgr.create(None, 1000).await.unwrap();
        mgr.create(None, 1000).await.unwrap();
        let sessions = mgr.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn delete_session_removes_it() {
        let mgr = make_manager();
        let state = mgr.create(None, 1000).await.unwrap();
        let id = state.session_id;
        mgr.delete_session(id).await.unwrap();
        let result = mgr.resume(id).await;
        assert!(result.is_err());
    }
}
