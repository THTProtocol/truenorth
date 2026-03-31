//! `truenorth memory` — memory inspection and maintenance sub-commands.
//!
//! **v1 stub**: all actions print what they would do.  Full memory management
//! is provided by `truenorth-memory`.

use anyhow::Result;
use clap::Subcommand;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Sub-commands for `truenorth memory`.
#[derive(Debug, Subcommand)]
pub enum MemoryAction {
    /// Semantic search over the memory store.
    Query {
        /// Natural-language query string.
        #[arg(long)]
        query: String,

        /// Maximum number of results to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Trigger background memory consolidation (autoDream).
    Consolidate,
    /// Wipe all memory entries (requires explicit confirmation flag).
    Wipe {
        /// Must be passed to confirm the destructive operation.
        #[arg(long)]
        confirm: bool,
    },
}

/// Execute a `memory` sub-command.
///
/// # Arguments
///
/// - `action` — which memory operation to perform.
/// - `format` — output format selector.
pub async fn execute(action: MemoryAction, format: OutputFormat) -> Result<()> {
    match action {
        MemoryAction::Query { query, limit } => query_memory(&query, limit, format).await,
        MemoryAction::Consolidate => consolidate(format).await,
        MemoryAction::Wipe { confirm } => wipe(confirm, format).await,
    }
}

/// Perform a semantic memory query.
async fn query_memory(query: &str, limit: usize, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory query");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info(&format!("  Query: {query}"));
            terminal::print_info(&format!("  Limit: {limit}"));
            terminal::print_info("");
            terminal::print_info("When wired, this command will:");
            terminal::print_info("  1. Embed the query via the configured embedding provider");
            terminal::print_info("  2. Run a cosine-similarity search over episodic + semantic memory");
            terminal::print_info("  3. Return the top-N ranked memory entries");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "memory query",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": { "query": query, "limit": limit },
                "data": { "results": [] }
            }));
        }
    }
    Ok(())
}

/// Trigger memory consolidation.
async fn consolidate(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory consolidate");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info("When wired, this command will trigger the autoDream consolidation cycle:");
            terminal::print_info("  1. Scan episodic memory for entries since the last consolidation");
            terminal::print_info("  2. Generate semantic summaries via the LLM");
            terminal::print_info("  3. Upsert into the semantic memory store");
            terminal::print_info("  4. Prune low-importance episodic entries");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "memory consolidate",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release."
            }));
        }
    }
    Ok(())
}

/// Wipe all memory.
async fn wipe(confirm: bool, format: OutputFormat) -> Result<()> {
    if !confirm {
        match format {
            OutputFormat::Text => {
                terminal::print_error(
                    "Refusing to wipe memory without --confirm flag. \
                     This operation is irreversible.",
                );
            }
            OutputFormat::Json => {
                json::print_json(&serde_json::json!({
                    "command": "memory wipe",
                    "status": "error",
                    "message": "Pass --confirm to confirm the destructive wipe operation."
                }));
            }
        }
        return Ok(());
    }

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory wipe");
            terminal::print_info("Orchestrator not yet wired — this command will be functional in the next release.");
            terminal::print_info("");
            terminal::print_info("When wired, this command will erase ALL memory tiers:");
            terminal::print_info("  - Working memory (in-flight context)");
            terminal::print_info("  - Episodic memory (session history)");
            terminal::print_info("  - Semantic memory (consolidated facts)");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "memory wipe",
                "status": "stub",
                "message": "Orchestrator not yet wired — this command will be functional in the next release.",
                "params": { "confirm": true }
            }));
        }
    }
    Ok(())
}
