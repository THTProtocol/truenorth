/// ContextBudgetManager trait — context window management.
///
/// Monitors token consumption and triggers compaction, handoff, or halt
/// before the context window is exhausted. The agent loop delegates all
/// context tracking to this component — it never tracks tokens directly.

use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::types::context::{BudgetAction, ContextBudget, ContextThresholds, ContextUtilization};
use crate::traits::memory::CompactionResult;

/// Errors from context budget management.
#[derive(Debug, Error)]
pub enum BudgetError {
    /// The session has no budget configured (session not found).
    #[error("Session {session_id} has no budget configured")]
    NoBudget { session_id: Uuid },

    /// Token estimation failed.
    #[error("Token estimation failed: {message}")]
    EstimationError { message: String },

    /// Context compaction failed.
    #[error("Context compaction failed: {message}")]
    CompactionFailed { message: String },

    /// The budget manager is in an invalid state.
    #[error("Budget manager internal error: {message}")]
    Internal { message: String },
}

/// The context budget manager: tracks and enforces context window limits.
///
/// Design rationale: context anxiety (Article 3 failure mode) is one of the
/// most destructive problems in autonomous agents. The budget manager is the
/// structural solution: it monitors utilization continuously, triggers
/// compaction proactively at 70%, initiates handoff at 90%, and halts at 98%.
/// The agent loop never needs to track token counts — it delegates entirely
/// to this component.
#[async_trait]
pub trait ContextBudgetManager: Send + Sync + std::fmt::Debug {
    /// Initializes a context budget for a new session.
    ///
    /// Must be called before any other budget operations for the session.
    /// Sets the total token budget and configures action thresholds.
    fn initialize(
        &self,
        session_id: Uuid,
        total_tokens: usize,
        thresholds: ContextThresholds,
    ) -> Result<ContextBudget, BudgetError>;

    /// Returns the current budget state for a session.
    fn current_budget(&self, session_id: Uuid) -> Result<ContextBudget, BudgetError>;

    /// Returns a detailed utilization snapshot for a session.
    fn utilization(&self, session_id: Uuid) -> Result<ContextUtilization, BudgetError>;

    /// Returns what action the budget manager recommends right now.
    fn recommended_action(&self, session_id: Uuid) -> Result<BudgetAction, BudgetError>;

    /// Estimates the token count for a serialized message list.
    ///
    /// Uses a tokenizer appropriate for the current LLM provider.
    /// The estimate is a conservative upper bound (rounds up).
    fn estimate_tokens(&self, messages: &[serde_json::Value]) -> usize;

    /// Checks whether a proposed addition would fit within the context budget.
    ///
    /// Returns true if adding `additional_tokens` would not trigger compaction.
    /// Used before injecting large content (tool results, skill bodies, memory retrievals).
    fn can_fit(&self, session_id: Uuid, additional_tokens: usize) -> bool;

    /// Records tokens consumed by an LLM completion.
    ///
    /// Called after every `LlmRouter::route()` call with the actual token usage.
    /// Updates the running token count and recalculates utilization.
    fn record_usage(
        &self,
        session_id: Uuid,
        input_tokens: usize,
        output_tokens: usize,
    ) -> Result<(), BudgetError>;

    /// Records tokens consumed by the system prompt and tool definitions.
    ///
    /// Called once per session when the system prompt is first built.
    fn record_system_tokens(
        &self,
        session_id: Uuid,
        system_tokens: usize,
    ) -> Result<(), BudgetError>;

    /// Requests a context compaction.
    ///
    /// Summarizes older conversation history to free up context space.
    /// Returns the compaction result including the summary and tokens saved.
    async fn compact(
        &self,
        session_id: Uuid,
        history: &[serde_json::Value],
    ) -> Result<CompactionResult, BudgetError>;

    /// Reserves tokens for an anticipated upcoming operation.
    ///
    /// Returns false if the reservation cannot be satisfied without triggering halt.
    /// The reservation is tracked by `reservation_id` and released after use.
    fn reserve(
        &self,
        session_id: Uuid,
        token_count: usize,
        reservation_id: &str,
    ) -> bool;

    /// Releases a previously made token reservation.
    ///
    /// Called after the anticipated operation completes (whether or not it used
    /// all the reserved tokens).
    fn release_reservation(&self, session_id: Uuid, reservation_id: &str);

    /// Resets the budget for a session (used on handoff to a new context window).
    ///
    /// Preserves the session ID and thresholds but clears token counts.
    fn reset_for_handoff(&self, session_id: Uuid) -> Result<(), BudgetError>;

    /// Removes the budget for a session (called on session end).
    fn cleanup(&self, session_id: Uuid);
}
