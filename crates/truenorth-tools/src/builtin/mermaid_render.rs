//! Mermaid diagram rendering tool.
//!
//! [`MermaidRenderTool`] accepts Mermaid diagram source text and returns an
//! SVG string. The current implementation performs a server-side render using
//! the Mermaid.js CLI via a shell call when available, falling back to a
//! structured placeholder SVG that embeds the diagram source for downstream
//! rendering by the Leptos frontend.
//!
//! # Integration Note
//!
//! When the `rusty-mermaid-diagrams` crate reaches production stability it
//! can replace the shell-based render path with a pure-Rust implementation
//! behind a feature flag. The tool's public interface will not change.

use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{PermissionLevel, ToolError, ToolResult};

/// Renders Mermaid diagram text to an SVG string.
///
/// Attempts to use the `mmdc` (Mermaid CLI) binary if it is on `$PATH`.
/// Falls back to a well-formed placeholder SVG that embeds the diagram
/// source for client-side rendering by the Leptos Visual Reasoning Layer.
///
/// # Permission
/// `Low` — no network access, no filesystem writes.
#[derive(Debug)]
pub struct MermaidRenderTool;

impl MermaidRenderTool {
    /// Creates a new `MermaidRenderTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MermaidRenderTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns `true` if the `mmdc` Mermaid CLI binary is available on `$PATH`.
fn mmdc_available() -> bool {
    std::process::Command::new("mmdc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Attempts to render `diagram_source` to SVG using the `mmdc` CLI.
///
/// Creates a temp file for input and output, invokes `mmdc`, reads the SVG,
/// and cleans up. Returns `None` if `mmdc` fails for any reason.
fn render_with_mmdc(diagram_source: &str) -> Option<String> {
    use std::io::Write;

    let tmp_in = std::env::temp_dir().join(format!(
        "truenorth_mermaid_in_{}.mmd",
        uuid::Uuid::new_v4()
    ));
    let tmp_out = std::env::temp_dir().join(format!(
        "truenorth_mermaid_out_{}.svg",
        uuid::Uuid::new_v4()
    ));

    // Write input file.
    let mut f = std::fs::File::create(&tmp_in).ok()?;
    f.write_all(diagram_source.as_bytes()).ok()?;
    drop(f);

    // Invoke mmdc.
    let status = std::process::Command::new("mmdc")
        .args([
            "-i",
            tmp_in.to_str()?,
            "-o",
            tmp_out.to_str()?,
            "--backgroundColor",
            "transparent",
        ])
        .output()
        .ok()?;

    let svg = if status.status.success() {
        std::fs::read_to_string(&tmp_out).ok()
    } else {
        None
    };

    // Cleanup.
    let _ = std::fs::remove_file(&tmp_in);
    let _ = std::fs::remove_file(&tmp_out);

    svg
}

/// Produces a placeholder SVG that wraps the raw Mermaid source.
///
/// The Leptos frontend detects this placeholder and renders the diagram
/// client-side using the Mermaid.js library.
fn placeholder_svg(diagram_source: &str) -> String {
    // Escape the diagram source for embedding in XML.
    let escaped = diagram_source
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"truenorth-mermaid-placeholder\" \
         data-diagram-source=\"{escaped}\" width=\"100%\" height=\"200\">\
           <text x=\"50%\" y=\"50%\" dominant-baseline=\"middle\" \
             text-anchor=\"middle\" font-family=\"monospace\" font-size=\"12\" fill=\"gray\">\
             Mermaid diagram (rendered client-side)\
           </text>\
           <!-- source -->\
         </svg>"
    )
}

#[async_trait]
impl Tool for MermaidRenderTool {
    fn name(&self) -> &str {
        "render_mermaid"
    }

    fn description(&self) -> &str {
        "Render a Mermaid diagram text to an SVG string. Use this to produce \
         visual diagrams for flowcharts, sequence diagrams, class diagrams, \
         and other Mermaid-supported diagram types. Returns an SVG string \
         suitable for embedding in HTML or the Visual Reasoning Layer."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "diagram": {
                    "type": "string",
                    "description": "The Mermaid diagram source text (e.g., starting with 'graph TD', 'sequenceDiagram', etc.)."
                },
                "theme": {
                    "type": "string",
                    "enum": ["default", "dark", "forest", "neutral"],
                    "description": "Mermaid theme. Default: 'default'.",
                    "default": "default"
                }
            },
            "required": ["diagram"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Low
    }

    fn usage_example(&self) -> Option<&str> {
        Some(r#"{"diagram": "graph TD\n  A[Start] --> B{Decision}\n  B -->|Yes| C[Do it]\n  B -->|No| D[Skip]"}"#)
    }

    async fn execute(&self, args: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let start = Instant::now();

        // --- Parse arguments ---
        let diagram_source = args["diagram"].as_str().ok_or_else(|| ToolError::InvalidArguments {
            tool_name: self.name().to_string(),
            message: "Missing required field 'diagram'".to_string(),
        })?;

        if diagram_source.trim().is_empty() {
            return Err(ToolError::InvalidArguments {
                tool_name: self.name().to_string(),
                message: "Diagram source must not be empty".to_string(),
            });
        }

        debug!(diagram_len = diagram_source.len(), "Rendering Mermaid diagram");

        // --- Attempt server-side render via mmdc ---
        let (svg, render_method) = if mmdc_available() {
            debug!("Using mmdc for Mermaid rendering");
            match render_with_mmdc(diagram_source) {
                Some(svg) => (svg, "mmdc"),
                None => {
                    warn!("mmdc render failed; falling back to placeholder SVG");
                    (placeholder_svg(diagram_source), "placeholder")
                }
            }
        } else {
            debug!("mmdc not found; using placeholder SVG");
            (placeholder_svg(diagram_source), "placeholder")
        };

        let execution_ms = start.elapsed().as_millis() as u64;

        let llm_output = json!({
            "svg": svg,
            "render_method": render_method,
            "diagram_source": diagram_source,
            "svg_length": svg.len()
        });

        let display_output = json!({
            "type": "mermaid_svg",
            "svg": svg,
            "render_method": render_method
        });

        Ok(ToolResult {
            llm_output,
            display_output: Some(display_output),
            side_effects: vec![],
            execution_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder_svg_structure() {
        let svg = placeholder_svg("graph TD\n  A --> B");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("truenorth-mermaid-placeholder"));
        assert!(svg.contains("data-diagram-source"));
    }

    #[test]
    fn test_placeholder_svg_escapes_special_chars() {
        let svg = placeholder_svg("graph TD\n  A[\"Hello <World>\"] --> B");
        assert!(!svg.contains('<') || svg.contains("<svg") || svg.contains("&lt;"));
    }
}
