//! Context budget manager implementation.
//!
//! Tracks per-session token consumption, computes utilization percentages,
//! and recommends actions (Continue, Compact, Handoff, Halt) based on
//! configurable thresholds.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::RwLock;
use tracing::{debug, instrument};
use uuid::Uuid;

use truenorth_core::traits::context::{BudgetError, ContextBudgetManager};
use truenorth_core::traits::memory::CompactionResult;
use truenorth_core::types::context::{
    BudgetAction, ContextBudget, ContextThresholds, ContextUtilization,
};

/// Per-session budget state (internal).
#[derive(Debug, Clone)]
struct BudgetState {
    budget: ContextBudget,
    thresholds: ContextThresholds,
    reservations: HashMap<String, usize>,
}

impl BudgetState {
    fn new(session_id: Uuid, total_tokens: usize, thresholds: ContextThresholds) -> Self {
        let budget = ContextBudget {
            session_id,
            total_tokens,
            history_tokens: 0,
            system_tokens: 0,
            response_reserve: 1024,
            reserved_tokens: 0,
            utilization: 0.0,
            compaction_count: 0,
            handoff_issued: false,
        };
        Self {
            budget,
            thresholds,
            reservations: HashMap::new(),
        }
    }

    fn recalculate_utilization(&mut self) {
        let used = self.budget.history_tokens
            + self.budget.system_tokens
            + self.budget.reserved_tokens
            + self.budget.response_reserve;
        self.budget.utilization = if self.budget.total_tokens == 0 {
            0.0
        } else {
            used as f32 / self.budget.total_tokens as f32
        };
    }
}

/// Default context budget manager.
///
/// Maintains a per-session HashMap of `ContextBudget` states.
/// All operations are synchronous (uses `parking_lot::RwLock` for lock-free reads).
#[derive(Debug)]
pub struct DefaultContextBudgetManager {
    sessions: RwLock<HashMap<Uuid, BudgetState>>,
}

impl DefaultContextBudgetManager {
    /// Creates a new `DefaultContextBudgetManager`.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for DefaultContextBudgetManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextBudgetManager for DefaultContextBudgetManager {
    #[instrument(skip(self), fields(session_id = %session_id))]
    fn initialize(
        &self,
        session_id: Uuid,
        total_tokens: usize,
        thresholds: ContextThresholds,
    ) -> Result<ContextBudget, BudgetError> {
        let state = BudgetState::new(session_id, total_tokens, thresholds);
        let budget = state.budget.clone();
        self.sessions.write().insert(session_id, state);
        debug!("Initialized context budget for session {}: {} tokens", session_id, total_tokens);
        Ok(budget)
    }

    fn current_budget(&self, session_id: Uuid) -> Result<ContextBudget, BudgetError> {
        self.sessions.read()
            .get(&session_id)
            .map(|s| s.budget.clone())
            .ok_or(BudgetError::NoBudget { session_id })
    }

    fn utilization(&self, session_id: Uuid) -> Result<ContextUtilization, BudgetError> {
        let sessions = self.sessions.read();
        let state = sessions.get(&session_id)
            .ok_or(BudgetError::NoBudget { session_id })?;
        Ok(ContextUtilization::from_budget(&state.budget, &state.thresholds))
    }

    fn recommended_action(&self, session_id: Uuid) -> Result<BudgetAction, BudgetError> {
        let sessions = self.sessions.read();
        let state = sessions.get(&session_id)
            .ok_or(BudgetError::NoBudget { session_id })?;
        let util = state.budget.utilization;
        let t = &state.thresholds;

        if util >= t.halt_at {
            Ok(BudgetAction::Halt)
        } else if util >= t.handoff_at {
            Ok(BudgetAction::Handoff)
        } else if util >= t.compact_at {
            Ok(BudgetAction::Compact)
        } else {
            Ok(BudgetAction::Continue)
        }
    }

    fn estimate_tokens(&self, messages: &[serde_json::Value]) -> usize {
        // Rough estimate: 4 chars ≈ 1 token (cl100k_base encoding)
        messages.iter()
            .map(|m| {
                m.as_str().map(|s| s.len()).unwrap_or_else(|| {
                    m.to_string().len()
                }) / 4
            })
            .sum::<usize>()
            .max(1)
    }

    fn can_fit(&self, session_id: Uuid, additional_tokens: usize) -> bool {
        if let Ok(budget) = self.current_budget(session_id) {
            budget.available_tokens() >= additional_tokens
        } else {
            false
        }
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    fn record_usage(
        &self,
        session_id: Uuid,
        input_tokens: usize,
        output_tokens: usize,
    ) -> Result<(), BudgetError> {
        let mut sessions = self.sessions.write();
        let state = sessions.get_mut(&session_id)
            .ok_or(BudgetError::NoBudget { session_id })?;

        state.budget.history_tokens += input_tokens + output_tokens;
        state.recalculate_utilization();

        debug!("Recorded {} input + {} output tokens for session {}. Util: {:.1}%",
            input_tokens, output_tokens, session_id,
            state.budget.utilization * 100.0);
        Ok(())
    }

    fn record_system_tokens(
        &self,
        session_id: Uuid,
        system_tokens: usize,
    ) -> Result<(), BudgetError> {
        let mut sessions = self.sessions.write();
        let state = sessions.get_mut(&session_id)
            .ok_or(BudgetError::NoBudget { session_id })?;

        state.budget.system_tokens = system_tokens;
        state.recalculate_utilization();
        Ok(())
    }

    async fn compact(
        &self,
        session_id: Uuid,
        _history: &[serde_json::Value],
    ) -> Result<CompactionResult, BudgetError> {
        let mut sessions = self.sessions.write();
        let state = sessions.get_mut(&session_id)
            .ok_or(BudgetError::NoBudget { session_id })?;

        let before = state.budget.history_tokens;
        // Simple compaction: keep last 25% of history
        let after = before / 4;
        let _saved = before.saturating_sub(after);

        state.budget.history_tokens = after;
        state.budget.compaction_count += 1;
        state.recalculate_utilization();

        Ok(CompactionResult {
            session_id,
            messages_removed: (before.saturating_sub(after) / 200).max(0),
            tokens_before: before,
            tokens_after: after,
            summary: format!("Compacted session context: {} → {} tokens", before, after),
        })
    }

    fn reserve(
        &self,
        session_id: Uuid,
        token_count: usize,
        reservation_id: &str,
    ) -> bool {
        let mut sessions = self.sessions.write();
        let state = match sessions.get_mut(&session_id) {
            Some(s) => s,
            None => return false,
        };

        if state.budget.available_tokens() < token_count {
            return false;
        }

        state.reservations.insert(reservation_id.to_string(), token_count);
        state.budget.reserved_tokens += token_count;
        state.recalculate_utilization();
        true
    }

    fn release_reservation(&self, session_id: Uuid, reservation_id: &str) {
        let mut sessions = self.sessions.write();
        if let Some(state) = sessions.get_mut(&session_id) {
            if let Some(count) = state.reservations.remove(reservation_id) {
                state.budget.reserved_tokens = state.budget.reserved_tokens.saturating_sub(count);
                state.recalculate_utilization();
            }
        }
    }

    fn reset_for_handoff(&self, session_id: Uuid) -> Result<(), BudgetError> {
        let mut sessions = self.sessions.write();
        let state = sessions.get_mut(&session_id)
            .ok_or(BudgetError::NoBudget { session_id })?;

        state.budget.history_tokens = 0;
        state.budget.reserved_tokens = 0;
        state.budget.handoff_issued = true;
        state.reservations.clear();
        state.recalculate_utilization();
        Ok(())
    }

    fn cleanup(&self, session_id: Uuid) {
        self.sessions.write().remove(&session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::context::BudgetAction;

    #[test]
    fn initialize_and_track_usage() {
        let mgr = DefaultContextBudgetManager::new();
        let session_id = Uuid::new_v4();
        let thresholds = ContextThresholds::default();
        let budget = mgr.initialize(session_id, 10000, thresholds).unwrap();
        assert_eq!(budget.total_tokens, 10000);
        assert_eq!(budget.history_tokens, 0);

        mgr.record_usage(session_id, 1000, 500).unwrap();
        let budget = mgr.current_budget(session_id).unwrap();
        assert_eq!(budget.history_tokens, 1500);
    }

    #[test]
    fn recommended_action_continue_when_low() {
        let mgr = DefaultContextBudgetManager::new();
        let session_id = Uuid::new_v4();
        mgr.initialize(session_id, 10000, ContextThresholds::default()).unwrap();
        assert_eq!(mgr.recommended_action(session_id).unwrap(), BudgetAction::Continue);
    }

    #[test]
    fn recommended_action_compact_at_threshold() {
        let mgr = DefaultContextBudgetManager::new();
        let session_id = Uuid::new_v4();
        mgr.initialize(session_id, 10000, ContextThresholds::default()).unwrap();
        // Use 72% (above 70% compact threshold)
        mgr.record_usage(session_id, 7200, 0).unwrap();
        let action = mgr.recommended_action(session_id).unwrap();
        assert!(matches!(action, BudgetAction::Compact | BudgetAction::Handoff | BudgetAction::Halt));
    }

    #[test]
    fn reservation_reduces_available_tokens() {
        let mgr = DefaultContextBudgetManager::new();
        let session_id = Uuid::new_v4();
        mgr.initialize(session_id, 10000, ContextThresholds::default()).unwrap();
        let ok = mgr.reserve(session_id, 5000, "test-reservation");
        assert!(ok);
        let budget = mgr.current_budget(session_id).unwrap();
        assert_eq!(budget.reserved_tokens, 5000);
        mgr.release_reservation(session_id, "test-reservation");
        let budget = mgr.current_budget(session_id).unwrap();
        assert_eq!(budget.reserved_tokens, 0);
    }

    #[tokio::test]
    async fn compact_reduces_history_tokens() {
        let mgr = DefaultContextBudgetManager::new();
        let session_id = Uuid::new_v4();
        mgr.initialize(session_id, 10000, ContextThresholds::default()).unwrap();
        mgr.record_usage(session_id, 8000, 0).unwrap();
        let result = mgr.compact(session_id, &[]).await.unwrap();
        assert!(result.tokens_after < result.tokens_before);
    }
}
