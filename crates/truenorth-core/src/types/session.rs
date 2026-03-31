/// Session types — the persistence unit of TrueNorth agent state.
///
/// Sessions carry the full in-flight state of an agent loop, including
/// conversation history, active plan, and provider routing state.
/// They serialize to JSON for save/resume functionality.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A globally unique session identifier.
pub type SessionId = Uuid;

/// The complete, serializable state of an agent session.
///
/// This struct is the unit of persistence. It is saved to SQLite + JSON
/// whenever the agent loop is interrupted, context is exhausted, or the
/// user explicitly requests a save. On resume, this struct is loaded and
/// the agent loop continues from the exact point it paused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Globally unique session identifier.
    pub session_id: SessionId,
    /// Human-readable session title (auto-generated from the first task).
    pub title: String,
    /// When this session was first created.
    pub created_at: DateTime<Utc>,
    /// When this state snapshot was taken.
    pub snapshot_at: DateTime<Utc>,
    /// Current agent state tag (e.g., "Idle", "Planning", "Executing").
    pub agent_state: String,
    /// The active task being executed (serialized to JSON for flexibility).
    pub current_task: Option<serde_json::Value>,
    /// Full conversation history as raw JSON (provider-normalized).
    pub conversation_history: Vec<serde_json::Value>,
    /// The active execution plan (serialized to JSON).
    pub active_plan: Option<serde_json::Value>,
    /// Current context window token count.
    pub context_tokens: usize,
    /// Context budget configured for this session.
    pub context_budget: usize,
    /// LLM routing state at the time of snapshot.
    pub routing_state: LlmRoutingState,
    /// All reasoning events emitted this session (for replay on resume).
    pub reasoning_events: Vec<serde_json::Value>,
    /// Why this session was saved (for display in `truenorth resume` listing).
    pub save_reason: Option<String>,
    /// Schema version for migration compatibility checks.
    pub schema_version: String,
}

/// The LLM provider availability state at the time of a session snapshot.
///
/// Persisted so that on resume, the router can immediately know which
/// providers are exhausted or rate-limited without re-testing them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRoutingState {
    /// The name of the primary (preferred) provider.
    pub primary_provider: String,
    /// Providers permanently marked as exhausted this session.
    pub exhausted_providers: Vec<String>,
    /// Providers currently in a rate-limited cooldown period.
    pub rate_limited_providers: Vec<RateLimitedProvider>,
}

/// A provider that is currently rate-limited, with expiry information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitedProvider {
    /// Provider name.
    pub name: String,
    /// When the rate limit expires and the provider becomes usable again.
    pub expires_at: DateTime<Utc>,
}

/// A handoff document: the compact state-transfer document for context continuation.
///
/// Created when context approaches exhaustion (90% threshold). The receiving
/// agent reads this document as its initial system context and continues the
/// task without the full conversation history, preserving task continuity
/// while staying within context budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffDocument {
    /// The session ID this handoff continues from.
    pub from_session_id: SessionId,
    /// A new session ID for the continuation session.
    pub to_session_id: SessionId,
    /// When this handoff document was created.
    pub created_at: DateTime<Utc>,
    /// Concise statement of the overall goal (1-3 sentences).
    pub objective: String,
    /// What has been completed so far (bullet-point list).
    pub completed_steps: Vec<String>,
    /// What still needs to be done (bullet-point list).
    pub remaining_steps: Vec<String>,
    /// Key decisions, findings, or facts that must be preserved for continuation.
    pub critical_context: Vec<String>,
    /// The original approved plan (for deviation tracking in the new session).
    pub original_plan: Option<serde_json::Value>,
    /// Memory entry IDs whose contents are relevant to continuation.
    pub memory_references: Vec<Uuid>,
    /// Files written or modified this session (for audit trail continuity).
    pub modified_files: Vec<String>,
    /// The agent state to resume from.
    pub resume_from_state: String,
}

/// A complete session snapshot with all associated data.
///
/// Wraps `SessionState` with additional filesystem metadata about the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// The session state data.
    pub state: SessionState,
    /// Path where the JSON snapshot file is stored.
    pub snapshot_path: String,
    /// Size of the snapshot file in bytes.
    pub snapshot_size_bytes: u64,
    /// SHA-256 checksum of the snapshot for integrity verification.
    pub checksum: String,
}
