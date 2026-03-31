//! Output formatting module.
//!
//! Two formatters are provided:
//! - [`terminal`] — coloured, human-readable text via ANSI escape codes.
//! - [`json`] — machine-readable pretty-printed JSON, activated by
//!   `--format json`.
//!
//! Both modules expose a small set of standalone functions; callers pick the
//! right module based on the [`crate::OutputFormat`] flag.

pub mod json;
pub mod terminal;

pub use json::print_json;
pub use terminal::{print_error, print_header, print_info, print_success, print_table};
