//! Infinite loop detection module.
//!
//! Three complementary mechanisms protect against infinite loops:
//! - `StepCounter` — enforces a maximum step count per task
//! - `SemanticSimilarityGuard` — detects identical/near-identical consecutive outputs
//! - `Watchdog` — enforces a wall-clock time limit per task

pub mod semantic_similarity;
pub mod step_counter;
pub mod watchdog;
