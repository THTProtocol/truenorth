//! Obsidian vault sync subsystem — bidirectional sync with Obsidian Markdown files.
//!
//! TrueNorth writes memory entries as Obsidian-compatible Markdown files so users
//! can view, annotate, and link their memory in Obsidian. Changes made in Obsidian
//! are detected by the `ObsidianWatcher` and re-indexed by the `Reindexer`.
//!
//! ## Bidirectional sync flow
//!
//! ```text
//! TrueNorth writes entry
//!   → MarkdownWriter writes .md file
//!   → Obsidian reads .md file (user can annotate)
//!   → User edits .md file in Obsidian
//!   → ObsidianWatcher detects change (notify)
//!   → Reindexer parses changed .md, re-embeds, updates SQLite + Tantivy
//! ```
//!
//! ## Modules
//!
//! - [`watcher`] — `ObsidianWatcher`: filesystem event watcher with debounce.
//! - [`reindexer`] — `Reindexer`: parses changed Markdown and updates stores.
//! - [`wikilink`] — `WikilinkParser`: `[[wikilink]]` parsing and link graph.

pub mod reindexer;
pub mod watcher;
pub mod wikilink;

pub use reindexer::Reindexer;
pub use watcher::ObsidianWatcher;
pub use wikilink::WikilinkParser;
