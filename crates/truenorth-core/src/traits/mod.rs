/// Trait modules for the TrueNorth core crate.
///
/// Every inter-crate interface is defined here as a trait.
/// No module communicates with another module except through these
/// trait boundaries. This enforces testability and modularity.

pub mod llm_provider;
pub mod llm_router;
pub mod embedding_provider;
pub mod tool;
pub mod skill;
pub mod memory;
pub mod session;
pub mod context;
pub mod reasoning;
pub mod execution;
pub mod state;
pub mod deviation;
pub mod checklist;
pub mod heartbeat;
pub mod wasm;
