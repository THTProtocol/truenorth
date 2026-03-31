//! SKILL.md file parser.
//!
//! Parses a SKILL.md file into a [`ParsedSkill`] containing the typed [`SkillFrontmatter`]
//! and a [`SkillBody`] with all extracted sections. Validation of the parsed data is
//! handled separately by [`crate::validator::SkillValidator`].

use std::path::Path;

use thiserror::Error;
use tracing::{debug, warn};

use truenorth_core::types::skill::SkillFrontmatter;

/// Errors produced by the Markdown parser.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The file could not be read from disk.
    #[error("Failed to read skill file at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// No `---` delimiters were found.
    #[error("No YAML frontmatter found in {path}: file must begin with ---")]
    NoFrontmatter { path: std::path::PathBuf },

    /// The YAML between the `---` delimiters could not be deserialized.
    #[error("YAML frontmatter parse error in {path}: {message}")]
    YamlError { path: std::path::PathBuf, message: String },

    /// The parsed frontmatter was missing a required field.
    #[error("Missing required frontmatter field '{field}' in {path}")]
    MissingField { field: String, path: std::path::PathBuf },
}

/// The structured body of a SKILL.md file after the frontmatter.
///
/// Sections are extracted by heading name. If a section heading is absent
/// the corresponding field is an empty `String`; callers should treat an
/// empty string as "section not present" rather than as an error.
#[derive(Debug, Clone, Default)]
pub struct SkillBody {
    /// The raw Markdown source of the full body (everything after the closing `---`).
    pub raw: String,

    /// Content of the "## When to Use" section.
    pub when_to_use: String,

    /// Content of the "## Workflow" section (numbered steps, sub-headings, etc.).
    pub workflow: String,

    /// Content of the "## Best Practices" section.
    pub best_practices: String,

    /// Content of the "## References" section.
    pub references: String,
}

/// A fully parsed SKILL.md file: typed frontmatter plus extracted body sections.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    /// The parsed and deserialized frontmatter.
    pub frontmatter: SkillFrontmatter,

    /// The body sections extracted from the Markdown source.
    pub body: SkillBody,

    /// The filesystem path this skill was loaded from.
    pub file_path: std::path::PathBuf,
}

/// Parser for SKILL.md-format files.
///
/// Separates the YAML frontmatter (between `---` delimiters) from the Markdown
/// body, deserializes the frontmatter, and extracts named sections from the body.
///
/// # Example
///
/// ```rust,ignore
/// use std::path::Path;
/// use truenorth_skills::parser::SkillMarkdownParser;
///
/// let parser = SkillMarkdownParser::new();
/// let skill = parser.parse_skill_file(Path::new("skills/research.md")).unwrap();
/// println!("Loaded: {}", skill.frontmatter.name);
/// ```
#[derive(Debug, Default)]
pub struct SkillMarkdownParser;

impl SkillMarkdownParser {
    /// Creates a new parser instance.
    pub fn new() -> Self {
        Self
    }

    /// Extracts and deserializes the YAML frontmatter from Markdown source text.
    ///
    /// The frontmatter must appear at the very start of the file between `---`
    /// delimiters.  Everything before the first `---` and after the second
    /// `---` is ignored by this function.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::NoFrontmatter`] if no `---` delimiters are found,
    /// or [`ParseError::YamlError`] if the YAML cannot be deserialized into
    /// [`SkillFrontmatter`].
    pub fn parse_frontmatter(
        &self,
        content: &str,
        path: &Path,
    ) -> Result<SkillFrontmatter, ParseError> {
        let yaml = self.extract_frontmatter_yaml(content).ok_or_else(|| {
            ParseError::NoFrontmatter { path: path.to_path_buf() }
        })?;

        debug!(path = %path.display(), "Deserializing YAML frontmatter ({} bytes)", yaml.len());

        serde_yaml::from_str::<SkillFrontmatter>(yaml).map_err(|e| ParseError::YamlError {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// Extracts and parses all body sections from Markdown source text.
    ///
    /// Sections are identified by level-2 headings (`## Heading Name`).
    /// The known headings are:
    /// - `When to Use`
    /// - `Workflow`
    /// - `Best Practices`
    /// - `References`
    ///
    /// Any unrecognised headings are silently skipped; their content is still
    /// included in [`SkillBody::raw`].
    pub fn parse_body(&self, content: &str) -> SkillBody {
        let raw = self.extract_body(content);
        let mut body = SkillBody {
            raw: raw.to_string(),
            ..Default::default()
        };

        let mut current_section: Option<&str> = None;
        let mut current_lines: Vec<&str> = Vec::new();

        for line in raw.lines() {
            if let Some(heading) = Self::parse_h2_heading(line) {
                // Flush the previous section.
                if let Some(sec) = current_section {
                    let text = current_lines.join("\n").trim().to_string();
                    Self::assign_section(&mut body, sec, text);
                }
                current_section = Some(Self::canonical_section_name(heading));
                current_lines.clear();
            } else {
                current_lines.push(line);
            }
        }

        // Flush the last section.
        if let Some(sec) = current_section {
            let text = current_lines.join("\n").trim().to_string();
            Self::assign_section(&mut body, sec, text);
        }

        body
    }

    /// Reads a SKILL.md file from disk and returns a fully parsed [`ParsedSkill`].
    ///
    /// This is the primary entry point for the loader.  It reads the file,
    /// extracts the frontmatter and body, and returns the combined result.
    ///
    /// # Errors
    ///
    /// - [`ParseError::Io`] if the file cannot be read.
    /// - [`ParseError::NoFrontmatter`] if no frontmatter delimiters are found.
    /// - [`ParseError::YamlError`] if the YAML is malformed.
    pub fn parse_skill_file(&self, path: &Path) -> Result<ParsedSkill, ParseError> {
        debug!(path = %path.display(), "Parsing SKILL.md file");

        let content = std::fs::read_to_string(path).map_err(|e| ParseError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

        let frontmatter = self.parse_frontmatter(&content, path)?;
        let body = self.parse_body(&content);

        debug!(
            name = %frontmatter.name,
            version = %frontmatter.version,
            path = %path.display(),
            "Successfully parsed skill"
        );

        Ok(ParsedSkill {
            frontmatter,
            body,
            file_path: path.to_path_buf(),
        })
    }

    // ─── Private helpers ──────────────────────────────────────────────────────

    /// Returns the raw YAML string between the first pair of `---` delimiters,
    /// or `None` if the file does not start with a `---` line.
    fn extract_frontmatter_yaml<'a>(&self, content: &'a str) -> Option<&'a str> {
        // Must start with "---" (optionally followed by whitespace).
        let content = content.trim_start_matches('\u{feff}'); // strip BOM if present
        let rest = content.strip_prefix("---")?;

        // The rest should start with a newline.
        let rest = if rest.starts_with('\n') {
            &rest[1..]
        } else if rest.starts_with("\r\n") {
            &rest[2..]
        } else if rest.is_empty() {
            rest
        } else {
            return None;
        };

        // Find the closing "---".
        let end = rest.find("\n---")?;
        Some(&rest[..end])
    }

    /// Returns the Markdown body: everything after the closing `---` delimiter.
    fn extract_body<'a>(&self, content: &'a str) -> &'a str {
        let content = content.trim_start_matches('\u{feff}');
        // Skip opening ---
        let rest = match content.strip_prefix("---") {
            Some(r) => r,
            None => return content,
        };
        let rest = if rest.starts_with('\n') {
            &rest[1..]
        } else if rest.starts_with("\r\n") {
            &rest[2..]
        } else {
            rest
        };

        // Find closing --- and take everything after it.
        if let Some(pos) = rest.find("\n---") {
            let after = &rest[pos + 4..]; // skip "\n---"
            // Skip optional trailing \n or \r\n on the closing delimiter line.
            if after.starts_with('\n') {
                &after[1..]
            } else if after.starts_with("\r\n") {
                &after[2..]
            } else {
                after
            }
        } else {
            warn!("Could not find closing --- delimiter; treating entire remainder as body");
            rest
        }
    }

    /// If `line` is a level-2 Markdown heading (`## Foo`), returns the heading
    /// text (e.g., `"Foo"`).  Otherwise returns `None`.
    fn parse_h2_heading(line: &str) -> Option<&str> {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            Some(trimmed[3..].trim())
        } else {
            None
        }
    }

    /// Normalises a heading string to the known canonical section names used as
    /// [`SkillBody`] field selectors.
    fn canonical_section_name(heading: &str) -> &'static str {
        match heading.to_lowercase().as_str() {
            h if h.contains("when") && h.contains("use") => "when_to_use",
            h if h.contains("workflow") => "workflow",
            h if h.contains("best") && h.contains("practice") => "best_practices",
            h if h.contains("reference") => "references",
            _ => "unknown",
        }
    }

    /// Assigns `text` to the appropriate field of `body` based on `section`.
    fn assign_section(body: &mut SkillBody, section: &str, text: String) {
        match section {
            "when_to_use" => body.when_to_use = text,
            "workflow" => body.workflow = text,
            "best_practices" => body.best_practices = text,
            "references" => body.references = text,
            _ => {} // unknown sections are captured in raw only
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const SAMPLE_SKILL: &str = r#"---
name: Test Skill
version: 1.0.0
description: A skill for testing.
triggers:
  - test
  - testing
tools_required:
  - search_web
permission_level: low
author: Test Author
sandboxed: false
tags:
  - test
---

## When to Use

Use this when you want to test something.

## Workflow

1. Step one.
2. Step two.

## Best Practices

- Do good things.

## References

- https://example.com
"#;

    #[test]
    fn parse_frontmatter_success() {
        let parser = SkillMarkdownParser::new();
        let path = Path::new("test.md");
        let fm = parser.parse_frontmatter(SAMPLE_SKILL, path).unwrap();
        assert_eq!(fm.name, "Test Skill");
        assert_eq!(fm.version, "1.0.0");
        assert_eq!(fm.triggers, vec!["test", "testing"]);
    }

    #[test]
    fn parse_body_sections() {
        let parser = SkillMarkdownParser::new();
        let body = parser.parse_body(SAMPLE_SKILL);
        assert!(body.when_to_use.contains("test something"));
        assert!(body.workflow.contains("Step one"));
        assert!(body.best_practices.contains("good things"));
        assert!(body.references.contains("example.com"));
    }

    #[test]
    fn parse_skill_file_roundtrip() {
        let parser = SkillMarkdownParser::new();
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(SAMPLE_SKILL.as_bytes()).unwrap();
        let parsed = parser.parse_skill_file(tmp.path()).unwrap();
        assert_eq!(parsed.frontmatter.name, "Test Skill");
        assert!(parsed.body.workflow.contains("Step one"));
    }

    #[test]
    fn missing_frontmatter_returns_error() {
        let parser = SkillMarkdownParser::new();
        let content = "# Just a heading\n\nNo frontmatter here.";
        let result = parser.parse_frontmatter(content, Path::new("no_fm.md"));
        assert!(matches!(result, Err(ParseError::NoFrontmatter { .. })));
    }
}
