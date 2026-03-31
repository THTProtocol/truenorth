//! `truenorth run` — launch the agent loop.
//!
//! **v1 stub**: prints what the command would do and exits cleanly.
//! Full orchestrator wiring is added in Wave 5.

use anyhow::Result;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Execute the `run` command.
///
/// # Arguments
///
/// - `task` — optional initial task description.
/// - `session_id` — optional existing session to continue.
/// - `interactive` — if `true`, the agent would enter a REPL loop.
/// - `format` — output format selector.
pub async fn execute(
    task: Option<String>,
    session_id: Option<String>,
    interactive: bool,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth run");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");

            if let Some(ref t) = task {
                terminal::print_info(&format!("  Task       : {t}"));
            }
            if let Some(ref s) = session_id {
                terminal::print_info(&format!("  Session ID : {s}"));
            }
            terminal::print_info(&format!("  Interactive: {interactive}"));
            terminal::print_info("");
            terminal::print_info("When wired, this command will:");
            terminal::print_info("  1. Load configuration and initialise component graph");
            terminal::print_info("  2. Create (or resume) a session via SessionManager");
            terminal::print_info("  3. Submit the task to the AgentLoop for planning + execution");
            terminal::print_info("  4. Stream ReasoningEvents to the terminal in real time");
        }
        OutputFormat::Json => {
            let data = serde_json::json!({
                "command": "run",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": {
                    "task": task,
                    "session_id": session_id,
                    "interactive": interactive,
                }
            });
            json::print_json(&data);
        }
    }

    Ok(())
}
