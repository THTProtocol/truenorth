//! `truenorth run` — launch the agent loop with an optional task.

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use truenorth_core::types::task::{ExecutionMode, Task, TaskPriority};
use truenorth_orchestrator::{Orchestrator, OrchestratorConfig};

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Execute the `run` command.
///
/// Builds an [`Orchestrator`] with default configuration, creates a [`Task`]
/// from the user's prompt, and runs it through the full agent loop.
pub async fn execute(
    task: Option<String>,
    session_id: Option<String>,
    interactive: bool,
    format: OutputFormat,
) -> Result<()> {
    let prompt = match task {
        Some(t) => t,
        None => {
            match format {
                OutputFormat::Text => {
                    terminal::print_error("No task provided. Use --task \"your prompt here\"");
                }
                OutputFormat::Json => {
                    json::print_json(&serde_json::json!({
                        "error": "No task provided",
                        "usage": "truenorth run --task \"your prompt here\""
                    }));
                }
            }
            return Ok(());
        }
    };

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth run");
            terminal::print_info(&format!("Task: {prompt}"));
            if let Some(ref s) = session_id {
                terminal::print_info(&format!("Session: {s}"));
            }
            if interactive {
                terminal::print_info("Mode: interactive (REPL after task completes)");
            }
            terminal::print_info("");
        }
        OutputFormat::Json => {}
    }

    // Build orchestrator with default config
    let config = OrchestratorConfig::default();
    let orchestrator = Orchestrator::builder()
        .with_config(config)
        .build()?;

    // Create task from prompt
    let task = Task {
        id: Uuid::new_v4(),
        parent_id: None,
        title: prompt.chars().take(80).collect(),
        description: prompt,
        constraints: vec![],
        context_requirements: vec![],
        execution_mode: ExecutionMode::Direct,
        created_at: Utc::now(),
        deadline: None,
        priority: TaskPriority::Normal,
        metadata: serde_json::Value::Null,
    };

    // Run through the agent loop
    match orchestrator.run_task(task).await {
        Ok(result) => {
            match format {
                OutputFormat::Text => {
                    if result.success {
                        terminal::print_success("Task completed successfully");
                    } else {
                        terminal::print_warning("Task completed with issues");
                    }
                    terminal::print_info(&format!("Steps: {}", result.steps_completed));
                    terminal::print_info(&format!("Tokens: {}", result.total_tokens));
                    terminal::print_info(&format!("Duration: {}ms", result.duration_ms));
                    terminal::print_info("");
                    terminal::print_info(&result.output_summary);
                }
                OutputFormat::Json => {
                    json::print_json(&serde_json::json!({
                        "success": result.success,
                        "task_id": result.task_id.to_string(),
                        "steps_completed": result.steps_completed,
                        "total_tokens": result.total_tokens,
                        "duration_ms": result.duration_ms,
                        "output_summary": result.output_summary,
                        "output": result.output,
                    }));
                }
            }
        }
        Err(e) => {
            match format {
                OutputFormat::Text => {
                    terminal::print_error(&format!("Execution failed: {e}"));
                }
                OutputFormat::Json => {
                    json::print_json(&serde_json::json!({
                        "error": format!("{e}"),
                    }));
                }
            }
        }
    }

    Ok(())
}
