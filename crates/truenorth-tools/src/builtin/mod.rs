//! Built-in tools shipped with TrueNorth.
//!
//! All tools in this module implement the `Tool` trait and are registered by
//! calling [`register_all_builtin_tools`] on a [`crate::DefaultToolRegistry`].
//!
//! # Available tools
//!
//! | Name | Module | Permission |
//! |------|--------|------------|
//! | `search_web` | [`web_search`] | Medium |
//! | `fetch_url` | [`web_fetch`] | Medium |
//! | `read_file` | [`file_read`] | Low |
//! | `write_file` | [`file_write`] | Medium |
//! | `list_files` | [`file_list`] | Low |
//! | `shell_exec` | [`shell_exec`] | High |
//! | `memory_query` | [`memory_query`] | Low |
//! | `render_mermaid` | [`mermaid_render`] | Low |

pub mod file_list;
pub mod file_read;
pub mod file_write;
pub mod memory_query;
pub mod mermaid_render;
pub mod shell_exec;
pub mod web_fetch;
pub mod web_search;

use truenorth_core::traits::tool::{RegistryError, ToolRegistry};

use crate::DefaultToolRegistry;

/// Registers all built-in tools with the provided registry.
///
/// This is the recommended entry point for populating a new registry at
/// application startup. Each tool is registered once; calling this function
/// more than once on the same registry will return a `DuplicateTool` error
/// for the second call.
///
/// # Errors
///
/// Returns [`RegistryError`] if any tool fails to register (e.g., because it
/// was already registered under the same name).
pub fn register_all_builtin_tools(registry: &DefaultToolRegistry) -> Result<(), RegistryError> {
    registry.register(Box::new(web_search::WebSearchTool::new()))?;
    registry.register(Box::new(web_fetch::WebFetchTool::new()))?;
    registry.register(Box::new(file_read::FileReadTool::new()))?;
    registry.register(Box::new(file_write::FileWriteTool::new()))?;
    registry.register(Box::new(file_list::FileListTool::new()))?;
    registry.register(Box::new(shell_exec::ShellExecTool::new()))?;
    registry.register(Box::new(memory_query::MemoryQueryTool::new()))?;
    registry.register(Box::new(mermaid_render::MermaidRenderTool::new()))?;
    Ok(())
}
