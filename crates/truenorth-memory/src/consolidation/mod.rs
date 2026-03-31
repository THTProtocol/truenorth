//! Consolidation subsystem — autoDream-style background memory optimization.
//!
//! The consolidation cycle runs between sessions to merge, promote, and prune
//! memory entries. It implements the four-phase autoDream cycle:
//!
//! ```text
//! Orient → Gather → Consolidate → Prune
//! ```
//!
//! ## Modules
//!
//! - [`consolidator`] — `AutoDreamConsolidator`: runs the four-phase cycle.
//! - [`scheduler`] — `ConsolidationScheduler`: gates and triggers consolidation.

pub mod consolidator;
pub mod scheduler;

pub use consolidator::AutoDreamConsolidator;
pub use scheduler::ConsolidationScheduler;
