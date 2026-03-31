//! Context management module.
//!
//! Provides the `ContextBudgetManager` implementation and compaction policies
//! for managing the LLM context window across a session.

pub mod budget_manager;
pub mod compaction_policy;
