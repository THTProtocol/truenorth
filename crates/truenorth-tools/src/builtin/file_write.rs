//! File write tool — writes content to a file within the workspace.
//!
//! [`FileWriteTool`] creates or overwrites a file at a workspace-relative path.
//! Every write operation creates an audit log entry recording the path, byte
//! count, and timestamp. Path traversal outside the workspace is rejected.

use std::path::Path;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{info, warn};

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, SideEffect, ToolError, ToolResult};

use super::file_read::normalize_path;

/// Writes content to a file within the workspace root.
///
/// Creates parent directories if they do not exist. Creates an audit log
/// entry for every write.
///
/// # Permission
/// `Medium` — creates or modifies files on the filesystem.
#[derive(Debug)]
pub struct FileWriteTool;

impl FileWriteTool {
    /// Creates a new `FileWriteTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolves `path_str` relative to `workspace_root` and ensures it stays within bounds.
fn resolve_write_path(
    tool_name: &str,
    workspace_root: &Path,
    path_str: &str,
) -> Result<std::path::PathBuf, ToolError> {
    let requested = if Path::new(path_str).is_absolute() {
        Path::new(path_str).to_path_buf()
    } else {
        workspace_root.join(path_str)
    };

    let canonical = normalize_path(&requested);

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

/// Writes an audit log entry to `<workspace_root>/.truenorth/audit.log`.
fn write_audit_entry(workspace_root: &Path, entry: &str) {
    let audit_dir = workspace_root.join(".truenorth");
    if let Err(e) = std::fs::create_dir_all(&audit_dir) {
        warn!(error = %e, "Failed to create audit log directory");
        return;
    }
    let audit_path = audit_dir.join("audit.log");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&audit_path)
    {
        let _ = writeln!(f, "{}", entry);
    } else {
        warn!("Failed to open audit log for writing");
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write text content to a file within the workspace. Creates the file if it \
         does not exist; overwrites it if it does (unless append mode is chosen). \
         Creates parent directories automatically. Every write is audit logged."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write. Relative to workspace root."
                },
                "content": {
                    "type": "string",
                    "description": "The text content to write."
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to the file instead of overwriting. Default: false.",
                    "default": false
                }
            },
            "required": ["path", "content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Medium
    }

    fn usage_example(&self) -> Option<&str> {
        Some("{\"path\": \"output/report.md\", \"content\": \"## Report\\n\\nContent here.\"}")
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let path_str = args["path"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'path'".to_string(),
        })?;

        let content = args["content"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'content'".to_string(),
        })?;

        let append = args["append"].as_bool().unwrap_or(false);

        // --- Resolve and validate path ---
        let abs_path =
            resolve_write_path(self.name(), &context.workspace_root, path_str)?;

        // --- Create parent directories ---
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to create parent directories: {e}"),
            })?;
        }

        // --- Dry-run mode: only log, don't write ---
        if context.dry_run {
            info!(
                path = path_str,
                bytes = content.len(),
                append,
                "[dry-run] Would write file"
            );
            let execution_ms = start.elapsed().as_millis() as u64;
            return Ok(ToolResult {
                llm_output: json!({
                    "path": path_str,
                    "bytes_written": content.len(),
                    "append": append,
                    "dry_run": true
                }),
                display_output: Some(json!({
                    "type": "file_write",
                    "path": path_str,
                    "dry_run": true
                })),
                side_effects: vec![SideEffect::FileWritten {
                    path: path_str.to_string(),
                    bytes: content.len(),
                }],
                execution_ms,
            });
        }

        // --- Write file ---
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(!append)
            .append(append)
            .open(&abs_path)
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to open file for writing: {e}"),
            })?;

        file.write_all(content.as_bytes())
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: self.name().to_string(),
                message: format!("Failed to write file content: {e}"),
            })?;

        let bytes_written = content.len();

        info!(
            path = %abs_path.display(),
            bytes = bytes_written,
            append,
            "File written"
        );

        // --- Write audit log entry ---
        let audit_entry = format!(
            "[{}] write_file path={} bytes={} append={} session={}",
            chrono::Utc::now().to_rfc3339(),
            abs_path.display(),
            bytes_written,
            append,
            context.session_id
        );
        write_audit_entry(&context.workspace_root, &audit_entry);

        let execution_ms = start.elapsed().as_millis() as u64;

        Ok(ToolResult {
            llm_output: json!({
                "path": path_str,
                "bytes_written": bytes_written,
                "append": append,
                "success": true
            }),
            display_output: Some(json!({
                "type": "file_write",
                "path": path_str,
                "bytes_written": bytes_written
            })),
            side_effects: vec![SideEffect::FileWritten {
                path: path_str.to_string(),
                bytes: bytes_written,
            }],
            execution_ms,
        })
    }
}
