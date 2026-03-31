/// Task types — the fundamental unit of work in TrueNorth.
///
/// Every user prompt, heartbeat firing, or sub-task decomposition becomes a `Task`.
/// Tasks carry their own execution mode, priority, and deadline, allowing the
/// orchestrator to schedule and dispatch them uniformly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A unit of work the agent loop processes.
///
/// Tasks are the fundamental scheduling primitive. A user prompt becomes
/// a Task. A heartbeat fires a Task. A sub-task decomposition produces
/// child Tasks. Every agent execution traces back to a Task with a stable UUID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Globally unique identifier for this task.
    pub id: Uuid,
    /// Parent task ID if this is a sub-task.
    pub parent_id: Option<Uuid>,
    /// Short human-readable title.
    pub title: String,
    /// Full description of what the task requires.
    pub description: String,
    /// Hard constraints the agent must not violate (e.g., "Do not modify prod database").
    pub constraints: Vec<String>,
    /// Resources that must be present in context before execution begins.
    pub context_requirements: Vec<ContextRequirement>,
    /// How the orchestrator should execute this task.
    pub execution_mode: ExecutionMode,
    /// When the task was created.
    pub created_at: DateTime<Utc>,
    /// Optional deadline after which the task is considered stale.
    pub deadline: Option<DateTime<Utc>>,
    /// Scheduling priority.
    pub priority: TaskPriority,
    /// Free-form metadata for extensions and debugging.
    pub metadata: serde_json::Value,
}

/// A requirement that must be satisfied in context before the task executes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextRequirement {
    /// A file whose contents must be in context.
    File { path: std::path::PathBuf },
    /// Memory entries that match this query must be retrieved first.
    MemoryQuery {
        query: String,
        scope: super::memory::MemoryScope,
    },
    /// A named skill that must be loaded and active.
    Skill { name: String },
}

/// How the agent should execute this task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Single-shot direct completion. No planning required.
    Direct,
    /// Execute a list of steps sequentially, one after another.
    Sequential,
    /// Execute independent steps in parallel where possible.
    Parallel,
    /// Execute a directed-acyclic graph of dependent steps.
    Graph,
    /// A time-triggered persistent agent that runs on a schedule.
    Persistent { interval_seconds: u64 },
    /// Full Reason → Critic → Synthesis loop for high-stakes decisions.
    ReasonCriticSynthesis,
}

/// Task scheduling priority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Background work, lowest urgency.
    Low = 0,
    /// Default priority for user-initiated tasks.
    Normal = 1,
    /// Time-sensitive or user-flagged as important.
    High = 2,
    /// Must be processed before all other queued tasks.
    Critical = 3,
}

/// A sub-task within a parent task graph.
///
/// Sub-tasks are created by the orchestrator during task decomposition.
/// Each sub-task has its own execution mode and may produce its own sub-tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// Unique identifier for this sub-task.
    pub id: Uuid,
    /// The parent task this sub-task belongs to.
    pub parent_task_id: Uuid,
    /// Ordered position within the parent task's decomposition.
    pub index: usize,
    /// The wrapped task to execute.
    pub task: Task,
    /// IDs of sub-tasks that must complete before this one starts.
    pub depends_on: Vec<Uuid>,
    /// Whether this sub-task has completed.
    pub completed: bool,
    /// The output summary of this sub-task (set on completion).
    pub output_summary: Option<String>,
}

/// A directed-acyclic graph of tasks with dependency tracking.
///
/// Used when `ExecutionMode::Graph` is selected. The orchestrator
/// traverses the graph in topological order, parallelizing where possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraph {
    /// The root task this graph was created from.
    pub root_task_id: Uuid,
    /// All nodes in the graph.
    pub nodes: Vec<SubTask>,
    /// Total number of nodes.
    pub total_nodes: usize,
    /// Number of nodes that have completed.
    pub completed_nodes: usize,
    /// When the graph was created.
    pub created_at: DateTime<Utc>,
}

/// The lifecycle status of a task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting to be dispatched to the agent loop.
    Queued,
    /// Currently being executed by the agent loop.
    Running,
    /// Execution paused, awaiting user input or provider recovery.
    Paused { reason: String },
    /// Successfully completed.
    Completed,
    /// Failed with an error.
    Failed { error: String },
    /// Cancelled before completion.
    Cancelled,
}

/// A numerical estimate of how complex a task is.
///
/// Computed by the orchestrator before planning. Higher scores
/// trigger R/C/S mode and longer planning phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityScore {
    /// Overall complexity (0.0 = trivial, 1.0 = extremely complex).
    pub score: f32,
    /// Estimated number of tool calls required.
    pub estimated_tool_calls: usize,
    /// Estimated token consumption.
    pub estimated_tokens: usize,
    /// Whether R/C/S mode is recommended.
    pub recommend_rcs: bool,
    /// Justification for the score.
    pub rationale: String,
}

/// The source of a task's input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputSource {
    /// Direct user message via CLI.
    UserCli,
    /// Submitted via the web interface.
    WebUi,
    /// Triggered by a scheduled heartbeat.
    Heartbeat { registration_id: String },
    /// Created by the orchestrator during task decomposition.
    TaskDecomposition { parent_task_id: Uuid },
    /// Submitted programmatically via the API.
    Api { client_id: String },
}
