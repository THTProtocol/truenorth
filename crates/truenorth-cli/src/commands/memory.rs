//! `truenorth memory` — memory inspection and maintenance sub-commands.

use anyhow::Result;
use clap::Subcommand;
use std::path::Path;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Sub-commands for `truenorth memory`.
#[derive(Debug, Subcommand)]
pub enum MemoryAction {
    /// Search memory entries.
    Query {
        /// Search query string.
        #[arg(long)]
        query: String,

        /// Maximum number of results to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Trigger memory consolidation (session → project tier promotion).
    Consolidate,
    /// Show memory store statistics.
    Stats,
    /// Wipe all memory entries (requires --confirm flag).
    Wipe {
        /// Must be passed to confirm the destructive operation.
        #[arg(long)]
        confirm: bool,
    },
}

/// Execute a `memory` sub-command.
pub async fn execute(action: MemoryAction, format: OutputFormat) -> Result<()> {
    match action {
        MemoryAction::Query { query, limit } => query_memory(&query, limit, format).await,
        MemoryAction::Consolidate => consolidate(format).await,
        MemoryAction::Stats => stats(format).await,
        MemoryAction::Wipe { confirm } => wipe(confirm, format).await,
    }
}

/// Report on the memory data directory.
fn check_memory_paths() -> (bool, bool, bool) {
    let db_exists = Path::new("data/truenorth.db").exists();
    let index_exists = Path::new("data/tantivy").exists();
    let vault_exists = Path::new("vault").exists();
    (db_exists, index_exists, vault_exists)
}

/// Search memory.
async fn query_memory(query: &str, limit: usize, format: OutputFormat) -> Result<()> {
    let (db, index, vault) = check_memory_paths();

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory query");
            terminal::print_info(&format!("Query: \"{query}\"  (limit: {limit})"));
            terminal::print_info("");

            if !db && !index {
                terminal::print_warning("No memory store found. Run a task first to initialize memory.");
                terminal::print_info("");
                terminal::print_info("Expected data paths:");
                terminal::print_info("  data/truenorth.db  — SQLite memory store");
                terminal::print_info("  data/tantivy/      — Full-text search index");
                terminal::print_info("  vault/             — Obsidian-compatible Markdown vault");
            } else {
                terminal::print_info("Memory store found:");
                terminal::print_info(&format!("  SQLite DB:     {}", if db { "present" } else { "missing" }));
                terminal::print_info(&format!("  Tantivy index: {}", if index { "present" } else { "missing" }));
                terminal::print_info(&format!("  Obsidian vault: {}", if vault { "present" } else { "missing" }));
                terminal::print_info("");
                terminal::print_info("Direct CLI memory search will be available once the memory layer");
                terminal::print_info("is wired into the CLI. For now, use the web API:");
                terminal::print_info(&format!("  curl 'http://localhost:8080/api/v1/memory/search?q={query}&limit={limit}'"));
            }
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "query": query,
                "limit": limit,
                "store": {
                    "sqlite_db": db,
                    "tantivy_index": index,
                    "obsidian_vault": vault,
                },
                "results": [],
                "note": "Direct CLI search not yet wired. Use the web API.",
            }));
        }
    }
    Ok(())
}

/// Trigger consolidation.
async fn consolidate(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory consolidate");
            terminal::print_info("Consolidation promotes session-tier memories to project-tier.");
            terminal::print_info("This runs automatically on a timer, but can be triggered manually.");
            terminal::print_info("");
            terminal::print_info("Direct CLI consolidation not yet wired. The consolidation scheduler");
            terminal::print_info("runs automatically when the agent loop is active via `truenorth serve`.");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "memory consolidate",
                "status": "not_yet_wired",
                "note": "Consolidation runs automatically via the agent loop.",
            }));
        }
    }
    Ok(())
}

/// Show memory statistics.
async fn stats(format: OutputFormat) -> Result<()> {
    let (db, index, vault) = check_memory_paths();

    // Try to get file sizes
    let db_size = std::fs::metadata("data/truenorth.db").map(|m| m.len()).unwrap_or(0);
    let vault_files = if vault {
        std::fs::read_dir("vault")
            .map(|rd| rd.flatten().count())
            .unwrap_or(0)
    } else {
        0
    };

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory stats");
            terminal::print_info("");
            terminal::print_table(
                &["Store", "Status", "Size/Count"],
                &[
                    vec!["SQLite DB".into(), if db { "present" } else { "not initialized" }.into(), format_bytes(db_size)],
                    vec!["Tantivy index".into(), if index { "present" } else { "not initialized" }.into(), "-".into()],
                    vec!["Obsidian vault".into(), if vault { "present" } else { "not initialized" }.into(), format!("{vault_files} files")],
                ],
            );
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "sqlite_db": { "exists": db, "size_bytes": db_size },
                "tantivy_index": { "exists": index },
                "obsidian_vault": { "exists": vault, "file_count": vault_files },
            }));
        }
    }
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 { return "0 B".into(); }
    if bytes < 1024 { return format!("{bytes} B"); }
    if bytes < 1024 * 1024 { return format!("{:.1} KB", bytes as f64 / 1024.0); }
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// Wipe all memory.
async fn wipe(confirm: bool, format: OutputFormat) -> Result<()> {
    if !confirm {
        match format {
            OutputFormat::Text => {
                terminal::print_error("Refusing to wipe memory without --confirm flag.");
                terminal::print_error("This operation is irreversible and deletes ALL memory tiers.");
                terminal::print_info("");
                terminal::print_info("Usage: truenorth memory wipe --confirm");
            }
            OutputFormat::Json => {
                json::print_json(&serde_json::json!({
                    "error": "Pass --confirm to confirm the destructive wipe operation.",
                }));
            }
        }
        return Ok(());
    }

    // Actually attempt to delete memory files
    let mut deleted = Vec::new();
    if Path::new("data/truenorth.db").exists() {
        if std::fs::remove_file("data/truenorth.db").is_ok() {
            deleted.push("data/truenorth.db");
        }
    }
    if Path::new("data/tantivy").exists() {
        if std::fs::remove_dir_all("data/tantivy").is_ok() {
            deleted.push("data/tantivy/");
        }
    }

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth memory wipe");
            if deleted.is_empty() {
                terminal::print_info("No memory files found to delete.");
            } else {
                terminal::print_success(&format!("Deleted {} store(s):", deleted.len()));
                for d in &deleted {
                    terminal::print_info(&format!("  - {d}"));
                }
            }
            terminal::print_info("");
            terminal::print_warning("Obsidian vault (vault/) was NOT deleted. Remove manually if needed.");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "deleted": deleted,
                "note": "Obsidian vault was not deleted.",
            }));
        }
    }
    Ok(())
}
