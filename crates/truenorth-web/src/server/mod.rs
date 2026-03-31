//! Server module root.
//!
//! Re-exports the major server building blocks for use in [`crate::lib`].

pub mod errors;
pub mod handlers;
pub mod middleware;
pub mod router;
pub mod state;

pub use router::build_router;
pub use state::{AppState, AppStateBuilder, SessionInfo};
