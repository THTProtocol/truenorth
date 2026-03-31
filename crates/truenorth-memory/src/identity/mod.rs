//! Identity memory tier — persistent user profile and cross-project preferences.
//!
//! The identity tier stores knowledge about the user that applies across all
//! projects and sessions. It is the most slowly-changing tier. Typical contents:
//!
//! - Communication style preferences ("prefers bullet points")
//! - Workflow patterns ("works in blockchain", "uses TDD")
//! - Inferred user roles (developer, architect, writer, …)
//! - Confirmed preferences updated by the dialectic modeler
//!
//! ## Modules
//!
//! - [`profile`] — `UserProfile`: structured user preferences and patterns.
//! - [`dialectic`] — `HonchoDialecticModeler`: infers patterns and asks nudge questions.
//! - [`sqlite_store`] — `IdentityMemoryStore`: cross-project SQLite persistence.

pub mod dialectic;
pub mod profile;
pub mod sqlite_store;

pub use dialectic::HonchoDialecticModeler;
pub use profile::UserProfile;
pub use sqlite_store::IdentityMemoryStore;
