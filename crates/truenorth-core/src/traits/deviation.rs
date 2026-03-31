/// DeviationTracker trait — plan execution fidelity monitoring.
///
/// The deviation tracker compares each step's actual action against the
/// approved plan using semantic similarity. When the similarity drops below
/// a threshold, a deviation is flagged. This is the structural implementation
/// of SEED+PAUL's deviation documentation requirement.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::types::plan::{Plan, PlanStep};
use crate::types::event::DeviationSeverity;
use crate::traits::execution::StepResult;

/// A detected deviation from the approved plan.
///
/// Records both what was expected (from the plan) and what actually happened
/// (from the step result), along with the severity and resolution status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deviation {
    /// Unique identifier for this deviation record.
    pub id: Uuid,
    /// The task this deviation occurred in.
    pub task_id: Uuid,
    /// The plan step that was deviated from.
    pub plan_step: PlanStep,
    /// A description of what the agent actually did.
    pub actual_action: String,
    /// The semantic similarity score between planned and actual (0.0–1.0).
    pub similarity_score: f32,
    /// The severity of this deviation.
    pub severity: DeviationSeverity,
    /// When this deviation was detected.
    pub detected_at: DateTime<Utc>,
    /// Whether this deviation was automatically resolved (minor deviations only).
    pub auto_resolved: bool,
    /// The resolution description (if resolved).
    pub resolution: Option<String>,
    /// When the deviation was resolved (if resolved).
    pub resolved_at: Option<DateTime<Utc>>,
}

/// The alert sent to the orchestrator when a deviation is detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationAlert {
    /// The detected deviation.
    pub deviation: Deviation,
    /// The recommended action for the orchestrator.
    pub recommended_action: DeviationAction,
}

/// What the orchestrator should do when a deviation is detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeviationAction {
    /// Minor deviation: log and continue without interruption.
    ContinueWithLog,
    /// The plan step description should be updated to reflect the actual action.
    UpdatePlan { new_step_description: String },
    /// Pause and ask the user whether to continue with the deviation.
    RequireUserApproval { message: String },
    /// The deviation is severe enough to halt execution and flag for review.
    HaltAndFlag { reason: String },
}

/// Errors from the deviation tracker.
#[derive(Debug, Error)]
pub enum DeviationError {
    /// No plan has been registered for this task.
    #[error("No plan registered for task {task_id}")]
    NoPlanRegistered { task_id: Uuid },

    /// The deviation analysis could not complete.
    #[error("Deviation analysis failed: {message}")]
    AnalysisFailed { message: String },

    /// The deviation record was not found.
    #[error("Deviation {deviation_id} not found")]
    DeviationNotFound { deviation_id: Uuid },

    /// Embedding-based similarity computation failed.
    #[error("Semantic similarity computation failed: {message}")]
    SimilarityFailed { message: String },
}

/// The deviation tracker: monitors execution fidelity against the approved plan.
///
/// Design rationale: planning deviation (Article 3 failure mode) occurs when
/// the agent subtly drifts from the approved plan without acknowledgment.
/// The deviation tracker compares each step's actual action (as described in
/// the StepResult) against the planned step description using semantic similarity.
/// When similarity drops below a threshold, a deviation is flagged.
///
/// This is the structural implementation of SEED+PAUL's deviation documentation
/// requirement: every deviation from the plan is recorded, evaluated, and either
/// auto-resolved (minor) or escalated (critical).
#[async_trait]
pub trait DeviationTracker: Send + Sync + std::fmt::Debug {
    /// Registers the approved plan for a task.
    ///
    /// Must be called before any `check_step()` calls for the task.
    /// The plan provides the baseline against which deviations are measured.
    async fn register_plan(
        &self,
        task_id: Uuid,
        plan: Plan,
    ) -> Result<(), DeviationError>;

    /// Checks whether a completed step deviated from the plan.
    ///
    /// Compares the step's `output_summary` against the planned step description
    /// using semantic similarity (embedding cosine similarity).
    /// If similarity < deviation_threshold (configurable, default 0.75),
    /// a deviation is detected and a `DeviationAlert` is returned.
    ///
    /// Also emits a `ReasoningEvent::DeviationDetected` if a deviation is found.
    async fn check_step(
        &self,
        task_id: Uuid,
        step_number: usize,
        result: &StepResult,
    ) -> Result<Option<DeviationAlert>, DeviationError>;

    /// Returns all deviations recorded for a task.
    async fn task_deviations(
        &self,
        task_id: Uuid,
    ) -> Result<Vec<Deviation>, DeviationError>;

    /// Marks a deviation as resolved with an explanation.
    ///
    /// Called when the user approves a deviating action or when auto-resolution
    /// occurs for minor deviations.
    async fn resolve_deviation(
        &self,
        deviation_id: Uuid,
        resolution: String,
    ) -> Result<(), DeviationError>;

    /// Updates the plan to reflect an approved deviation.
    ///
    /// Used when the user approves a plan update to accommodate a deviation.
    /// The new plan becomes the baseline for subsequent deviation checks.
    async fn update_plan(
        &self,
        task_id: Uuid,
        updated_plan: Plan,
    ) -> Result<(), DeviationError>;

    /// Returns the configured deviation threshold.
    ///
    /// Deviations are flagged when semantic similarity drops below this value.
    /// Default: 0.75.
    fn deviation_threshold(&self) -> f32;

    /// Returns whether any unresolved critical deviations exist for a task.
    async fn has_unresolved_critical(
        &self,
        task_id: Uuid,
    ) -> Result<bool, DeviationError>;
}
