//! Application state shared across all Axum handlers.
//!
//! [`AppState`] is cloned cheaply (all fields are `Arc<>`-wrapped) and
//! injected into every handler via Axum's `State` extractor.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use truenorth_core::types::session::SessionState;

/// Summary information about a session visible through the REST API.
///
/// A lighter-weight projection of [`SessionState`] returned when listing sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique session identifier.
    pub session_id: Uuid,
    /// Human-readable title auto-generated from the first task.
    pub title: String,
    /// Current agent state tag (e.g., "Idle", "Planning", "Executing").
    pub agent_state: String,
    /// When the session was first created.
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp.
    pub updated_at: DateTime<Utc>,
    /// Current context token usage.
    pub context_tokens: usize,
}

impl From<&SessionState> for SessionInfo {
    fn from(s: &SessionState) -> Self {
        Self {
            session_id: s.session_id,
            title: s.title.clone(),
            agent_state: s.agent_state.clone(),
            created_at: s.created_at,
            updated_at: s.snapshot_at,
            context_tokens: s.context_tokens,
        }
    }
}

/// Shared application state injected into every Axum handler.
///
/// All fields use `Arc<>` or `Clone`-able primitives so that the state can be
/// cloned cheaply for each request without copying heap allocations.
///
/// # Construction
///
/// Use [`AppState::new`] or [`AppState::builder`] to construct an instance,
/// then wrap it in `Arc` and pass it to `Router::with_state`.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Optional bearer token for API authentication.
    ///
    /// When `Some`, every request (except `/health` and `/.well-known/agent.json`)
    /// must present an `Authorization: Bearer <token>` header matching this value.
    ///
    /// When `None`, authentication is disabled (development mode).
    pub auth_token: Option<String>,

    /// Broadcast sender for visual reasoning events.
    ///
    /// WebSocket handlers subscribe to the corresponding receiver to stream
    /// events to the frontend in real time.
    pub visual_event_tx: broadcast::Sender<serde_json::Value>,

    /// Map from session UUID to its current [`SessionInfo`] summary.
    ///
    /// Protected by an async `RwLock` so that concurrent handlers can read
    /// without blocking, while write operations are serialised.
    pub active_sessions: Arc<RwLock<HashMap<Uuid, SessionInfo>>>,

    /// Name of this TrueNorth agent (surfaced in the A2A Agent Card).
    pub agent_name: String,

    /// Short description of this agent (surfaced in the A2A Agent Card).
    pub agent_description: String,

    /// Semantic version of the TrueNorth API (e.g., "1.0.0").
    pub api_version: String,
}

impl AppState {
    /// Create a new [`AppState`] with sensible defaults.
    ///
    /// The broadcast channel is created with a capacity of 1 024 events.
    /// No authentication token is set (development mode).
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            auth_token: None,
            visual_event_tx: tx,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            agent_name: "TrueNorth".to_string(),
            agent_description: "LLM-agnostic AI orchestration harness".to_string(),
            api_version: "1.0.0".to_string(),
        }
    }

    /// Create a new [`AppStateBuilder`].
    pub fn builder() -> AppStateBuilder {
        AppStateBuilder::default()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Fluent builder for [`AppState`].
#[derive(Default)]
pub struct AppStateBuilder {
    auth_token: Option<String>,
    agent_name: Option<String>,
    agent_description: Option<String>,
    api_version: Option<String>,
    channel_capacity: Option<usize>,
}

impl AppStateBuilder {
    /// Set the bearer token required for API authentication.
    ///
    /// If not called, authentication is disabled (development mode).
    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Override the agent name shown in the A2A Agent Card.
    pub fn with_agent_name(mut self, name: impl Into<String>) -> Self {
        self.agent_name = Some(name.into());
        self
    }

    /// Override the agent description shown in the A2A Agent Card.
    pub fn with_agent_description(mut self, desc: impl Into<String>) -> Self {
        self.agent_description = Some(desc.into());
        self
    }

    /// Override the API version shown in the A2A Agent Card.
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = Some(version.into());
        self
    }

    /// Set the capacity of the visual event broadcast channel.
    ///
    /// Defaults to 1 024 if not set.
    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = Some(capacity);
        self
    }

    /// Consume the builder and produce an [`AppState`].
    pub fn build(self) -> AppState {
        let capacity = self.channel_capacity.unwrap_or(1024);
        let (tx, _) = broadcast::channel(capacity);
        AppState {
            auth_token: self.auth_token,
            visual_event_tx: tx,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            agent_name: self.agent_name.unwrap_or_else(|| "TrueNorth".to_string()),
            agent_description: self
                .agent_description
                .unwrap_or_else(|| "LLM-agnostic AI orchestration harness".to_string()),
            api_version: self.api_version.unwrap_or_else(|| "1.0.0".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_no_auth_token() {
        let state = AppState::new();
        assert!(state.auth_token.is_none());
    }

    #[test]
    fn builder_sets_auth_token() {
        let state = AppState::builder()
            .with_auth_token("secret")
            .build();
        assert_eq!(state.auth_token.as_deref(), Some("secret"));
    }

    #[test]
    fn broadcast_channel_is_functional() {
        let state = AppState::new();
        let mut rx = state.visual_event_tx.subscribe();
        state.visual_event_tx.send(serde_json::json!({"hello": "world"})).unwrap();
        let msg = rx.try_recv().unwrap();
        assert_eq!(msg["hello"], "world");
    }
}
