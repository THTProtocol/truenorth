//! Heartbeat scheduler module.
//!
//! Manages persistent scheduled agents (Paperclip pattern).
//! Registered heartbeats fire on a configurable interval and dispatch
//! tasks to the agent loop with circuit-breaker semantics.

pub mod scheduler;
