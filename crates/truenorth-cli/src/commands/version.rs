//! `truenorth version` — display version and build information.
//!
//! This is the **only fully implemented command** in v1.  It prints static
//! metadata about the TrueNorth binary without requiring any wired
//! dependencies.

use anyhow::Result;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// TrueNorth version string, taken from the crate manifest at compile time.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Execute the `version` command.
///
/// # Arguments
///
/// - `format` — output format selector.
pub async fn execute(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => print_text_version(),
        OutputFormat::Json => print_json_version(),
    }
    Ok(())
}

/// Print a human-readable version block.
fn print_text_version() {
    terminal::print_header(&format!("TrueNorth v{VERSION}"));
    println!();
    terminal::print_info(&format!("Version      : {VERSION}"));
    terminal::print_info("Commit       : (built from source)");
    terminal::print_info("Rust         : 1.80+");
    terminal::print_info("License      : Apache-2.0");
    println!();
    terminal::print_info("Architecture :");
    terminal::print_info("  • File-tree-as-program execution model");
    terminal::print_info("  • WASM-sandboxed skills (wasmtime)");
    terminal::print_info("  • Visual Reasoning Layer (live Mermaid graph)");
    terminal::print_info("  • Multi-provider LLM routing (OpenAI, Anthropic, Ollama, …)");
    terminal::print_info("  • Three-tier memory (working / episodic / semantic)");
    terminal::print_info("  • MCP + A2A protocol support");
}

/// Print a machine-readable JSON version block.
fn print_json_version() {
    let data = serde_json::json!({
        "name": "truenorth",
        "version": VERSION,
        "commit": "(built from source)",
        "rust_version": "1.80+",
        "license": "Apache-2.0",
        "architecture": {
            "execution_model": "file-tree-as-program",
            "skill_sandbox": "WASM (wasmtime)",
            "visual_reasoning": true,
            "llm_providers": ["openai", "anthropic", "ollama", "together", "groq"],
            "memory_tiers": ["working", "episodic", "semantic"],
            "protocols": ["MCP", "A2A"]
        }
    });
    json::print_json(&data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_version_text_does_not_error() {
        let result = execute(OutputFormat::Text).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_version_json_does_not_error() {
        let result = execute(OutputFormat::Json).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_version_constant_is_semver() {
        // Verify the version string looks like a semver.
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert!(
            parts.len() >= 2,
            "VERSION {VERSION:?} should be a valid semver"
        );
        for part in &parts {
            assert!(
                part.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false),
                "VERSION component {part:?} should start with a digit"
            );
        }
    }
}
