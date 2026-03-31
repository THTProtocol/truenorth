//! `truenorth config` — configuration management sub-commands.
//!
//! **v1 stub**: all actions print what they would do.

use anyhow::Result;
use clap::Subcommand;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Sub-commands for `truenorth config`.
#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Validate the configuration file for correctness.
    Validate,
    /// Display the current effective configuration.
    Show,
}

/// Execute a `config` sub-command.
///
/// # Arguments
///
/// - `action` — which config operation to perform.
/// - `format` — output format selector.
pub async fn execute(action: ConfigAction, format: OutputFormat) -> Result<()> {
    match action {
        ConfigAction::Validate => validate(format).await,
        ConfigAction::Show => show(format).await,
    }
}

/// Validate the configuration file.
async fn validate(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth config validate");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info("When wired, this command will:");
            terminal::print_info("  1. Load config.toml from the configured path");
            terminal::print_info("  2. Validate all required fields are present");
            terminal::print_info("  3. Check provider API key environment variables");
            terminal::print_info("  4. Report any errors or warnings");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "config validate",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release."
            }));
        }
    }
    Ok(())
}

/// Show the current effective configuration.
async fn show(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth config show");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info("When wired, this command will display the merged effective config");
            terminal::print_info("(file + env overrides) with secrets redacted.");
            terminal::print_table(
                &["Key", "Value", "Source"],
                &[
                    vec![
                        "log_level".to_string(),
                        "info".to_string(),
                        "default".to_string(),
                    ],
                    vec![
                        "orchestrator.max_steps".to_string(),
                        "50".to_string(),
                        "default".to_string(),
                    ],
                    vec![
                        "memory.backend".to_string(),
                        "sqlite".to_string(),
                        "default".to_string(),
                    ],
                ],
            );
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "config show",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "data": {
                    "log_level": "info",
                    "orchestrator": { "max_steps": 50 },
                    "memory": { "backend": "sqlite" }
                }
            }));
        }
    }
    Ok(())
}
