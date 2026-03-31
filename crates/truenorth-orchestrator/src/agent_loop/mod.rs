//! Agent loop module — the core execution driver.
//!
//! This module implements the full agent loop including:
//! - The `AgentState` state machine with all transitions
//! - The `AgentLoopExecutor` implementing the `AgentLoop` trait
//! - Task planning and complexity assessment
//! - Single-step execution with tool calls and observation

pub mod executor;
pub mod planner;
pub mod state_machine;
pub mod step_runner;
