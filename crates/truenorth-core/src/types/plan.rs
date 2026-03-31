/// Plan types — the structured contract between planning and execution phases.
///
/// Plans are created by the SEED phase before execution begins (in PAUL mode,
/// they require user approval). The `DeviationTracker` continuously compares
/// execution against the plan. Plans are the formal representation of what
/// the agent promised to do.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A structured execution plan produced by the planning phase.
///
/// Plans are created by the SEED phase (plan before you build) and optionally
/// approved by the user before execution begins. The plan is the contract
/// between what the agent promised to do and what it actually does. The
/// `DeviationTracker` compares execution against this plan continuously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Unique identifier for this plan.
    pub id: Uuid,
    /// The task this plan was created for.
    pub task_id: Uuid,
    /// When the plan was created.
    pub created_at: DateTime<Utc>,
    /// When the user approved the plan (None if not in PAUL mode or not yet approved).
    pub approved_at: Option<DateTime<Utc>>,
    /// Ordered list of steps in this plan.
    pub steps: Vec<PlanStep>,
    /// Estimated total token consumption for executing this plan.
    pub estimated_tokens: u32,
    /// Estimated wall-clock duration in seconds.
    pub estimated_duration_seconds: u64,
    /// A Mermaid flowchart diagram string for visual display.
    pub mermaid_diagram: String,
    /// The current status of this plan.
    pub status: PlanStatus,
    /// Free-form metadata.
    pub metadata: serde_json::Value,
}

/// A single step within a plan.
///
/// Each step maps to a distinct action the agent will take — a tool call,
/// an LLM generation, or a sub-task decomposition. Steps can depend on
/// previous steps via `depends_on`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Unique identifier for this step.
    pub id: Uuid,
    /// Ordered position in the plan (1-based for human display).
    pub step_number: usize,
    /// Short title describing what this step does.
    pub title: String,
    /// Full description of the expected action and rationale.
    pub description: String,
    /// Tool names expected to be called during this step.
    pub tools_expected: Vec<String>,
    /// Skill names expected to be active during this step.
    pub skills_expected: Vec<String>,
    /// IDs of steps that must complete before this step can start.
    pub depends_on: Vec<Uuid>,
    /// Estimated token consumption for this step.
    pub estimated_tokens: u32,
    /// Current execution status of this step.
    pub status: PlanStepStatus,
    /// When execution of this step started.
    pub started_at: Option<DateTime<Utc>>,
    /// When this step completed (success or failure).
    pub completed_at: Option<DateTime<Utc>>,
    /// The actual output produced by this step (set on completion).
    pub actual_output: Option<serde_json::Value>,
}

/// The lifecycle status of an overall plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStatus {
    /// Created but awaiting user approval (PAUL mode).
    PendingApproval,
    /// Approved and ready to execute.
    Approved,
    /// Currently being executed.
    Executing,
    /// All steps completed successfully.
    Completed,
    /// One or more steps failed and execution halted.
    Failed { reason: String },
    /// Cancelled before completion.
    Cancelled,
}

/// The lifecycle status of a single plan step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStepStatus {
    /// Waiting for dependency steps to complete.
    Pending,
    /// Currently being executed.
    InProgress,
    /// Completed successfully.
    Completed,
    /// Failed with an error message.
    Failed { error: String },
    /// Deliberately skipped (e.g., dependency failed, step became unnecessary).
    Skipped { reason: String },
}

/// The result of executing a complete plan.
///
/// Returned by the agent loop after a plan finishes execution.
/// Contains both the structured output and a human-readable summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The plan that was executed.
    pub plan_id: Uuid,
    /// The task the plan was created for.
    pub task_id: Uuid,
    /// Whether execution succeeded overall.
    pub success: bool,
    /// Structured output from the final step.
    pub output: serde_json::Value,
    /// Human-readable summary of what was accomplished.
    pub output_summary: String,
    /// Number of steps that completed successfully.
    pub steps_completed: usize,
    /// Number of steps that failed.
    pub steps_failed: usize,
    /// Total tokens consumed across all steps.
    pub total_tokens: u32,
    /// Total wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// When execution completed.
    pub completed_at: DateTime<Utc>,
}

/// The execution mode to use for a task or plan step.
///
/// Mirrors `task::ExecutionMode` but at the plan level, where individual
/// steps may use different modes than the overall task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Single-shot direct completion without planning.
    Direct,
    /// Execute steps sequentially in order.
    Sequential,
    /// Execute independent steps in parallel.
    Parallel,
    /// Execute a dependency graph of steps in topological order.
    Graph,
    /// Scheduled recurring execution.
    Persistent { interval_seconds: u64 },
    /// Full Reason → Critic → Synthesis loop.
    ReasonCriticSynthesis,
}
