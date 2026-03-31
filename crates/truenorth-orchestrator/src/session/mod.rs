//! Session management module.
//!
//! Provides session lifecycle management (create, save, resume, delete),
//! handoff document generation, and state serialization to SQLite + JSON.

pub mod handoff;
pub mod manager;
pub mod serializer;
