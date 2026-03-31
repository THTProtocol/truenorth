//! `truenorth skill` — skill management sub-commands.
//!
//! **v1 stub**: all actions print what they would do.  Full skill management
//! is provided by `truenorth-skills`.

use anyhow::Result;
use clap::Subcommand;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Sub-commands for `truenorth skill`.
#[derive(Debug, Subcommand)]
pub enum SkillAction {
    /// Install a skill from the marketplace or a local path.
    Install {
        /// Skill name or `path:/path/to/skill.wasm`.
        source: String,
    },
    /// List all installed skills.
    List,
    /// Remove an installed skill.
    Remove {
        /// Name of the skill to remove.
        name: String,
    },
}

/// Execute a `skill` sub-command.
///
/// # Arguments
///
/// - `action` — which skill operation to perform.
/// - `format` — output format selector.
pub async fn execute(action: SkillAction, format: OutputFormat) -> Result<()> {
    match action {
        SkillAction::Install { source } => install(&source, format).await,
        SkillAction::List => list(format).await,
        SkillAction::Remove { name } => remove(&name, format).await,
    }
}

/// Install a skill from the given source.
async fn install(source: &str, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth skill install");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info(&format!("  Source: {source}"));
            terminal::print_info("");
            terminal::print_info("When wired, this command will:");
            terminal::print_info("  1. Resolve the skill from the marketplace registry or local path");
            terminal::print_info("  2. Verify the WASM module signature");
            terminal::print_info("  3. Register the skill in the local skill store");
            terminal::print_info("  4. Make the skill available to the agent loop");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "skill install",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": { "source": source }
            }));
        }
    }
    Ok(())
}

/// List all installed skills.
async fn list(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth skill list");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info("When wired, this command will display a table of all installed skills:");
            terminal::print_table(
                &["Name", "Version", "Status", "Description"],
                &[vec![
                    "(no skills installed yet)".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                    "-".to_string(),
                ]],
            );
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "skill list",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "data": { "skills": [] }
            }));
        }
    }
    Ok(())
}

/// Remove a skill by name.
async fn remove(name: &str, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth skill remove");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info(&format!("  Skill: {name}"));
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "skill remove",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": { "name": name }
            }));
        }
    }
    Ok(())
}
