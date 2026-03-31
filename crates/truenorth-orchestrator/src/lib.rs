//! # truenorth-orchestrator
//!
//! The central orchestrator for TrueNorth: wires together all subsystems
//! (LLM, memory, tools, skills, visual) into a coherent agent loop.
//!
//! ## Architecture
//!
//! The orchestrator implements the full agent loop state machine:
//! Idle → Intake → ContextGathering → ComplexityAssessment → Planning →
//! Executing → Reflecting → (CriticReview → SynthesisResolve) → Complete.
//!
//! ### Key components
//!
//! - **[`Orchestrator`]** — Top-level struct wiring all subsystems together.
//! - **[`agent_loop`]** — Agent loop implementation with state machine transitions.
//! - **[`execution_modes`]** — Direct, Sequential, Parallel, Graph, and R/C/S strategies.
//! - **[`context`]** — Context budget manager and compaction policy.
//! - **[`session`]** — Session lifecycle management, persistence, and handoff.
//! - **[`checklist`]** — Negative checklist verifier for anti-pattern detection.
//! - **[`deviation`]** — Plan deviation tracker and alert system.
//! - **[`heartbeat`]** — Persistent scheduled agent scheduler.
//! - **[`loop_guard`]** — Infinite loop detection, step counting, and watchdog.

#![warn(missing_docs)]
#![allow(clippy::module_name_repetitions)]

pub mod agent_loop;
pub mod checklist;
pub mod context;
pub mod deviation;
pub mod execution_modes;
pub mod heartbeat;
pub mod loop_guard;
pub mod orchestrator;
pub mod session;

// Re-export the primary entry points
pub use orchestrator::{Orchestrator, OrchestratorBuilder, OrchestratorConfig};
pub use agent_loop::executor::AgentLoopExecutor;
pub use execution_modes::{
    direct::DirectExecutionStrategy,
    sequential::SequentialExecutionStrategy,
    parallel::ParallelExecutionStrategy,
    graph::GraphExecutionStrategy,
    rcs::RCSExecutionStrategy,
};
pub use context::budget_manager::DefaultContextBudgetManager;
pub use session::manager::DefaultSessionManager;
pub use session::serializer::SqliteStateSerializer;
pub use checklist::verifier::DefaultNegativeChecklist;
pub use deviation::tracker::DefaultDeviationTracker;
pub use heartbeat::scheduler::DefaultHeartbeatScheduler;
pub use loop_guard::watchdog::Watchdog;
