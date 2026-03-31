//! TrueNorth — Single-binary, LLM-agnostic AI orchestration harness.
//!
//! This is the main entry point for the `truenorth` binary.
//! It parses CLI arguments, initializes the runtime, and dispatches commands.

use anyhow::Result;
use clap::Parser;
use truenorth_cli::{Cli, run};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}
