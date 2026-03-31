//! File list tool — lists the contents of a directory within the workspace.
//!
//! [`FileListTool`] lists files and subdirectories at a workspace-relative path.
//! Path traversal outside the workspace is rejected. Optionally recurses into
//! subdirectories and applies a glob-style name filter.

use std::path::Path;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::debug;

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, ToolError, ToolResult};

use super::file_read::normalize_path;

/// Metadata for a single directory entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct DirEntry {
    /// Filename (basename only).
    pub name: String,
    /// Path relative to the requested directory.
    pub path: String,
    /// `"file"` or `"dir"`.
    pub kind: String,
    /// File size in bytes (`None` for directories).
    pub size_bytes: Option<u64>,
}

/// Lists directory contents within the workspace.
///
/// # Permission
/// `Low` — read-only, no external side effects.
#[derive(Debug)]
pub struct FileListTool;

impl FileListTool {
    /// Creates a new `FileListTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileListTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolves and validates that `path_str` is within `workspace_root`.
fn resolve_list_path(
    tool_name: &str,
    workspace_root: &Path,
    path_str: &str,
) -> Result<std::path::PathBuf, ToolError> {
    let requested = if Path::new(path_str).is_absolute() {
        Path::new(path_str).to_path_buf()
    } else {
        workspace_root.join(path_str)
    };

    let canonical = if requested.exists() {
        requested
            .canonicalize()
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: tool_name.to_string(),
                message: format!("Failed to canonicalize path: {e}"),
            })?
    } else {
        normalize_path(&requested)
    };

    let workspace_canonical = if workspace_root.exists() {
        workspace_root
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(workspace_root))
    } else {
        normalize_path(workspace_root)
    };

    if !canonical.starts_with(&workspace_canonical) {
        return Err(ToolError::PathTraversal {
            tool_name: tool_name.to_string(),
            path: canonical.to_string_lossy().to_string(),
        });
    }

    Ok(canonical)
}

/// Recursively lists entries under `dir`, appending to `entries`.
/// Limits total entries to `max_entries`.
fn list_dir_recursive(
    base: &Path,
    dir: &Path,
    recursive: bool,
    max_entries: usize,
    entries: &mut Vec<DirEntry>,
) -> Result<(), std::io::Error> {
    let read_dir = std::fs::read_dir(dir)?;
    for entry in read_dir {
        if entries.len() >= max_entries {
            break;
        }
        let entry = entry?;
        let entry_path = entry.path();
        let relative = entry_path
            .strip_prefix(base)
            .unwrap_or(&entry_path)
            .to_string_lossy()
            .to_string();

        let metadata = entry.metadata()?;
        let kind = if metadata.is_dir() { "dir" } else { "file" };
        let size_bytes = if metadata.is_file() {
            Some(metadata.len())
        } else {
            None
        };
        let name = entry
            .file_name()
            .to_string_lossy()
            .to_string();

        entries.push(DirEntry {
            name,
            path: relative,
            kind: kind.to_string(),
            size_bytes,
        });

        if recursive && metadata.is_dir() {
            list_dir_recursive(base, &entry_path, true, max_entries, entries)?;
        }
    }
    Ok(())
}

#[async_trait]
impl Tool for FileListTool {
    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "List files and directories within the workspace. Returns a structured list \
         with names, types, and sizes. Optionally recurse into subdirectories."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to list. Relative to workspace root. \
                                    Defaults to the workspace root itself.",
                    "default": "."
                },
                "recursive": {
                    "type": "boolean",
                    "description": "If true, list all files recursively. Default: false.",
                    "default": false
                },
                "max_entries": {
                    "type": "integer",
                    "description": "Maximum number of entries to return (default 200, max 2000).",
                    "minimum": 1,
                    "maximum": 2000,
                    "default": 200
                }
            },
            "required": []
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Low
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"path": "src", "recursive": true}"#)
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        let path_str = args["path"].as_str().unwrap_or(".");
        let recursive = args["recursive"].as_bool().unwrap_or(false);
        let max_entries = args["max_entries"]
            .as_u64()
            .map(|v| v.clamp(1, 2000) as usize)
            .unwrap_or(200);

        debug!(path = path_str, recursive, "Listing directory");

        // --- Resolve and validate ---
        let abs_path =
            resolve_list_path(self.name(), &context.workspace_root, path_str)?;

        if !abs_path.exists() {
            return Err(ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Directory not found: {}", abs_path.display()),
            });
        }

        if !abs_path.is_dir() {
            return Err(ToolError::InvalidArguments {
                tool_name: self.name().to_string(),
                message: format!("'{}' is a file, not a directory", path_str),
            });
        }

        // --- List ---
        let mut entries: Vec<DirEntry> = Vec::new();
        list_dir_recursive(&abs_path, &abs_path, recursive, max_entries, &mut entries)
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to list directory: {e}"),
            })?;

        // Sort: directories first, then files, alphabetically within each.
        entries.sort_by(|a, b| {
            let kind_cmp = b.kind.cmp(&a.kind); // "file" < "dir"  -> dirs first
            if kind_cmp == std::cmp::Ordering::Equal {
                a.path.cmp(&b.path)
            } else {
                kind_cmp
            }
        });

        let truncated = entries.len() >= max_entries;
        let execution_ms = start.elapsed().as_millis() as u64;

        let llm_output = json!({
            "path": path_str,
            "entry_count": entries.len(),
            "truncated": truncated,
            "entries": entries
        });

        let display_output = json!({
            "type": "dir_listing",
            "path": path_str,
            "entries": entries,
            "truncated": truncated
        });

        Ok(ToolResult {
            llm_output,
            display_output: Some(display_output),
            side_effects: vec![],
            execution_ms,
        })
    }
}
