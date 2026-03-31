//! Deviation tracking module.
//!
//! Monitors execution fidelity against the approved plan.
//! Compares step outputs against planned step descriptions using
//! semantic similarity (heuristic cosine similarity fallback when
//! embedding providers are not available).

pub mod tracker;
