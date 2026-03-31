//! # truenorth-cli
//!
//! Command-line interface for the TrueNorth AI orchestration harness.
//!
//! This crate provides the [`Cli`] struct (parsed by [`clap`]) and a top-level
//! [`run`] function that dispatches to per-command handlers in the
//! [`commands`] module.
//!
//! In v1 all commands are **placeholder stubs** that print what they would do.
//! Heavy orchestrator wiring is deferred to Wave 5.

pub mod commands;
pub mod init;
pub mod output;

use anyhow::Result;
use clap::{Parser, ValueEnum};

use commands::Commands;

/// Top-level CLI structure parsed by [`clap`].
///
/// Invoke with `truenorth <command> [options]`.
#[derive(Debug, Parser)]
#[command(
    name = "truenorth",
    about = "TrueNorth: LLM-agnostic AI orchestration harness",
    version,
    long_about = "Single-binary, locally-hosted AI orchestrator with visual reasoning, \
                  multi-provider LLM routing, and WASM-sandboxed skills."
)]
pub struct Cli {
    /// Sub-command to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Output format (text or json).
    #[arg(long, default_value = "text", global = true)]
    pub format: OutputFormat,

    /// Verbosity level — pass up to three times for more detail (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Path to the configuration file.
    #[arg(long, default_value = "config.toml", global = true)]
    pub config: String,
}

/// Output format selector, driven by the `--format` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable coloured text (default).
    Text,
    /// Machine-readable JSON, suitable for scripting.
    Json,
}

/// Library entry point: parse args then dispatch to the appropriate command
/// handler.
///
/// Call this from `main` with the already-parsed [`Cli`]:
///
/// ```no_run
/// use truenorth_cli::{Cli, run};
/// use clap::Parser;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let cli = Cli::parse();
///     run(cli).await
/// }
/// ```
pub async fn run(cli: Cli) -> Result<()> {
    // Initialise tracing as the very first step.
    init::init_tracing(cli.verbose);

    // Load configuration (best-effort; falls back to defaults on error).
    let _config = init::load_config(&cli.config);

    // Dispatch.
    commands::dispatch(cli.command, cli.format).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Helper: parse a slice of string args into [`Cli`].
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn test_version_subcommand() {
        let cli = parse(&["truenorth", "version"]).unwrap();
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn test_run_with_task() {
        let cli = parse(&["truenorth", "run", "--task", "do something"]).unwrap();
        match cli.command {
            Commands::Run { task, .. } => assert_eq!(task.as_deref(), Some("do something")),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_run_with_session_id() {
        let cli = parse(&["truenorth", "run", "--session-id", "abc-123"]).unwrap();
        match cli.command {
            Commands::Run { session_id, .. } => {
                assert_eq!(session_id.as_deref(), Some("abc-123"))
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_run_interactive_flag() {
        let cli = parse(&["truenorth", "run", "--interactive"]).unwrap();
        match cli.command {
            Commands::Run { interactive, .. } => assert!(interactive),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_serve_default_port() {
        let cli = parse(&["truenorth", "serve"]).unwrap();
        match cli.command {
            Commands::Serve { port, .. } => assert_eq!(port, 8080),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_serve_custom_port() {
        let cli = parse(&["truenorth", "serve", "--port", "9090"]).unwrap();
        match cli.command {
            Commands::Serve { port, .. } => assert_eq!(port, 9090),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_resume_session_id() {
        let cli = parse(&["truenorth", "resume", "sess-xyz"]).unwrap();
        match cli.command {
            Commands::Resume { session_id } => assert_eq!(session_id, "sess-xyz"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_json_output_format() {
        let cli = parse(&["truenorth", "--format", "json", "version"]).unwrap();
        assert_eq!(cli.format, OutputFormat::Json);
    }

    #[test]
    fn test_verbosity_count() {
        let cli = parse(&["truenorth", "-vvv", "version"]).unwrap();
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn test_config_flag() {
        let cli = parse(&["truenorth", "--config", "/etc/truenorth.toml", "version"]).unwrap();
        assert_eq!(cli.config, "/etc/truenorth.toml");
    }

    #[test]
    fn test_skill_list() {
        let cli = parse(&["truenorth", "skill", "list"]).unwrap();
        match cli.command {
            Commands::Skill { action } => {
                assert!(matches!(action, commands::SkillAction::List))
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_memory_query() {
        let cli = parse(&["truenorth", "memory", "query", "--query", "test"]).unwrap();
        match cli.command {
            Commands::Memory { action } => {
                assert!(matches!(action, commands::MemoryAction::Query { .. }))
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn test_config_show() {
        let cli = parse(&["truenorth", "config", "show"]).unwrap();
        match cli.command {
            Commands::Config { action } => {
                assert!(matches!(action, commands::ConfigAction::Show))
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
