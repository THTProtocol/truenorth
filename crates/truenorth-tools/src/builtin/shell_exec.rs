//! Shell execution tool — runs a shell command via `tokio::process::Command`.
//!
//! [`ShellExecTool`] is the highest-permission built-in tool. It executes an
//! arbitrary shell command (via `/bin/sh -c`) with a configurable timeout and
//! captures stdout, stderr, and the exit code.
//!
//! # Safety
//!
//! This tool requires `PermissionLevel::High`. It should only be granted in
//! explicitly trusted contexts. The working directory is always set to the
//! workspace root.

use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, SideEffect, ToolError, ToolResult};

/// Default timeout for shell commands in milliseconds (30 seconds).
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Executes shell commands within the workspace.
///
/// # Permission
/// `High` — can run arbitrary code. Always requires explicit user approval
/// in step-wise mode.
#[derive(Debug)]
pub struct ShellExecTool;

impl ShellExecTool {
    /// Creates a new `ShellExecTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ShellExecTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the workspace directory. Captures stdout, stderr, \
         and the exit code. Use this for running build tools, tests, scripts, and \
         other system commands. Requires High permission. Has a configurable timeout."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute (passed to /bin/sh -c)."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Execution timeout in milliseconds (default 30000 = 30s, max 300000 = 5min).",
                    "minimum": 1000,
                    "maximum": 300000,
                    "default": 30000
                },
                "env": {
                    "type": "object",
                    "description": "Additional environment variables to set for the command.",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::High
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"command": "cargo test --workspace 2>&1", "timeout_ms": 60000}"#)
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let command_str = args["command"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'command'".to_string(),
        })?;

        let timeout_ms = args["timeout_ms"]
            .as_u64()
            .map(|v| v.clamp(1000, 300_000))
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        // --- Dry-run mode ---
        if context.dry_run {
            info!(command = command_str, "[dry-run] Would execute shell command");
            let execution_ms = start.elapsed().as_millis() as u64;
            return Ok(ToolResult {
                llm_output: json!({
                    "command": command_str,
                    "dry_run": true,
                    "stdout": "",
                    "stderr": "",
                    "exit_code": 0
                }),
                display_output: Some(json!({
                    "type": "shell_exec",
                    "command": command_str,
                    "dry_run": true
                })),
                side_effects: vec![SideEffect::ShellCommandExecuted {
                    command: command_str.to_string(),
                    exit_code: 0,
                }],
                execution_ms,
            });
        }

        // --- Build command ---
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(command_str)
            .current_dir(&context.workspace_root)
            .kill_on_drop(true);

        // Inject additional environment variables.
        if let Some(env_obj) = args["env"].as_object() {
            for (k, v) in env_obj {
                if let Some(val) = v.as_str() {
                    cmd.env(k, val);
                }
            }
        }

        info!(
            command = command_str,
            workspace = %context.workspace_root.display(),
            timeout_ms,
            "Executing shell command"
        );

        // --- Execute with timeout ---
        let output_result = timeout(
            Duration::from_millis(timeout_ms),
            cmd.output(),
        )
        .await;

        let output = match output_result {
            Err(_elapsed) => {
                warn!(command = command_str, timeout_ms, "Shell command timed out");
                return Err(ToolError::ExecutionTimeout {
                    tool_name: self.name().to_string(),
                    timeout_ms,
                });
            }
            Ok(Err(io_err)) => {
                return Err(ToolError::ExecutionFailed {
                    tool_name: self.name().to_string(),
                    message: format!("Failed to spawn process: {io_err}"),
                });
            }
            Ok(Ok(out)) => out,
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        let execution_ms = start.elapsed().as_millis() as u64;

        info!(
            command = command_str,
            exit_code,
            stdout_bytes = stdout.len(),
            stderr_bytes = stderr.len(),
            execution_ms,
            "Shell command completed"
        );

        let success = output.status.success();

        let llm_output = json!({
            "command": command_str,
            "exit_code": exit_code,
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "execution_ms": execution_ms
        });

        let display_output = json!({
            "type": "shell_exec",
            "command": command_str,
            "exit_code": exit_code,
            "success": success,
            "stdout": stdout,
            "stderr": stderr
        });

        Ok(ToolResult {
            llm_output,
            display_output: Some(display_output),
            side_effects: vec![SideEffect::ShellCommandExecuted {
                command: command_str.to_string(),
                exit_code,
            }],
            execution_ms,
        })
    }
}
