/// ExecutionStrategy and AgentLoop traits — the execution contract.
///
/// ExecutionStrategy implements a specific mode of task execution (Direct,
/// Sequential, Parallel, Graph, R/C/S). The AgentLoop coordinates all
/// components: task intake → planning → execution → observation → completion.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::types::plan::{Plan, PlanStep};
use crate::types::task::Task;
use crate::types::session::SessionState;

/// The result of executing a single plan step.
///
/// Returned by `ExecutionStrategy::execute_step()` after each step completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// The step that was executed.
    pub step_id: Uuid,
    /// Sequential step number (for display).
    pub step_number: usize,
    /// Whether the step succeeded.
    pub success: bool,
    /// Structured output from this step (tool results, LLM outputs).
    pub output: serde_json::Value,
    /// Human-readable summary of what was accomplished.
    pub output_summary: String,
    /// Names of tools called during this step.
    pub tool_calls_made: Vec<String>,
    /// Total tokens consumed during this step.
    pub tokens_used: usize,
    /// Wall-clock duration in milliseconds.
    pub execution_ms: u64,
    /// Whether a deviation from the plan was detected.
    pub deviation_detected: bool,
}

/// The result of completing an entire task.
///
/// Returned by `AgentLoop::run()` after a task finishes execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// The task that was executed.
    pub task_id: Uuid,
    /// Whether execution succeeded overall.
    pub success: bool,
    /// Structured output from the final step.
    pub output: serde_json::Value,
    /// Human-readable summary of what was accomplished.
    pub output_summary: String,
    /// Number of steps that completed successfully.
    pub steps_completed: usize,
    /// Total tokens consumed across all steps.
    pub total_tokens: usize,
    /// Total wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

/// The context passed to `ExecutionStrategy::execute_step`.
///
/// Contains all dependencies the step needs without requiring direct references
/// to the full orchestrator graph (avoiding Arc<Mutex<everything>> tangles).
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// The session this execution belongs to.
    pub session_id: Uuid,
    /// The task being executed.
    pub task_id: Uuid,
    /// The current step number.
    pub step_number: usize,
    /// The approved plan (for deviation checking).
    pub approved_plan: Plan,
    /// Results from previously completed steps (for context injection).
    pub previous_results: Vec<StepResult>,
}

/// The control signal for pausing or halting agent execution.
///
/// Returned by `ExecutionStrategy::control_signal()` after each step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionControl {
    /// Continue to the next step normally.
    Continue,
    /// Pause before the next step (e.g., awaiting user approval in PAUL mode).
    Pause { reason: String },
    /// Halt execution entirely.
    Halt { reason: String, save_state: bool },
}

/// Errors from execution strategies and the agent loop.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// The task reached the maximum step limit (loop guard).
    #[error("Task '{task_id}' reached maximum step limit ({max_steps})")]
    MaxStepsExceeded { task_id: Uuid, max_steps: usize },

    /// An infinite loop was detected (same tool call pattern repeated).
    #[error("Infinite loop detected on task '{task_id}': {evidence}")]
    InfiniteLoopDetected { task_id: Uuid, evidence: String },

    /// The context budget was exhausted during execution.
    #[error("Context budget exhausted during task '{task_id}'")]
    ContextExhausted { task_id: Uuid },

    /// All LLM providers were exhausted during execution.
    #[error("All LLM providers exhausted during task '{task_id}'")]
    LlmExhausted { task_id: Uuid },

    /// A tool error that cannot be recovered from.
    #[error("Unrecoverable tool error on step {step_number}: {message}")]
    UnrecoverableToolError { step_number: usize, message: String },

    /// The agent loop was halted by an external signal.
    #[error("Agent loop was halted: {reason}")]
    Halted { reason: String },

    /// The selected execution strategy is not applicable to this task.
    #[error("Execution strategy '{strategy}' is not applicable to this task: {reason}")]
    StrategyMismatch { strategy: String, reason: String },

    /// The plan could not be created.
    #[error("Failed to create plan for task '{task_id}': {message}")]
    PlanningFailed { task_id: Uuid, message: String },
}

/// An execution strategy implements a specific mode of task execution.
///
/// The orchestrator selects the appropriate strategy based on task complexity
/// and the configured execution mode. Strategies are interchangeable algorithms
/// for accomplishing the same goal (executing a plan).
#[async_trait]
pub trait ExecutionStrategy: Send + Sync + std::fmt::Debug {
    /// Returns the name of this execution strategy.
    fn name(&self) -> &str;

    /// Determines whether this strategy is applicable to a given task.
    ///
    /// Called by the orchestrator during strategy selection. Returns true
    /// if this strategy can handle the task's execution mode and requirements.
    fn is_applicable(&self, task: &Task) -> bool;

    /// Creates an execution plan for the given task.
    ///
    /// - Direct mode: returns a single-step plan.
    /// - Sequential mode: returns an ordered list of steps.
    /// - Parallel mode: returns a list of independent steps with no ordering.
    /// - Graph mode: returns a DAG structure encoded in the plan.
    /// - R/C/S mode: returns a three-step plan (Reason, Critic, Synthesis).
    async fn plan(&self, task: &Task) -> Result<Plan, ExecutionError>;

    /// Executes a single step from the plan.
    ///
    /// Returns the step result, which includes whether a deviation was detected.
    /// The agent loop calls this repeatedly for each step in the plan.
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError>;

    /// Returns true when the strategy considers the task complete.
    ///
    /// - Sequential/Parallel: all steps completed successfully.
    /// - Graph: terminal node reached.
    /// - R/C/S: Synthesis completed without unresolved conflicts.
    fn is_complete(&self, plan: &Plan, results: &[StepResult]) -> bool;

    /// Returns the control signal the strategy recommends given the current state.
    ///
    /// Called after each step result. Strategies may request pauses (e.g., for
    /// user approval in SEED+PAUL mode) or halts (e.g., on critical deviation).
    fn control_signal(&self, plan: &Plan, results: &[StepResult]) -> ExecutionControl;
}

/// The agent loop trait: the top-level execution driver.
///
/// The agent loop coordinates all components:
/// task intake → planning → execution → observation → completion.
/// It delegates to execution strategies for step execution, to the context
/// budget manager for context tracking, to the session manager for state
/// persistence, and to the reasoning event emitter for observability.
///
/// Design rationale: the loop itself is a trait (not a concrete struct in the
/// public API) to allow testing with mock implementations and to allow future
/// variations (e.g., a headless batch loop vs. an interactive REPL loop).
#[async_trait]
pub trait AgentLoop: Send + Sync + std::fmt::Debug {
    /// Runs the complete agent loop for a task from intake to completion.
    ///
    /// The loop:
    /// 1. Receives the task and emits `TaskReceived`
    /// 2. Gathers context (memory queries, skill loading)
    /// 3. Assesses complexity and selects execution strategy
    /// 4. Creates plan (optional user approval in SEED+PAUL mode)
    /// 5. Executes steps, checking deviation and context budget after each
    /// 6. Handles R/C/S activation when needed
    /// 7. Returns final result or saves state if exhausted
    async fn run(&self, task: Task) -> Result<TaskResult, ExecutionError>;

    /// Pauses execution at the next safe checkpoint.
    ///
    /// The agent completes the current step before pausing.
    /// Returns the current session state for inspection or modification.
    async fn pause(&self) -> Result<SessionState, ExecutionError>;

    /// Resumes execution from a paused state.
    async fn resume(&self) -> Result<(), ExecutionError>;

    /// Halts execution immediately and saves state.
    ///
    /// Called by: watchdog timer, loop guard, LLM exhaustion handler, user signal.
    async fn halt(&self, reason: &str) -> Result<SessionState, ExecutionError>;

    /// Returns the current agent state as a string (for logging and UI display).
    fn current_state(&self) -> String;

    /// Returns whether the agent loop is currently running a task.
    fn is_running(&self) -> bool;
}
