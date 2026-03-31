/// Event types — the visual reasoning event system.
///
/// Every observable action in TrueNorth emits a `ReasoningEvent`.
/// Events are the source of truth for the Visual Reasoning Layer.
/// They are stored append-only in SQLite and streamed via WebSocket
/// to connected Leptos frontends. The frontend reconstructs the full
/// reasoning graph by replaying events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::llm::TokenUsage;
use super::memory::MemoryScope;

/// A unique identifier for a reasoning event.
pub type EventId = Uuid;

/// An observed event in the TrueNorth reasoning system.
///
/// Every event is timestamped, scoped to a session, and carries a typed payload.
/// The event store is append-only — events are never modified or deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningEvent {
    /// Unique event ID.
    pub id: EventId,
    /// The session this event belongs to.
    pub session_id: Uuid,
    /// When this event occurred.
    pub timestamp: DateTime<Utc>,
    /// The event payload.
    pub payload: ReasoningEventPayload,
}

impl ReasoningEvent {
    /// Creates a new reasoning event with a generated ID and current timestamp.
    pub fn new(session_id: Uuid, payload: ReasoningEventPayload) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            timestamp: Utc::now(),
            payload,
        }
    }
}

/// The typed payload of a reasoning event.
///
/// Each variant corresponds to a distinct observable moment in the agent loop.
/// All variants are serialized with a `type` tag for JSON deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningEventPayload {
    /// A new task has been received from the user or a trigger source.
    TaskReceived {
        task_id: Uuid,
        title: String,
        description: String,
        execution_mode: String,
        input_source: String,
    },

    /// The agent has created an execution plan.
    /// Includes the Mermaid diagram string for immediate frontend rendering.
    PlanCreated {
        task_id: Uuid,
        plan_id: Uuid,
        step_count: usize,
        mermaid_diagram: String,
        estimated_tokens: u32,
        estimated_duration_secs: u64,
    },

    /// The user approved a plan (PAUL mode).
    PlanApproved {
        plan_id: Uuid,
        approved_at: DateTime<Utc>,
    },

    /// A plan step has begun execution.
    StepStarted {
        task_id: Uuid,
        plan_id: Uuid,
        step_id: Uuid,
        step_number: usize,
        title: String,
        description: String,
    },

    /// A plan step completed successfully.
    StepCompleted {
        task_id: Uuid,
        step_id: Uuid,
        step_number: usize,
        output_summary: String,
        duration_ms: u64,
    },

    /// A plan step failed.
    StepFailed {
        task_id: Uuid,
        step_id: Uuid,
        step_number: usize,
        error: String,
        duration_ms: u64,
        will_retry: bool,
    },

    /// A tool call has been initiated.
    ToolCalled {
        step_id: Uuid,
        call_id: String,
        tool_name: String,
        input_summary: String,
        permission_level: String,
    },

    /// A tool call has completed (success or failure).
    ToolResult {
        step_id: Uuid,
        call_id: String,
        tool_name: String,
        success: bool,
        result_summary: String,
        duration_ms: u64,
    },

    /// An LLM call completed. Includes routing and token usage.
    LlmRouted {
        request_id: Uuid,
        provider: String,
        model: String,
        usage: TokenUsage,
        latency_ms: u64,
        fallback_number: u32,
    },

    /// The router fell back to an alternative provider.
    LlmFallback {
        request_id: Uuid,
        failed_provider: String,
        next_provider: String,
        reason: String,
    },

    /// All LLM providers have been exhausted.
    LlmExhausted {
        session_id: Uuid,
        loops_attempted: u32,
        providers_tried: Vec<String>,
    },

    /// The R/C/S (Reason/Critic/Synthesis) loop was activated.
    RcsActivated {
        task_id: Uuid,
        reason: String,
        complexity_score: f32,
    },

    /// The Reason phase of an R/C/S loop completed.
    ReasonCompleted {
        task_id: Uuid,
        summary: String,
        token_count: u32,
    },

    /// The Critic phase of an R/C/S loop completed.
    CriticCompleted {
        task_id: Uuid,
        approved: bool,
        issues: Vec<String>,
        token_count: u32,
    },

    /// The Synthesis phase of an R/C/S loop completed.
    SynthesisCompleted {
        task_id: Uuid,
        final_decision: String,
        resolved_conflicts: Vec<String>,
        token_count: u32,
    },

    /// Context compaction was triggered and completed.
    ContextCompacted {
        session_id: Uuid,
        before_tokens: u32,
        after_tokens: u32,
        messages_removed: u32,
        compaction_ratio: f32,
        trigger: String,
    },

    /// A memory entry was written to the store.
    MemoryWritten {
        entry_id: Uuid,
        scope: MemoryScope,
        content_preview: String,
        was_duplicate: bool,
    },

    /// A memory consolidation cycle completed.
    MemoryConsolidated {
        scope: MemoryScope,
        entries_reviewed: u32,
        entries_merged: u32,
        entries_pruned: u32,
        duration_ms: u64,
    },

    /// A memory query was executed.
    MemoryQueried {
        session_id: Uuid,
        query_preview: String,
        scope: MemoryScope,
        results_count: usize,
        search_type: String,
    },

    /// A deviation from the approved plan was detected.
    DeviationDetected {
        task_id: Uuid,
        plan_id: Uuid,
        step_id: Uuid,
        expected_summary: String,
        actual_summary: String,
        severity: DeviationSeverity,
    },

    /// A negative checklist checkpoint was verified.
    ChecklistVerified {
        session_id: Uuid,
        checkpoint: String,
        total_items: u32,
        passed: u32,
        failed: u32,
        items: Vec<ChecklistResultItem>,
    },

    /// Session state was saved to durable storage.
    SessionSaved {
        session_id: Uuid,
        reason: String,
        snapshot_path: String,
        context_tokens: usize,
    },

    /// A session was resumed from a saved snapshot.
    SessionResumed {
        session_id: Uuid,
        resumed_from_snapshot: Uuid,
        resumed_at_step: usize,
    },

    /// A heartbeat tick fired for a persistent agent.
    HeartbeatFired {
        registration_id: String,
        tick_count: u64,
        next_tick_in_secs: u64,
    },

    /// A skill was loaded or triggered.
    SkillActivated {
        session_id: Uuid,
        skill_name: String,
        skill_version: String,
        load_level: String,
        trigger_phrase: Option<String>,
    },

    /// The overall task completed successfully.
    TaskCompleted {
        task_id: Uuid,
        session_id: Uuid,
        output_summary: String,
        total_steps: usize,
        total_tokens: u32,
        duration_ms: u64,
    },

    /// The task failed and could not recover.
    TaskFailed {
        task_id: Uuid,
        session_id: Uuid,
        error: String,
        duration_ms: u64,
        state_saved: bool,
    },

    /// An unrecoverable fatal error halted the system.
    FatalError {
        session_id: Uuid,
        error: String,
        state_saved: bool,
    },
}

/// Severity of a detected plan deviation.
///
/// Used by the DeviationTracker to classify deviations and determine
/// the appropriate response (log and continue vs. halt and flag).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeviationSeverity {
    /// Minor deviation: different approach to the same goal. Log and continue.
    Minor,
    /// Significant deviation: goal drift detected. Alert and ask for confirmation.
    Significant,
    /// Critical deviation: contradicts the approved plan. Halt and require user input.
    Critical,
}

/// A single checklist item result in a checkpoint verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistResultItem {
    /// The unique ID of the checklist item.
    pub item_id: String,
    /// Human-readable description of the check.
    pub description: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Optional detail about why the check failed (if applicable).
    pub detail: Option<String>,
}
