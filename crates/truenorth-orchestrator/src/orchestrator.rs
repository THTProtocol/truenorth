//! Main Orchestrator struct — wires all subsystems together.
//!
//! The `Orchestrator` is the top-level entry point for all agent operations.
//! It holds `Arc` references to every subsystem and serves as the dependency
//! injection root for the entire TrueNorth system.

use std::sync::Arc;

use tracing::{info, instrument};

use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::heartbeat::HeartbeatScheduler;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::task::Task;
use truenorth_core::traits::execution::{AgentLoop, ExecutionError, TaskResult};

use crate::agent_loop::executor::AgentLoopExecutor;
use crate::context::budget_manager::DefaultContextBudgetManager;
use crate::session::manager::DefaultSessionManager;
use crate::session::serializer::SqliteStateSerializer;
use crate::checklist::verifier::DefaultNegativeChecklist;
use crate::deviation::tracker::DefaultDeviationTracker;
use crate::heartbeat::scheduler::DefaultHeartbeatScheduler;

/// Configuration for the Orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Maximum steps per task before loop guard triggers.
    pub max_steps: usize,
    /// Wall-clock task timeout in seconds.
    pub task_timeout_secs: u64,
    /// Whether plan approval is required before execution (PAUL mode).
    pub require_plan_approval: bool,
    /// Default context token budget per session.
    pub default_context_budget: usize,
    /// Complexity score threshold above which R/C/S is activated.
    pub rcs_threshold: f32,
    /// Maximum R/C/S loop iterations before escalating.
    pub max_rcs_iterations: u8,
    /// SQLite database path for session persistence.
    pub sessions_db_path: String,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_steps: 50,
            task_timeout_secs: 1800,
            require_plan_approval: false,
            default_context_budget: 100_000,
            rcs_threshold: 0.75,
            max_rcs_iterations: 3,
            sessions_db_path: "./truenorth_sessions.db".to_string(),
        }
    }
}

/// The central TrueNorth orchestrator.
///
/// Holds Arc references to all subsystems and provides the top-level
/// `run_task` entry point. All subsystems implement their respective
/// core traits for testability and swappability.
#[derive(Debug, Clone)]
pub struct Orchestrator {
    /// The agent loop implementation.
    pub agent_loop: Arc<AgentLoopExecutor>,
    /// The context budget manager.
    pub budget_manager: Arc<DefaultContextBudgetManager>,
    /// The session manager.
    pub session_manager: Arc<DefaultSessionManager>,
    /// The negative checklist verifier.
    pub checklist: Arc<DefaultNegativeChecklist>,
    /// The deviation tracker.
    pub deviation_tracker: Arc<DefaultDeviationTracker>,
    /// The heartbeat scheduler.
    pub heartbeat_scheduler: Arc<DefaultHeartbeatScheduler>,
    /// Configuration.
    pub config: OrchestratorConfig,
}

impl Orchestrator {
    /// Creates a new `OrchestratorBuilder`.
    pub fn builder() -> OrchestratorBuilder {
        OrchestratorBuilder::default()
    }

    /// Runs a task through the full agent loop.
    ///
    /// This is the primary entry point for all agent work.
    /// The agent loop handles state machine transitions, LLM calls,
    /// tool dispatch, context management, and result synthesis.
    #[instrument(skip(self, task), fields(task_id = %task.id, title = %task.title))]
    pub async fn run_task(&self, task: Task) -> Result<TaskResult, ExecutionError> {
        info!("Orchestrator: running task {}", task.id);
        self.agent_loop.run(task).await
    }

    /// Returns the current state of the agent loop as a string.
    pub fn current_state(&self) -> String {
        self.agent_loop.current_state()
    }

    /// Returns whether the agent loop is currently running.
    pub fn is_running(&self) -> bool {
        self.agent_loop.is_running()
    }

    /// Starts background tasks (heartbeat scheduler, memory consolidation, etc.).
    pub async fn start_background_tasks(&self) {
        info!("Orchestrator: starting background tasks");
        if let Err(e) = self.heartbeat_scheduler.start().await {
            tracing::warn!("Failed to start heartbeat scheduler: {}", e);
        }
    }

    /// Gracefully shuts down the orchestrator.
    pub async fn shutdown(&self) {
        info!("Orchestrator: shutting down");
        if let Err(e) = self.heartbeat_scheduler.shutdown().await {
            tracing::warn!("Heartbeat scheduler shutdown error: {}", e);
        }
    }
}

/// Builder for the `Orchestrator`.
///
/// Use `Orchestrator::builder()` to start, then call the configuration
/// methods, and finally call `build()` to get a ready-to-use orchestrator.
#[derive(Default)]
pub struct OrchestratorBuilder {
    llm_router: Option<Arc<dyn LlmRouter>>,
    event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    config: OrchestratorConfig,
}

impl OrchestratorBuilder {
    /// Sets the LLM router for the orchestrator.
    pub fn with_llm_router(mut self, router: Arc<dyn LlmRouter>) -> Self {
        self.llm_router = Some(router);
        self
    }

    /// Sets the reasoning event emitter (visual event bus).
    pub fn with_event_emitter(mut self, emitter: Arc<dyn ReasoningEventEmitter>) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Sets the orchestrator configuration.
    pub fn with_config(mut self, config: OrchestratorConfig) -> Self {
        self.config = config;
        self
    }

    /// Builds the orchestrator with all subsystems wired together.
    ///
    /// # Errors
    /// Returns an error if required subsystems (e.g., LLM router) are not configured.
    pub fn build(self) -> Result<Orchestrator, anyhow::Error> {
        let config = self.config;

        let budget_manager = Arc::new(DefaultContextBudgetManager::new());
        let checklist = Arc::new(DefaultNegativeChecklist::new());
        let deviation_tracker = Arc::new(DefaultDeviationTracker::new());
        let state_serializer = Arc::new(
            SqliteStateSerializer::new(&config.sessions_db_path)?
        );
        let session_manager = Arc::new(DefaultSessionManager::new(state_serializer));
        let heartbeat_scheduler = Arc::new(DefaultHeartbeatScheduler::new());

        let agent_loop = Arc::new(AgentLoopExecutor::new(
            self.llm_router,
            self.event_emitter,
            budget_manager.clone(),
            session_manager.clone(),
            checklist.clone(),
            deviation_tracker.clone(),
            config.clone(),
        ));

        Ok(Orchestrator {
            agent_loop,
            budget_manager,
            session_manager,
            checklist,
            deviation_tracker,
            heartbeat_scheduler,
            config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_sanity() {
        let cfg = OrchestratorConfig::default();
        assert_eq!(cfg.max_steps, 50);
        assert!(cfg.rcs_threshold > 0.0 && cfg.rcs_threshold < 1.0);
    }

    #[test]
    fn builder_builds_without_llm() {
        // Builder should succeed even without an LLM router (it will fail on run)
        let result = Orchestrator::builder().build();
        assert!(result.is_ok());
    }
}
