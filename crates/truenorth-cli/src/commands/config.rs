//! `truenorth config` — configuration management sub-commands.

use anyhow::Result;
use clap::Subcommand;

use truenorth_orchestrator::OrchestratorConfig;

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
pub async fn execute(action: ConfigAction, format: OutputFormat) -> Result<()> {
    match action {
        ConfigAction::Validate => validate(format).await,
        ConfigAction::Show => show(format).await,
    }
}

/// Validate the configuration.
async fn validate(format: OutputFormat) -> Result<()> {
    let config = OrchestratorConfig::default();
    let mut issues: Vec<String> = Vec::new();

    // Check for API keys
    if std::env::var("ANTHROPIC_API_KEY").unwrap_or_default().is_empty()
        && std::env::var("OPENAI_API_KEY").unwrap_or_default().is_empty()
        && std::env::var("GOOGLE_AI_API_KEY").unwrap_or_default().is_empty()
    {
        issues.push("No LLM API keys found. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or GOOGLE_AI_API_KEY.".to_string());
    }

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth config validate");
            if issues.is_empty() {
                terminal::print_success("Configuration is valid.");
            } else {
                terminal::print_warning(&format!("{} issue(s) found:", issues.len()));
                for issue in &issues {
                    terminal::print_warning(&format!("  - {issue}"));
                }
            }
            terminal::print_info("");
            terminal::print_info(&format!("  Max steps:       {}", config.max_steps));
            terminal::print_info(&format!("  Task timeout:    {}s", config.task_timeout_secs));
            terminal::print_info(&format!("  R/C/S threshold: {}", config.rcs_threshold));
            terminal::print_info(&format!("  Sessions DB:     {}", config.sessions_db_path));
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "valid": issues.is_empty(),
                "issues": issues,
                "config": {
                    "max_steps": config.max_steps,
                    "task_timeout_secs": config.task_timeout_secs,
                    "rcs_threshold": config.rcs_threshold,
                    "sessions_db_path": config.sessions_db_path,
                }
            }));
        }
    }
    Ok(())
}

/// Show the current effective configuration.
async fn show(format: OutputFormat) -> Result<()> {
    let config = OrchestratorConfig::default();

    // Check env vars for API keys (redacted)
    let anthropic = if std::env::var("ANTHROPIC_API_KEY").unwrap_or_default().is_empty() { "not set" } else { "sk-ant-***" };
    let openai = if std::env::var("OPENAI_API_KEY").unwrap_or_default().is_empty() { "not set" } else { "sk-***" };
    let google = if std::env::var("GOOGLE_AI_API_KEY").unwrap_or_default().is_empty() { "not set" } else { "AI***" };
    let auth = if std::env::var("TRUENORTH_AUTH_TOKEN").unwrap_or_default().is_empty() { "disabled" } else { "enabled (***)" };

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth config show");
            terminal::print_info("");
            terminal::print_table(
                &["Key", "Value", "Source"],
                &[
                    vec!["agent.max_steps".into(), config.max_steps.to_string(), "default".into()],
                    vec!["agent.task_timeout_secs".into(), config.task_timeout_secs.to_string(), "default".into()],
                    vec!["agent.rcs_threshold".into(), config.rcs_threshold.to_string(), "default".into()],
                    vec!["agent.max_rcs_iterations".into(), config.max_rcs_iterations.to_string(), "default".into()],
                    vec!["agent.require_plan_approval".into(), config.require_plan_approval.to_string(), "default".into()],
                    vec!["agent.context_budget".into(), config.default_context_budget.to_string(), "default".into()],
                    vec!["sessions.db_path".into(), config.sessions_db_path.clone(), "default".into()],
                    vec!["providers.anthropic".into(), anthropic.into(), "env".into()],
                    vec!["providers.openai".into(), openai.into(), "env".into()],
                    vec!["providers.google".into(), google.into(), "env".into()],
                    vec!["auth.token".into(), auth.into(), "env".into()],
                ],
            );
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "agent": {
                    "max_steps": config.max_steps,
                    "task_timeout_secs": config.task_timeout_secs,
                    "rcs_threshold": config.rcs_threshold,
                    "max_rcs_iterations": config.max_rcs_iterations,
                    "require_plan_approval": config.require_plan_approval,
                    "default_context_budget": config.default_context_budget,
                },
                "sessions": {
                    "db_path": config.sessions_db_path,
                },
                "providers": {
                    "anthropic": anthropic,
                    "openai": openai,
                    "google": google,
                },
                "auth": {
                    "token": auth,
                }
            }));
        }
    }
    Ok(())
}
