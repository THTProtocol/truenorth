/// Context types — context window budget tracking and management.
///
/// The context budget manager monitors token consumption across the session
/// and triggers compaction, handoff, or halt actions before the context
/// window is fully exhausted. These types represent the budget state
/// and the actions the budget manager recommends.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The context window budget for a session.
///
/// Tracks token consumption across all categories (history, system, response).
/// The budget manager maintains one of these per active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudget {
    /// The session this budget belongs to.
    pub session_id: Uuid,
    /// Total token budget for this session (set at session creation).
    pub total_tokens: usize,
    /// Tokens currently consumed by conversation history.
    pub history_tokens: usize,
    /// Tokens consumed by the system prompt, injected skills, and tool definitions.
    pub system_tokens: usize,
    /// Tokens reserved for the next LLM response.
    pub response_reserve: usize,
    /// Tokens explicitly reserved for upcoming operations (see `reserve()`).
    pub reserved_tokens: usize,
    /// Current utilization as a fraction (0.0–1.0).
    /// Computed as `(history + system + reserved) / total`.
    pub utilization: f32,
    /// Number of times compaction has been run this session.
    pub compaction_count: usize,
    /// Whether a handoff document has been created this session.
    pub handoff_issued: bool,
}

impl ContextBudget {
    /// Returns the number of tokens still available (not consumed or reserved).
    pub fn available_tokens(&self) -> usize {
        let used = self.history_tokens + self.system_tokens + self.reserved_tokens + self.response_reserve;
        self.total_tokens.saturating_sub(used)
    }

    /// Returns true if the budget is in a healthy state (below the compact threshold).
    pub fn is_healthy(&self, compact_threshold: f32) -> bool {
        self.utilization < compact_threshold
    }
}

/// The action the budget manager recommends based on current utilization.
///
/// The agent loop checks this after every LLM call and acts accordingly.
/// Actions are ordered by urgency: Continue < Compact < Handoff < Halt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetAction {
    /// Context is healthy; no action needed.
    Continue,
    /// Approaching compact threshold (default: 70%); summarize older history.
    Compact,
    /// Approaching handoff threshold (default: 90%); create handoff document.
    Handoff,
    /// At halt threshold (default: 98%); save state and stop execution.
    Halt,
}

/// Utilization breakdown by category.
///
/// Used for display in the Visual Reasoning Layer's context panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUtilization {
    /// The session this utilization snapshot belongs to.
    pub session_id: Uuid,
    /// Total token budget.
    pub total: usize,
    /// Tokens consumed by conversation history.
    pub history: usize,
    /// Tokens consumed by system prompt + tools + skills.
    pub system: usize,
    /// Tokens reserved for the next response.
    pub response_reserve: usize,
    /// Tokens held for anticipated upcoming operations.
    pub reserved: usize,
    /// Remaining available tokens.
    pub available: usize,
    /// Overall utilization fraction (0.0–1.0).
    pub utilization_fraction: f32,
    /// Percentage representation of utilization (0–100).
    pub utilization_percent: u8,
    /// The recommended action given current utilization.
    pub recommended_action: BudgetAction,
}

impl ContextUtilization {
    /// Creates a utilization snapshot from a budget and configured thresholds.
    pub fn from_budget(budget: &ContextBudget, thresholds: &ContextThresholds) -> Self {
        let used = budget.history_tokens + budget.system_tokens + budget.reserved_tokens + budget.response_reserve;
        let available = budget.total_tokens.saturating_sub(used);
        let fraction = if budget.total_tokens == 0 {
            0.0
        } else {
            used as f32 / budget.total_tokens as f32
        };
        let percent = (fraction * 100.0).min(100.0) as u8;

        let recommended_action = if fraction >= thresholds.halt_at {
            BudgetAction::Halt
        } else if fraction >= thresholds.handoff_at {
            BudgetAction::Handoff
        } else if fraction >= thresholds.compact_at {
            BudgetAction::Compact
        } else {
            BudgetAction::Continue
        };

        Self {
            session_id: budget.session_id,
            total: budget.total_tokens,
            history: budget.history_tokens,
            system: budget.system_tokens,
            response_reserve: budget.response_reserve,
            reserved: budget.reserved_tokens,
            available,
            utilization_fraction: fraction,
            utilization_percent: percent,
            recommended_action,
        }
    }
}

/// Threshold configuration for context budget actions.
///
/// Stored per-session and respected by the budget manager when
/// determining which action to recommend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextThresholds {
    /// Utilization fraction at which to start compacting history. Default: 0.70.
    pub compact_at: f32,
    /// Utilization fraction at which to create a handoff document. Default: 0.90.
    pub handoff_at: f32,
    /// Utilization fraction at which to halt and save state. Default: 0.98.
    pub halt_at: f32,
}

impl Default for ContextThresholds {
    fn default() -> Self {
        Self {
            compact_at: 0.70,
            handoff_at: 0.90,
            halt_at: 0.98,
        }
    }
}

/// A token reservation for an anticipated upcoming operation.
///
/// Created by `ContextBudgetManager::reserve()` to hold space for operations
/// that are about to consume tokens (e.g., a large tool result about to be
/// injected). Released after the operation completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenReservation {
    /// The session this reservation belongs to.
    pub session_id: Uuid,
    /// A caller-supplied identifier for this reservation.
    pub reservation_id: String,
    /// Number of tokens reserved.
    pub token_count: usize,
    /// A description of what this reservation is for (for logging).
    pub purpose: String,
}

/// Context budget statistics for a session.
///
/// Aggregated over the session lifetime for monitoring and optimization.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextBudgetStats {
    /// Total LLM calls made this session.
    pub total_llm_calls: u64,
    /// Total input tokens consumed.
    pub total_input_tokens: u64,
    /// Total output tokens consumed.
    pub total_output_tokens: u64,
    /// Number of compactions performed.
    pub compactions_performed: u64,
    /// Total tokens saved by compaction.
    pub tokens_saved_by_compaction: u64,
    /// Number of handoffs issued.
    pub handoffs_issued: u64,
    /// Peak utilization fraction reached this session.
    pub peak_utilization: f32,
}
