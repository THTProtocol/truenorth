/// StateSerializer and StateMachine traits — state persistence and machine contracts.
///
/// StateSerializer handles durable snapshot persistence (SQLite + JSON).
/// StateMachine defines the agent's state transition contract, ensuring all
/// state changes are explicit, logged, and recoverable.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

use crate::types::session::SessionState;

/// Errors from state serialization and persistence.
#[derive(Debug, Error)]
pub enum StateError {
    /// The session state could not be serialized to JSON.
    #[error("Failed to serialize state for session {session_id}: {message}")]
    SerializationFailed { session_id: Uuid, message: String },

    /// The serialized state could not be written to the filesystem.
    #[error("Failed to write state to {path}: {message}")]
    WriteFailed { path: PathBuf, message: String },

    /// No snapshot was found for the given session ID.
    #[error("State snapshot not found for session {session_id}")]
    SnapshotNotFound { session_id: Uuid },

    /// The snapshot could not be deserialized (corrupt or incompatible).
    #[error("Failed to deserialize state: {message} (data may be corrupt or from incompatible version)")]
    DeserializationFailed { message: String },

    /// The snapshot's schema version is incompatible with the current version.
    #[error("State version mismatch: file version {file_version}, current version {current_version}")]
    VersionMismatch {
        file_version: String,
        current_version: String,
    },

    /// An I/O error occurred during snapshot operations.
    #[error("State I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Metadata about a saved state snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    /// The session this snapshot belongs to.
    pub session_id: Uuid,
    /// Filesystem path to the JSON snapshot file.
    pub snapshot_path: PathBuf,
    /// When this snapshot was created.
    pub created_at: DateTime<Utc>,
    /// Size of the snapshot file in bytes.
    pub file_size_bytes: u64,
    /// Schema version embedded in the snapshot.
    pub schema_version: String,
    /// A brief description of why the snapshot was taken.
    pub save_reason: Option<String>,
}

/// The state serializer trait: persists and restores complete session snapshots.
///
/// Design rationale: session state serialization must be resilient.
/// SQLite WAL mode ensures crash safety for the database.
/// The JSON snapshot provides a human-readable backup and enables
/// external tooling (e.g., diffing snapshots across runs).
/// The state schema is versioned to allow migration between TrueNorth versions.
#[async_trait]
pub trait StateSerializer: Send + Sync + std::fmt::Debug {
    /// Serializes the current session state and saves it to durable storage.
    ///
    /// Saves in two forms:
    /// 1. SQLite row in the sessions table (for fast metadata queries)
    /// 2. JSON file at ~/.truenorth/sessions/{session_id}.json (human-readable backup)
    async fn save_snapshot(&self, state: &SessionState) -> Result<SnapshotInfo, StateError>;

    /// Loads and deserializes a session snapshot.
    ///
    /// Attempts to load from SQLite first (faster). Falls back to JSON file.
    /// Runs schema migration if the snapshot's version is older than current.
    async fn load_snapshot(&self, session_id: Uuid) -> Result<SessionState, StateError>;

    /// Lists all available snapshots ordered by most recent first.
    async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, StateError>;

    /// Deletes a snapshot from both SQLite and the JSON file.
    async fn delete_snapshot(&self, session_id: Uuid) -> Result<(), StateError>;

    /// Validates a snapshot's integrity without fully deserializing it.
    ///
    /// Returns `Ok(schema_version)` if the snapshot is valid and readable.
    /// Returns `Err` if the file is missing, corrupt, or from an incompatible version.
    async fn validate_snapshot(&self, session_id: Uuid) -> Result<String, StateError>;

    /// Returns the current schema version supported by this serializer.
    fn current_schema_version(&self) -> &str;
}

/// The agent state machine states.
///
/// These variants represent the discrete states the agent loop moves through.
/// All transitions are explicit and logged as reasoning events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentState {
    /// The agent is idle, waiting for a task.
    Idle,
    /// The agent has received a task and is gathering context.
    GatheringContext { task_id: Uuid },
    /// The agent is assessing task complexity.
    AssessingComplexity { task_id: Uuid },
    /// The agent is creating an execution plan.
    Planning { task_id: Uuid },
    /// The plan has been created and is awaiting user approval.
    AwaitingApproval { task_id: Uuid, plan_id: Uuid },
    /// The agent is executing the plan.
    Executing {
        task_id: Uuid,
        plan_id: Uuid,
        current_step: usize,
    },
    /// The R/C/S loop has been activated.
    Reasoning { task_id: Uuid, phase: RcsPhase },
    /// A tool call is in progress.
    CallingTool {
        task_id: Uuid,
        step_id: Uuid,
        tool_name: String,
    },
    /// Execution has been paused.
    Paused { task_id: Uuid, reason: String },
    /// The agent is compacting context.
    CompactingContext { session_id: Uuid },
    /// The task is complete.
    Complete { task_id: Uuid },
    /// The agent has halted due to an error or exhaustion.
    Halted { reason: String, state_saved: bool },
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle => write!(f, "Idle"),
            AgentState::GatheringContext { .. } => write!(f, "GatheringContext"),
            AgentState::AssessingComplexity { .. } => write!(f, "AssessingComplexity"),
            AgentState::Planning { .. } => write!(f, "Planning"),
            AgentState::AwaitingApproval { .. } => write!(f, "AwaitingApproval"),
            AgentState::Executing { current_step, .. } => {
                write!(f, "Executing(step={})", current_step)
            }
            AgentState::Reasoning { phase, .. } => write!(f, "Reasoning({:?})", phase),
            AgentState::CallingTool { tool_name, .. } => write!(f, "CallingTool({})", tool_name),
            AgentState::Paused { reason, .. } => write!(f, "Paused({})", reason),
            AgentState::CompactingContext { .. } => write!(f, "CompactingContext"),
            AgentState::Complete { .. } => write!(f, "Complete"),
            AgentState::Halted { reason, .. } => write!(f, "Halted({})", reason),
        }
    }
}

/// The phase of an active R/C/S loop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RcsPhase {
    /// The Reason phase: the primary agent analyzes the problem.
    Reason,
    /// The Critic phase: a separate LLM context critiques the reasoning.
    Critic,
    /// The Synthesis phase: conflicting perspectives are resolved.
    Synthesis,
}

/// The state machine trait for the TrueNorth agent.
///
/// Ensures all state transitions are explicit, validated, and emitted
/// as reasoning events. No component is allowed to modify agent state
/// directly — all state changes go through this interface.
pub trait StateMachine: Send + Sync + std::fmt::Debug {
    /// Returns the current agent state.
    fn current_state(&self) -> &AgentState;

    /// Attempts to transition to a new state.
    ///
    /// Returns `Ok(new_state)` if the transition is valid.
    /// Returns `Err(InvalidTransition)` if the transition is not permitted
    /// from the current state (e.g., cannot go from Idle to Executing).
    fn transition(&self, new_state: AgentState) -> Result<&AgentState, StateTransitionError>;

    /// Returns all valid next states from the current state.
    ///
    /// Used by the agent loop to validate control flow and by the
    /// Visual Reasoning Layer to display the state machine diagram.
    fn valid_transitions(&self) -> Vec<AgentState>;

    /// Returns whether the given state transition is permitted.
    fn can_transition_to(&self, target: &AgentState) -> bool;

    /// Returns the history of state transitions this session.
    fn transition_history(&self) -> &[(AgentState, DateTime<Utc>)];
}

/// Error from an invalid state machine transition.
#[derive(Debug, Error)]
pub enum StateTransitionError {
    /// The requested transition is not valid from the current state.
    #[error("Cannot transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
}
