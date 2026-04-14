//! `truenorth serve` — start the Axum web server.

use anyhow::Result;

use truenorth_orchestrator::{Orchestrator, OrchestratorConfig};
use truenorth_web::server::state::AppState;
use truenorth_web::WebServer;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Execute the `serve` command.
///
/// Builds an [`Orchestrator`], wraps it in [`AppState`], and starts
/// the Axum HTTP server with REST, SSE, and WebSocket endpoints.
pub async fn execute(port: u16, host: &str, format: OutputFormat) -> Result<()> {
    let bind_addr = format!("{host}:{port}");

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth serve");
            terminal::print_info(&format!("Binding to {bind_addr}"));
            terminal::print_info("Building orchestrator...");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "serve",
                "status": "starting",
                "bind": bind_addr,
            }));
        }
    }

    // Build orchestrator
    let config = OrchestratorConfig::default();
    let _orchestrator = Orchestrator::builder()
        .with_config(config)
        .build()?;

    // Build AppState with auth token from env if present
    let mut state_builder = AppState::builder();
    if let Ok(token) = std::env::var("TRUENORTH_AUTH_TOKEN") {
        if !token.is_empty() {
            state_builder = state_builder.with_auth_token(token);
        }
    }
    let state = state_builder.build();

    match format {
        OutputFormat::Text => {
            terminal::print_success(&format!("Server starting on http://{bind_addr}"));
            terminal::print_info("Press Ctrl+C to stop");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "serve",
                "status": "listening",
                "url": format!("http://{bind_addr}"),
            }));
        }
    }

    // Start serving — blocks until shutdown
    WebServer::new(state)
        .bind(&bind_addr)
        .serve()
        .await?;

    Ok(())
}
