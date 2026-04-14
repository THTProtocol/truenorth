//! `truenorth resume` — resume a previously saved session.

use anyhow::Result;
use uuid::Uuid;

use truenorth_core::traits::session::SessionManager;
use truenorth_orchestrator::{Orchestrator, OrchestratorConfig};

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Execute the `resume` command.
///
/// Builds an [`Orchestrator`] and attempts to load the session identified by
/// `session_id` from the persistent session store.
pub async fn execute(session_id: &str, format: OutputFormat) -> Result<()> {
    let sid = session_id.parse::<Uuid>().map_err(|e| {
        anyhow::anyhow!("Invalid session ID '{session_id}': {e}")
    })?;

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth resume");
            terminal::print_info(&format!("Session: {sid}"));
            terminal::print_info("Building orchestrator...");
        }
        OutputFormat::Json => {}
    }

    let config = OrchestratorConfig::default();
    let orchestrator = Orchestrator::builder()
        .with_config(config)
        .build()?;

    // Attempt to load the session
    match orchestrator.session_manager.resume(sid).await {
        Ok(state) => {
            match format {
                OutputFormat::Text => {
                    terminal::print_success(&format!("Session loaded: {}", state.title));
                    terminal::print_info(&format!("  State: {}", state.agent_state));
                    terminal::print_info(&format!("  Created: {}", state.created_at));
                    terminal::print_info(&format!("  Context tokens: {}", state.context_tokens));
                    terminal::print_info("");
                    terminal::print_info("Session resume is not yet fully wired — the loaded state is displayed above.");
                    terminal::print_info("Full resume (re-entering the agent loop) will be available in the next release.");
                }
                OutputFormat::Json => {
                    json::print_json(&serde_json::json!({
                        "session_id": sid.to_string(),
                        "title": state.title,
                        "agent_state": state.agent_state,
                        "created_at": state.created_at.to_rfc3339(),
                        "context_tokens": state.context_tokens,
                        "status": "loaded",
                        "note": "Full resume not yet wired — session state loaded successfully."
                    }));
                }
            }
        }
        Err(e) => {
            match format {
                OutputFormat::Text => {
                    terminal::print_error(&format!("Failed to load session: {e}"));
                }
                OutputFormat::Json => {
                    json::print_json(&serde_json::json!({
                        "error": format!("Failed to load session: {e}"),
                    }));
                }
            }
        }
    }

    Ok(())
}
