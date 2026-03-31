/// Diagram rendering for the Visual Reasoning Layer.
///
/// The `DiagramRenderer` converts Mermaid DSL text into displayable formats.
///
/// ## Current implementation strategy
///
/// The Leptos frontend handles Mermaid rendering client-side using `mermaid.js`
/// (loaded from a CDN or bundled). This is the standard approach for Leptos/WASM
/// applications and avoids the need for a headless browser or a native Mermaid
/// renderer on the server.
///
/// `render_to_svg` therefore wraps the raw Mermaid text in a minimal SVG
/// container with a `<foreignObject>` that embeds the Mermaid source as a
/// `<pre class="mermaid">` element. When the browser loads this SVG it runs
/// `mermaid.js` and replaces the `<pre>` in place.
///
/// `render_to_html` produces a standalone HTML page suitable for direct
/// browser viewing or embedding in an `<iframe>`.
///
/// ## Future server-side rendering
///
/// A future version will integrate a native Rust Mermaid renderer for
/// headless PNG/SVG export (e.g. for report generation or PDF attachment).
/// The API surface intentionally returns `Result<String>` to allow this
/// transition without breaking callers.

use thiserror::Error;

/// Errors that can occur during diagram rendering.
#[derive(Debug, Error)]
pub enum RenderError {
    /// The Mermaid source text is empty.
    #[error("Mermaid source text is empty")]
    EmptySource,

    /// The rendering backend returned an error.
    #[error("Rendering backend error: {0}")]
    BackendError(String),
}

/// Converts Mermaid DSL text to displayable diagram formats.
///
/// All methods are stateless and can be called concurrently without
/// any synchronisation.
#[derive(Debug, Default, Clone)]
pub struct DiagramRenderer;

impl DiagramRenderer {
    /// Creates a new `DiagramRenderer`.
    pub fn new() -> Self {
        Self
    }

    /// Converts Mermaid text to an SVG string.
    ///
    /// ## Current behaviour
    ///
    /// Returns the Mermaid source wrapped in a minimal SVG container with a
    /// `<foreignObject>` that embeds the source as a `<pre class="mermaid">`
    /// element. The Leptos frontend's `mermaid.js` integration processes the
    /// `<pre>` element in place when the SVG is rendered in the browser.
    ///
    /// The outer `<svg>` carries a `viewBox` large enough for typical diagrams.
    /// The width/height are set to `100%` so the diagram scales to its container.
    ///
    /// ## Future behaviour
    ///
    /// Will invoke a server-side Mermaid renderer (e.g. via `rusty-mermaid-diagrams`
    /// or a bundled Node.js worker) to produce a self-contained SVG with no
    /// JavaScript dependency.
    ///
    /// # Errors
    /// Returns `RenderError::EmptySource` if `mermaid_text` is blank.
    pub fn render_to_svg(&self, mermaid_text: &str) -> Result<String, RenderError> {
        let trimmed = mermaid_text.trim();
        if trimmed.is_empty() {
            return Err(RenderError::EmptySource);
        }

        // Escape XML special characters in the Mermaid source so it is valid
        // inside an SVG CDATA / text node.
        let escaped = xml_escape(trimmed);

        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"
     xmlns:xhtml="http://www.w3.org/1999/xhtml"
     width="100%" height="100%"
     viewBox="0 0 1200 800"
     data-mermaid-source="true">
  <foreignObject width="100%" height="100%">
    <div xmlns="http://www.w3.org/1999/xhtml"
         style="width:100%;height:100%;overflow:auto;">
      <pre class="mermaid">{escaped}</pre>
    </div>
  </foreignObject>
</svg>"#
        );

        Ok(svg)
    }

    /// Converts Mermaid text to a standalone HTML page.
    ///
    /// The generated page:
    /// - Loads `mermaid.js` from the CDN (version-pinned).
    /// - Initialises Mermaid with the `default` theme.
    /// - Embeds the diagram source in a `<pre class="mermaid">` block.
    ///
    /// The resulting string can be served directly as an HTTP response
    /// (`Content-Type: text/html`) or written to a `.html` file for
    /// offline viewing.
    ///
    /// If `mermaid_text` is empty an empty-diagram placeholder page is returned.
    pub fn render_to_html(&self, mermaid_text: &str) -> String {
        let trimmed = mermaid_text.trim();

        if trimmed.is_empty() {
            return empty_html_page();
        }

        // Escape HTML special characters for safe embedding in a <pre> block.
        let escaped = html_escape(trimmed);

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>TrueNorth — Reasoning Diagram</title>
  <style>
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      background: #1a1a2e;
      color: #e0e0e0;
      font-family: 'Inter', system-ui, sans-serif;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: flex-start;
      min-height: 100vh;
      padding: 2rem;
    }}
    h1 {{
      font-size: 1.1rem;
      font-weight: 500;
      letter-spacing: 0.05em;
      text-transform: uppercase;
      color: #90caf9;
      margin-bottom: 2rem;
    }}
    .diagram-container {{
      background: #16213e;
      border: 1px solid #0f3460;
      border-radius: 8px;
      padding: 2rem;
      width: 100%;
      max-width: 1200px;
      overflow: auto;
    }}
    pre.mermaid {{
      display: block;
      width: 100%;
    }}
  </style>
</head>
<body>
  <h1>TrueNorth Reasoning Diagram</h1>
  <div class="diagram-container">
    <pre class="mermaid">{escaped}</pre>
  </div>
  <script type="module">
    import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs';
    mermaid.initialize({{
      startOnLoad: true,
      theme: 'dark',
      themeVariables: {{
        primaryColor: '#1565c0',
        primaryTextColor: '#ffffff',
        primaryBorderColor: '#0d47a1',
        lineColor: '#90caf9',
        sectionBkgColor: '#16213e',
        altSectionBkgColor: '#0f3460',
        gridColor: '#304050',
        secondaryColor: '#0f3460',
        tertiaryColor: '#1a1a2e',
      }},
    }});
  </script>
</body>
</html>"#
        )
    }

    /// Returns the Mermaid source text unchanged.
    ///
    /// Convenience method for callers that need the raw DSL string after
    /// going through the renderer pipeline (e.g. for logging or testing).
    pub fn raw_source<'a>(&self, mermaid_text: &'a str) -> &'a str {
        mermaid_text
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Escapes XML / SVG special characters for safe embedding in SVG text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Escapes HTML special characters for safe embedding in an HTML `<pre>` block.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Returns a minimal placeholder HTML page for when there is no diagram source.
fn empty_html_page() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>TrueNorth — No Diagram</title>
  <style>
    body { background: #1a1a2e; color: #9e9e9e; font-family: system-ui; display: flex; align-items: center; justify-content: center; height: 100vh; }
    p { font-size: 1.2rem; }
  </style>
</head>
<body>
  <p>No diagram available.</p>
</body>
</html>"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_DIAGRAM: &str = "graph TD\n  A[\"Start\"] --> B[\"End\"]";

    #[test]
    fn render_to_svg_produces_svg_element() {
        let renderer = DiagramRenderer::new();
        let svg = renderer.render_to_svg(SIMPLE_DIAGRAM).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("class=\"mermaid\""));
        assert!(svg.contains("graph TD"));
    }

    #[test]
    fn render_to_svg_empty_returns_error() {
        let renderer = DiagramRenderer::new();
        let result = renderer.render_to_svg("   ");
        assert!(matches!(result, Err(RenderError::EmptySource)));
    }

    #[test]
    fn render_to_html_contains_mermaid_js() {
        let renderer = DiagramRenderer::new();
        let html = renderer.render_to_html(SIMPLE_DIAGRAM);
        assert!(html.contains("mermaid"));
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("graph TD"));
    }

    #[test]
    fn render_to_html_empty_returns_placeholder() {
        let renderer = DiagramRenderer::new();
        let html = renderer.render_to_html("");
        assert!(html.contains("No diagram available"));
    }

    #[test]
    fn svg_escapes_special_chars() {
        let renderer = DiagramRenderer::new();
        // The `<` in the label should be escaped in the SVG output.
        let diagram = "graph TD\n  A[\"x < y\"] --> B[\"y > x\"]";
        let svg = renderer.render_to_svg(diagram).unwrap();
        assert!(svg.contains("&lt;"));
        assert!(svg.contains("&gt;"));
    }
}
