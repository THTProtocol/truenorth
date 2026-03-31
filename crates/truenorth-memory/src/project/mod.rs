//! Project memory tier — persistent SQLite + Obsidian Markdown storage.
//!
//! The project tier stores knowledge that must survive across sessions but is
//! scoped to a single project workspace. Typical contents include:
//!
//! - Codebase decisions and architectural notes
//! - Error patterns and their resolutions
//! - Domain knowledge specific to the project
//! - References to external resources
//!
//! ## Modules
//!
//! - [`sqlite_store`] — `ProjectMemoryStore`: full CRUD with WAL-mode SQLite.
//! - [`markdown_writer`] — `MarkdownWriter`: converts entries to Obsidian Markdown.
//! - [`deduplicator`] — `Deduplicator`: semantic similarity dedup before writes.

pub mod deduplicator;
pub mod markdown_writer;
pub mod sqlite_store;

pub use deduplicator::Deduplicator;
pub use markdown_writer::MarkdownWriter;
pub use sqlite_store::ProjectMemoryStore;
