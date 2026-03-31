//! Session memory tier — ephemeral in-memory storage for the active conversation.
//!
//! The session tier is the fastest and most transient tier. It stores the current
//! conversation history, tool call results, and scratchpad state entirely in RAM
//! using `Arc<RwLock<HashMap>>`. On session end, the contents are optionally
//! persisted to SQLite for the consolidation pipeline to review.
//!
//! ## Modules
//!
//! - [`store`] — `SessionMemoryStore`: the main in-memory storage struct.
//! - [`compactor`] — `ContextCompactor`: LLM-driven conversation summarization.

pub mod compactor;
pub mod store;

pub use compactor::ContextCompactor;
pub use store::SessionMemoryStore;
