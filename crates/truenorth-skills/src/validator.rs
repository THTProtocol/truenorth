//! Skill validation — checks a parsed skill against the project's rules.
//!
//! [`SkillValidator`] runs a comprehensive set of checks and collects **all**
//! errors before returning, so authors see every problem at once rather than
//! one at a time.

use tracing::debug;

use crate::parser::ParsedSkill;

/// A single validation failure found in a skill.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ValidationError {
    /// The frontmatter field or check that failed.
    pub field: String,
    /// Human-readable description of the failure.
    pub message: String,
}

impl ValidationError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Validates a [`ParsedSkill`] against TrueNorth's skill rules.
///
/// Validation is **non-fail-fast**: all checks run regardless of earlier
/// failures and the full list of errors is returned.
///
/// # Checks performed
///
/// 1. Required frontmatter fields are present and non-empty.
/// 2. `version` is a valid semver string (e.g., `"1.0.0"`).
/// 3. `tools_required` entries exist in the provided `available_tools` list
///    (pass an empty list to skip this check).
/// 4. `permission_level` is one of `"low"`, `"medium"`, or `"high"`.
/// 5. `triggers` is non-empty.
/// 6. The Markdown body contains no fenced executable code blocks (security).
#[derive(Debug, Default)]
pub struct SkillValidator;

impl SkillValidator {
    /// Creates a new `SkillValidator`.
    pub fn new() -> Self {
        Self
    }

    /// Validates a [`ParsedSkill`].
    ///
    /// `available_tools` is the list of tool names currently registered in the
    /// system.  Pass an empty slice to skip the tool-existence check (e.g.,
    /// during offline import).
    ///
    /// # Errors
    ///
    /// Returns `Err(errors)` containing every [`ValidationError`] found.
    /// Returns `Ok(())` only when all checks pass.
    pub fn validate(
        &self,
        skill: &ParsedSkill,
        available_tools: &[String],
    ) -> Result<(), Vec<ValidationError>> {
        let mut errors: Vec<ValidationError> = Vec::new();

        self.check_required_fields(skill, &mut errors);
        self.check_version(skill, &mut errors);
        self.check_permission_level(skill, &mut errors);
        self.check_triggers_non_empty(skill, &mut errors);
        self.check_tools_exist(skill, available_tools, &mut errors);
        self.check_no_executable_code_blocks(skill, &mut errors);

        if errors.is_empty() {
            debug!(name = %skill.frontmatter.name, "Skill passed all validation checks");
            Ok(())
        } else {
            Err(errors)
        }
    }

    // ─── Individual checks ────────────────────────────────────────────────────

    /// Verifies that every required frontmatter field is present and non-empty.
    fn check_required_fields(&self, skill: &ParsedSkill, errors: &mut Vec<ValidationError>) {
        let fm = &skill.frontmatter;

        if fm.name.trim().is_empty() {
            errors.push(ValidationError::new("name", "Required field 'name' is empty"));
        }
        if fm.version.trim().is_empty() {
            errors.push(ValidationError::new("version", "Required field 'version' is empty"));
        }
        if fm.description.trim().is_empty() {
            errors.push(ValidationError::new(
                "description",
                "Required field 'description' is empty",
            ));
        }
        if fm.author.trim().is_empty() {
            errors.push(ValidationError::new("author", "Required field 'author' is empty"));
        }
        if fm.permission_level.trim().is_empty() {
            errors.push(ValidationError::new(
                "permission_level",
                "Required field 'permission_level' is empty",
            ));
        }
    }

    /// Validates that `version` is a three-part semver string (`MAJOR.MINOR.PATCH`).
    ///
    /// Accepts an optional pre-release suffix (e.g., `"1.0.0-beta.1"`).
    fn check_version(&self, skill: &ParsedSkill, errors: &mut Vec<ValidationError>) {
        let version = skill.frontmatter.version.trim();
        if version.is_empty() {
            // Already caught by check_required_fields.
            return;
        }

        // Strip optional pre-release and build metadata before parsing numeric parts.
        let core = version.split('-').next().unwrap_or(version);
        let core = core.split('+').next().unwrap_or(core);

        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() < 2 || parts.len() > 3 {
            errors.push(ValidationError::new(
                "version",
                format!(
                    "Version '{}' is not valid semver — expected MAJOR.MINOR.PATCH",
                    version
                ),
            ));
            return;
        }

        let all_numeric = parts.iter().all(|p| p.parse::<u64>().is_ok());
        if !all_numeric {
            errors.push(ValidationError::new(
                "version",
                format!("Version '{}' contains non-numeric components", version),
            ));
        }
    }

    /// Validates that `permission_level` is one of the allowed values.
    fn check_permission_level(&self, skill: &ParsedSkill, errors: &mut Vec<ValidationError>) {
        let level = skill.frontmatter.permission_level.trim().to_lowercase();
        match level.as_str() {
            "low" | "medium" | "high" => {}
            "" => {} // Already caught.
            other => {
                errors.push(ValidationError::new(
                    "permission_level",
                    format!(
                        "Invalid permission level '{}' — must be 'low', 'medium', or 'high'",
                        other
                    ),
                ));
            }
        }
    }

    /// Validates that the `triggers` list is non-empty.
    fn check_triggers_non_empty(&self, skill: &ParsedSkill, errors: &mut Vec<ValidationError>) {
        if skill.frontmatter.triggers.is_empty() {
            errors.push(ValidationError::new(
                "triggers",
                "Skill must declare at least one trigger phrase",
            ));
        }
    }

    /// Validates that every entry in `tools_required` exists in `available_tools`.
    ///
    /// Skipped when `available_tools` is empty (offline / import-time validation).
    fn check_tools_exist(
        &self,
        skill: &ParsedSkill,
        available_tools: &[String],
        errors: &mut Vec<ValidationError>,
    ) {
        if available_tools.is_empty() {
            return; // Tool registry not available — skip.
        }

        for tool in &skill.frontmatter.tools_required {
            if !available_tools.iter().any(|t| t == tool) {
                errors.push(ValidationError::new(
                    "tools_required",
                    format!(
                        "Required tool '{}' is not registered in the tool registry",
                        tool
                    ),
                ));
            }
        }
    }

    /// Rejects skills that contain fenced code blocks with executable language tags.
    ///
    /// Allowed tags: none, `text`, `markdown`, `yaml`, `toml`, `json`.
    /// Rejected tags: `python`, `javascript`, `js`, `typescript`, `ts`, `bash`,
    /// `sh`, `shell`, `ruby`, `rust`, `go`, `java`, `c`, `cpp`, `csharp`, etc.
    ///
    /// This prevents community skills from sneaking in code that the agent might
    /// execute through a code-running tool.
    fn check_no_executable_code_blocks(
        &self,
        skill: &ParsedSkill,
        errors: &mut Vec<ValidationError>,
    ) {
        // Only sandboxed (community) skills need this check.  First-party skills
        // with `sandboxed: false` are trusted by convention.
        if !skill.frontmatter.sandboxed {
            return;
        }

        let safe_langs: &[&str] = &["", "text", "markdown", "md", "yaml", "toml", "json", "xml"];

        for line in skill.body.raw.lines() {
            let trimmed = line.trim();
            if let Some(lang) = trimmed.strip_prefix("```") {
                let lang = lang.trim().to_lowercase();
                if !safe_langs.contains(&lang.as_str()) {
                    errors.push(ValidationError::new(
                        "body",
                        format!(
                            "Sandboxed skill contains executable code block with language tag '{}'. \
                             Only documentation code blocks (text, yaml, json) are permitted.",
                            lang
                        ),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::SkillMarkdownParser;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_skill(content: &str) -> ParsedSkill {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(content.as_bytes()).unwrap();
        SkillMarkdownParser::new().parse_skill_file(tmp.path()).unwrap()
    }

    const VALID_SKILL: &str = r#"---
name: Test Skill
version: 1.0.0
description: A skill for testing.
triggers:
  - test
  - testing
tools_required: []
permission_level: low
author: Test Author
sandboxed: false
tags: []
---

## Workflow

1. Do the thing.
"#;

    #[test]
    fn valid_skill_passes() {
        let skill = write_skill(VALID_SKILL);
        let validator = SkillValidator::new();
        assert!(validator.validate(&skill, &[]).is_ok());
    }

    #[test]
    fn invalid_version_detected() {
        let content = VALID_SKILL.replace("version: 1.0.0", "version: not-a-version");
        let skill = write_skill(&content);
        let validator = SkillValidator::new();
        let errors = validator.validate(&skill, &[]).unwrap_err();
        assert!(errors.iter().any(|e| e.field == "version"));
    }

    #[test]
    fn invalid_permission_level_detected() {
        let content = VALID_SKILL.replace("permission_level: low", "permission_level: superuser");
        let skill = write_skill(&content);
        let validator = SkillValidator::new();
        let errors = validator.validate(&skill, &[]).unwrap_err();
        assert!(errors.iter().any(|e| e.field == "permission_level"));
    }

    #[test]
    fn missing_tool_detected() {
        let content = VALID_SKILL.replace("tools_required: []", "tools_required:\n  - missing_tool");
        let skill = write_skill(&content);
        let validator = SkillValidator::new();
        let available = vec!["other_tool".to_string()];
        let errors = validator.validate(&skill, &available).unwrap_err();
        assert!(errors.iter().any(|e| e.field == "tools_required"));
    }

    #[test]
    fn empty_triggers_detected() {
        let content = VALID_SKILL.replace("triggers:\n  - test\n  - testing", "triggers: []");
        let skill = write_skill(&content);
        let validator = SkillValidator::new();
        let errors = validator.validate(&skill, &[]).unwrap_err();
        assert!(errors.iter().any(|e| e.field == "triggers"));
    }

    #[test]
    fn executable_code_block_rejected_for_sandboxed() {
        let content = VALID_SKILL.replace("sandboxed: false", "sandboxed: true")
            + "\n```python\nprint('hello')\n```\n";
        let skill = write_skill(&content);
        let validator = SkillValidator::new();
        let errors = validator.validate(&skill, &[]).unwrap_err();
        assert!(errors.iter().any(|e| e.field == "body"));
    }

    #[test]
    fn all_errors_returned_at_once() {
        // skill with multiple failures — we must get all of them, not just the first.
        let content = r#"---
name: ""
version: bad
description: ""
triggers: []
tools_required: []
permission_level: none
author: ""
sandboxed: false
tags: []
---
"#;
        let skill = write_skill(content);
        let validator = SkillValidator::new();
        let errors = validator.validate(&skill, &[]).unwrap_err();
        assert!(errors.len() >= 4, "Expected at least 4 errors, got {}", errors.len());
    }
}
