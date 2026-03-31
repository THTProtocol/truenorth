//! `truenorth serve` — start the Axum + Leptos web server.
//!
//! **v1 stub**: prints what the command would do and exits cleanly.
//! The web server is provided by `truenorth-web` and wired in when the `web`
//! Cargo feature is enabled (not yet in v1).

use anyhow::Result;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Execute the `serve` command.
///
/// # Arguments
///
/// - `port` — TCP port to listen on (default `8080`).
/// - `host` — host/IP to bind to (default `"127.0.0.1"`).
/// - `format` — output format selector.
pub async fn execute(port: u16, host: &str, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth serve");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info(&format!("  Bind address: {host}:{port}"));
            terminal::print_info("");
            terminal::print_info("When wired, this command will:");
            terminal::print_info("  1. Initialise the full component graph (orchestrator, memory, skills, tools)");
            terminal::print_info("  2. Start the Axum HTTP server with Leptos SSR frontend");
            terminal::print_info("  3. Expose REST, SSE, and WebSocket endpoints");
            terminal::print_info("  4. Serve the Visual Reasoning dashboard at http://<host>:<port>/");
        }
        OutputFormat::Json => {
            let data = serde_json::json!({
                "command": "serve",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": {
                    "host": host,
                    "port": port,
                }
            });
            json::print_json(&data);
        }
    }

    Ok(())
}
