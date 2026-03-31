//! `ConsolidationScheduler` — gates and triggers background consolidation.
//!
//! The scheduler is a lightweight state machine that decides when to trigger
//! the autoDream consolidation cycle. It enforces three gates:
//!
//! 1. **Time gate**: at least `min_interval_hours` must have elapsed since the
//!    last consolidation.
//!
//! 2. **Session gate**: at least `min_sessions` new sessions must have ended
//!    since the last consolidation.
//!
//! 3. **Lock gate**: no consolidation is currently running (prevents concurrent
//!    cycles that could corrupt the memory store).
//!
//! The scheduler loop wakes every 5 minutes and checks all gates. If all three
//! pass, it spawns the consolidation as a tokio task.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::consolidation::consolidator::AutoDreamConsolidator;
use truenorth_core::types::memory::MemoryScope;

/// How often the scheduler loop wakes to check the gates.
const POLL_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Internal scheduler state shared between the loop task and gate checks.
#[derive(Debug)]
struct SchedulerState {
    /// When the last successful consolidation completed.
    last_consolidated_at: Option<DateTime<Utc>>,
    /// Number of sessions ended since the last consolidation.
    sessions_since_last: usize,
    /// Whether a consolidation cycle is currently running.
    is_running: bool,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self {
            last_consolidated_at: None,
            sessions_since_last: 0,
            is_running: false,
        }
    }
}

/// Background consolidation scheduler.
///
/// Holds the consolidator and the gate parameters. The `run_loop` method
/// should be called once in a dedicated tokio task.
#[derive(Debug)]
pub struct ConsolidationScheduler {
    consolidator: Arc<AutoDreamConsolidator>,
    /// Minimum hours between consolidation runs.
    min_interval_hours: u64,
    /// Minimum sessions required before triggering consolidation.
    min_sessions: usize,
    /// Shared state (protected for concurrent access).
    state: Arc<Mutex<SchedulerState>>,
}

impl ConsolidationScheduler {
    /// Create a new `ConsolidationScheduler`.
    pub fn new(
        consolidator: Arc<AutoDreamConsolidator>,
        min_interval_hours: u64,
        min_sessions: usize,
    ) -> Self {
        Self {
            consolidator,
            min_interval_hours,
            min_sessions,
            state: Arc::new(Mutex::new(SchedulerState::default())),
        }
    }

    /// Notify the scheduler that a session has ended.
    ///
    /// Increments the `sessions_since_last` counter. If all gates pass after
    /// incrementing, triggers an immediate consolidation.
    pub async fn on_session_end(&mut self, session_id: Uuid) {
        {
            let mut state = self.state.lock().await;
            state.sessions_since_last += 1;
            debug!(
                "ConsolidationScheduler: session {} ended ({} since last)",
                session_id, state.sessions_since_last
            );
        }

        // Check if we should trigger now.
        if self.gates_pass().await {
            self.trigger_consolidation().await;
        }
    }

    /// Run the background polling loop.
    ///
    /// Wakes every `POLL_INTERVAL_SECS` seconds and checks the three gates.
    /// This method never returns; it should be spawned as a tokio task.
    pub async fn run_loop(&self) {
        info!(
            "ConsolidationScheduler: started (interval={}h, min_sessions={})",
            self.min_interval_hours, self.min_sessions
        );
        loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;

            if self.gates_pass().await {
                self.trigger_consolidation().await;
            }
        }
    }

    /// Check whether all three gates are open.
    async fn gates_pass(&self) -> bool {
        let state = self.state.lock().await;

        // Gate 3: no consolidation is currently running.
        if state.is_running {
            debug!("ConsolidationScheduler: gate LOCK — consolidation already running");
            return false;
        }

        // Gate 1: time since last consolidation.
        if let Some(last) = state.last_consolidated_at {
            let elapsed_hours = (Utc::now() - last).num_hours();
            if elapsed_hours < self.min_interval_hours as i64 {
                debug!(
                    "ConsolidationScheduler: gate TIME — only {}h since last (need {}h)",
                    elapsed_hours, self.min_interval_hours
                );
                return false;
            }
        }

        // Gate 2: enough new sessions.
        if state.sessions_since_last < self.min_sessions {
            debug!(
                "ConsolidationScheduler: gate SESSIONS — only {} sessions (need {})",
                state.sessions_since_last, self.min_sessions
            );
            return false;
        }

        true
    }

    /// Trigger a consolidation cycle in the background.
    ///
    /// Sets `is_running = true`, spawns the consolidation task, and resets
    /// `sessions_since_last` and `last_consolidated_at` on completion.
    async fn trigger_consolidation(&self) {
        // Mark as running.
        {
            let mut state = self.state.lock().await;
            state.is_running = true;
            state.sessions_since_last = 0;
        }

        info!("ConsolidationScheduler: triggering consolidation cycle");

        let consolidator = self.consolidator.clone();
        let state = self.state.clone();

        tokio::spawn(async move {
            // Run consolidation for all persistent scopes.
            let scopes = [MemoryScope::Project, MemoryScope::Identity];
            for scope in &scopes {
                match consolidator.run(*scope).await {
                    Ok(report) => {
                        info!(
                            "ConsolidationScheduler: {} consolidation complete \
                             (reviewed={}, created={}, merged={}, pruned={}, {}ms)",
                            format!("{:?}", scope),
                            report.entries_reviewed,
                            report.entries_created,
                            report.entries_merged,
                            report.entries_pruned,
                            report.duration_ms,
                        );
                    }
                    Err(e) => {
                        warn!("ConsolidationScheduler: consolidation for {:?} failed: {e}", scope);
                    }
                }
            }

            // Update state after completion.
            let mut s = state.lock().await;
            s.is_running = false;
            s.last_consolidated_at = Some(Utc::now());
            debug!("ConsolidationScheduler: cycle complete, releasing lock");
        });
    }

    /// Force an immediate consolidation regardless of gate state.
    ///
    /// For use via the CLI (`truenorth memory consolidate`).
    pub async fn force_consolidate(&self) {
        info!("ConsolidationScheduler: forced consolidation triggered");
        self.trigger_consolidation().await;
    }

    /// Return whether a consolidation is currently running.
    pub async fn is_running(&self) -> bool {
        self.state.lock().await.is_running
    }

    /// Return when the last consolidation completed, or `None` if never run.
    pub async fn last_consolidated_at(&self) -> Option<DateTime<Utc>> {
        self.state.lock().await.last_consolidated_at
    }

    /// Return the number of sessions since the last consolidation.
    pub async fn sessions_since_last(&self) -> usize {
        self.state.lock().await.sessions_since_last
    }
}
