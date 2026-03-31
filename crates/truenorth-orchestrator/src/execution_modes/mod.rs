//! Execution strategy implementations.
//!
//! Each execution strategy implements a specific mode of task execution:
//! - `DirectExecutionStrategy` — single-shot Reason → Act → Respond
//! - `SequentialExecutionStrategy` — ordered multi-step execution
//! - `ParallelExecutionStrategy` — concurrent independent sub-tasks
//! - `GraphExecutionStrategy` — DAG execution with conditional routing
//! - `RCSExecutionStrategy` — Reason → Critic → Synthesis with fresh contexts

pub mod direct;
pub mod graph;
pub mod parallel;
pub mod rcs;
pub mod sequential;
