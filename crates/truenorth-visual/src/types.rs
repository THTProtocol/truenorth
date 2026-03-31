/// Local types for the Visual Reasoning Layer.
///
/// These types are used to carry aggregated state, snapshots, and stored
/// event data between the event store, aggregator, and the Leptos frontend
/// server functions. They are separate from the core event types to allow
/// visual-layer-specific augmentation (e.g. sequence numbers, Mermaid strings).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use truenorth_core::types::event::ReasoningEvent;
use truenorth_core::types::memory::MemoryScope;
use truenorth_core::types::plan::PlanStepStatus;

/// A reasoning event that has been stored in the SQLite event store.
///
/// Wraps a `ReasoningEvent` with its assigned database surrogate key
/// (`sequence_num`) and the row-level UUID (`id`). Used in replay and
/// range queries where ordering and stable identity matter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    /// UUID primary key assigned when the event was stored.
    pub id: Uuid,
    /// The session this event belongs to.
    pub session_id: Uuid,
    /// The full reasoning event payload.
    pub event: ReasoningEvent,
    /// Wall-clock time at which the event was persisted.
    pub timestamp: DateTime<Utc>,
    /// Monotonically increasing sequence number within the database.
    /// Corresponds to SQLite's implicit `rowid` — guaranteed to be strictly
    /// increasing in insertion order and used to guarantee replay order even
    /// when timestamps collide (e.g. events emitted in the same millisecond).
    pub sequence_num: i64,
}

/// A point-in-time snapshot of the task dependency graph.
///
/// Computed by the `EventAggregator` from accumulated `PlanCreated`,
/// `StepStarted`, `StepCompleted`, and `StepFailed` events. Consumed by
/// the Leptos `ReasoningGraph` component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraphSnapshot {
    /// Nodes representing each plan step.
    pub nodes: Vec<TaskNode>,
    /// Directed edges representing step dependencies.
    pub edges: Vec<TaskEdge>,
    /// A pre-rendered Mermaid flowchart string for the current state.
    /// Updated incrementally as step statuses change.
    pub mermaid: String,
}

/// A single node in the task dependency graph, representing one plan step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    /// The step's unique identifier.
    pub id: Uuid,
    /// Human-readable title of the step.
    pub title: String,
    /// Full description of the expected action.
    pub description: String,
    /// Current execution status.
    pub status: PlanStepStatus,
}

/// A directed dependency edge between two plan steps.
///
/// Indicates that the `to` step cannot begin until the `from` step completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEdge {
    /// The step that must complete first.
    pub from: Uuid,
    /// The step that depends on `from`.
    pub to: Uuid,
}

/// A currently-executing plan step with elapsed timing information.
///
/// Emitted by the `EventAggregator` in response to `StepStarted` events
/// and consumed by the Leptos active-step panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveStep {
    /// The step's unique identifier.
    pub step_id: Uuid,
    /// Human-readable title of the step.
    pub title: String,
    /// Full description of what the step is doing.
    pub description: String,
    /// When this step started executing.
    pub started_at: DateTime<Utc>,
    /// Elapsed wall-clock milliseconds since the step started.
    pub duration_ms: u64,
}

/// Context window utilization at a point in time.
///
/// Derived from `LlmRouted` and `ContextCompacted` events. Consumed by
/// the Leptos `ContextBudgetGauge` component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUtilization {
    /// Total tokens used in the current context window.
    pub tokens_used: u32,
    /// Maximum tokens available for the current provider/model.
    pub tokens_max: u32,
    /// Utilization percentage (0.0–100.0).
    pub percentage: f32,
    /// Human-readable budget state label (e.g. "Green", "Yellow", "Orange", "Red", "Critical").
    pub state_label: String,
    /// When this utilization snapshot was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Default for ContextUtilization {
    fn default() -> Self {
        Self {
            tokens_used: 0,
            tokens_max: 128_000,
            percentage: 0.0,
            state_label: "Green".to_string(),
            updated_at: Utc::now(),
        }
    }
}

/// A single LLM routing decision recorded for the routing log panel.
///
/// Derived from `LlmRouted` and `LlmFallback` events. Displayed in the
/// Leptos `RoutingLog` component to show provider health and fallback history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Unique request identifier.
    pub request_id: Uuid,
    /// The provider that was ultimately used.
    pub provider: String,
    /// The model that was ultimately used.
    pub model: String,
    /// Number of fallback hops before landing on this provider (0 = first choice).
    pub fallback_number: u32,
    /// Tokens used by this call (prompt + completion).
    pub tokens_used: u32,
    /// End-to-end latency in milliseconds.
    pub latency_ms: u64,
    /// When the routing decision was made.
    pub timestamp: DateTime<Utc>,
}

/// A memory store operation recorded by the aggregator.
///
/// Derived from `MemoryWritten` and `MemoryQueried` events. Displayed in the
/// Leptos `MemoryInspector` component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryOperation {
    /// Which memory tier was accessed.
    pub scope: MemoryScope,
    /// Human-readable operation label (e.g. "write", "query", "consolidate").
    pub operation: String,
    /// Brief preview of the content or query text.
    pub content_preview: String,
    /// When the operation occurred.
    pub timestamp: DateTime<Utc>,
}
