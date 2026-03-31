//! `truenorth resume <session_id>` — restore a saved agent session.
//!
//! **v1 stub**: prints what the command would do and exits cleanly.
//! Session persistence is implemented in `truenorth-orchestrator`.

use anyhow::Result;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Execute the `resume` command.
///
/// # Arguments
///
/// - `session_id` — the session UUID to restore.
/// - `format` — output format selector.
pub async fn execute(session_id: &str, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth resume");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info(&format!("  Session ID: {session_id}"));
            terminal::print_info("");
            terminal::print_info("When wired, this command will:");
            terminal::print_info("  1. Locate the serialised SessionState in the memory store");
            terminal::print_info("  2. Deserialise and validate the session snapshot");
            terminal::print_info("  3. Re-attach the agent loop to the restored session");
            terminal::print_info("  4. Continue from the last checkpoint");
        }
        OutputFormat::Json => {
            let data = serde_json::json!({
                "command": "resume",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": {
                    "session_id": session_id,
                }
            });
            json::print_json(&data);
        }
    }

    Ok(())
}
