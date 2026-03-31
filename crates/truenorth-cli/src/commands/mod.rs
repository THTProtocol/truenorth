//! Command module root.
//!
//! Defines the [`Commands`] enum (used by clap as the top-level subcommand),
//! the sub-action enums for nested subcommands, and the [`dispatch`] function
//! that routes a parsed command to the correct handler module.

pub mod config;
pub mod memory;
pub mod resume;
pub mod run;
pub mod serve;
pub mod skill;
pub mod version;

use anyhow::Result;
use clap::Subcommand;

use crate::OutputFormat;

// Re-export sub-action enums so callers (e.g. tests in lib.rs) can refer to
// them without a deep module path.
pub use config::ConfigAction;
pub use memory::MemoryAction;
pub use skill::SkillAction;

/// All top-level subcommands supported by `truenorth`.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Launch an agent loop with an optional initial task.
    ///
    /// In v1 this prints a placeholder message.  Full orchestrator wiring
    /// arrives in Wave 5.
    Run {
        /// Initial task description to pass to the agent.
        #[arg(long)]
        task: Option<String>,

        /// Resume or continue an existing session by ID.
        #[arg(long)]
        session_id: Option<String>,

        /// Start an interactive REPL-style session.
        #[arg(long, default_value_t = false)]
        interactive: bool,
    },

    /// Start the TrueNorth web server (Axum + Leptos).
    ///
    /// In v1 this prints a placeholder message.  The web server is wired in
    /// via the `truenorth-web` crate when the `web` feature is enabled.
    Serve {
        /// TCP port to listen on.
        #[arg(long, default_value_t = 8080)]
        port: u16,

        /// Host/IP address to bind to.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Resume a previously saved agent session.
    Resume {
        /// The session UUID to restore.
        session_id: String,
    },

    /// Skill management commands.
    Skill {
        /// Skill sub-command.
        #[command(subcommand)]
        action: SkillAction,
    },

    /// Memory inspection and maintenance commands.
    Memory {
        /// Memory sub-command.
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Configuration management commands.
    Config {
        /// Config sub-command.
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Print version and build information.
    Version,
}

/// Dispatch a parsed [`Commands`] variant to the correct handler.
///
/// Each handler receives the [`OutputFormat`] so it can choose between
/// human-readable text and machine-readable JSON.
pub async fn dispatch(command: Commands, format: OutputFormat) -> Result<()> {
    match command {
        Commands::Run {
            task,
            session_id,
            interactive,
        } => run::execute(task, session_id, interactive, format).await,

        Commands::Serve { port, host } => serve::execute(port, &host, format).await,

        Commands::Resume { session_id } => resume::execute(&session_id, format).await,

        Commands::Skill { action } => skill::execute(action, format).await,

        Commands::Memory { action } => memory::execute(action, format).await,

        Commands::Config { action } => config::execute(action, format).await,

        Commands::Version => version::execute(format).await,
    }
}
