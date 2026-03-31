//! File read tool — reads the contents of a file within the workspace.
//!
//! [`FileReadTool`] reads a file at a path relative to (or within) the
//! workspace root defined in the [`ToolContext`]. Path traversal outside the
//! workspace is rejected.

use std::path::Path;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::debug;

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, ToolError, ToolResult};

/// Maximum file size to read in bytes (1 MiB).
const MAX_FILE_SIZE_BYTES: u64 = 1024 * 1024;

/// Reads the content of a file within the workspace root.
///
/// All paths are validated against `context.workspace_root`. Reading outside
/// the workspace returns a [`ToolError::PathTraversal`] error.
///
/// # Permission
/// `Low` — read-only, no external side effects.
#[derive(Debug)]
pub struct FileReadTool;

impl FileReadTool {
    /// Creates a new `FileReadTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolves `path` relative to `workspace_root` and validates it stays within bounds.
///
/// Returns the absolute path if valid, or an error if path traversal is detected.
fn resolve_and_validate(
    tool_name: &str,
    workspace_root: &Path,
    path_str: &str,
) -> Result<std::path::PathBuf, ToolError> {
    let requested = if Path::new(path_str).is_absolute() {
        Path::new(path_str).to_path_buf()
    } else {
        workspace_root.join(path_str)
    };

    // Canonicalise to resolve symlinks and `..` components.
    // If the path doesn't exist yet we normalise manually.
    let canonical = if requested.exists() {
        requested
            .canonicalize()
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: tool_name.to_string(),
                message: format!("Failed to canonicalize path: {e}"),
            })?
    } else {
        // Path doesn't exist — do a lexicographic normalisation.
        normalize_path(&requested)
    };

    let workspace_canonical = if workspace_root.exists() {
        workspace_root
            .canonicalize()
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: tool_name.to_string(),
                message: format!("Failed to canonicalize workspace root: {e}"),
            })?
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

/// Lexicographically normalises a path (resolves `.` and `..` without hitting the filesystem).
pub(crate) fn normalize_path(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                components.pop();
            }
            Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the content of a file within the workspace. Supports reading text files, \
         code, configuration files, and Markdown. Returns the file content as a string. \
         Paths are relative to the workspace root."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read. Relative to workspace root, \
                                    or absolute (must still be within the workspace)."
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes to read (default 1048576 = 1 MiB).",
                    "minimum": 1,
                    "maximum": 10485760
                }
            },
            "required": ["path"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Low
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"path": "src/main.rs"}"#)
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let path_str = args["path"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'path'".to_string(),
        })?;

        let max_bytes = args["max_bytes"]
            .as_u64()
            .map(|v| v.min(10 * 1024 * 1024))
            .unwrap_or(MAX_FILE_SIZE_BYTES);

        debug!(path = path_str, "Reading file");

        // --- Resolve and validate path ---
        let abs_path = resolve_and_validate(self.name(), &context.workspace_root, path_str)?;

        // --- Check file exists ---
        if !abs_path.exists() {
            return Err(ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("File not found: {}", abs_path.display()),
            });
        }

        // --- Check file size ---
        let metadata = std::fs::metadata(&abs_path).map_err(|e| ToolError::ExecutionFailed {
            tool_name: self.name().to_string(),
            message: format!("Failed to read file metadata: {e}"),
        })?;

        let file_size = metadata.len();

        if file_size > max_bytes {
            // Read only up to max_bytes.
            let mut f = std::fs::File::open(&abs_path).map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to open file: {e}"),
            })?;
            use std::io::Read;
            let mut buf = vec![0u8; max_bytes as usize];
            let read_bytes = f.read(&mut buf).map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to read file: {e}"),
            })?;
            buf.truncate(read_bytes);
            let content = String::from_utf8_lossy(&buf).into_owned();

            let execution_ms = start.elapsed().as_millis() as u64;
            return Ok(ToolResult {
                llm_output: json!({
                    "path": path_str,
                    "content": content,
                    "bytes_read": read_bytes,
                    "file_size": file_size,
                    "truncated": true
                }),
                display_output: Some(json!({
                    "type": "file_content",
                    "path": path_str,
                    "content": content,
                    "truncated": true
                })),
                side_effects: vec![],
                execution_ms,
            });
        }

        // --- Read entire file ---
        let content = std::fs::read_to_string(&abs_path).map_err(|e| ToolError::ExecutionFailed {
            tool_name: self.name().to_string(),
            message: format!("Failed to read file as UTF-8: {e}"),
        })?;

        let execution_ms = start.elapsed().as_millis() as u64;

        Ok(ToolResult {
            llm_output: json!({
                "path": path_str,
                "content": content,
                "bytes_read": content.len(),
                "file_size": file_size,
                "truncated": false
            }),
            display_output: Some(json!({
                "type": "file_content",
                "path": path_str,
                "content": content,
                "truncated": false
            })),
            side_effects: vec![],
            execution_ms,
        })
    }
}
