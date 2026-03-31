//! Skill installation — copy skills from the filesystem or download from a URL.
//!
//! [`SkillInstaller`] handles the mechanics of bringing a new skill into the
//! local `/skills/` directory: copying files, downloading from a URL, validating,
//! and removing unwanted skills.
//!
//! It does **not** interact with the in-memory [`crate::registry::SkillRegistry`]
//! directly — that is the caller's responsibility (see [`crate::registry::SkillRegistry::install_from_url`]).

use std::path::{Path, PathBuf};

use tracing::{debug, info};

use truenorth_core::traits::skill::SkillError;

use crate::parser::{ParsedSkill, SkillMarkdownParser};
use crate::validator::SkillValidator;

/// Installs, validates, and removes SKILL.md skill files on the filesystem.
///
/// # Example — install from URL
///
/// ```rust,ignore
/// use std::path::Path;
/// use truenorth_skills::installer::SkillInstaller;
///
/// let installer = SkillInstaller::new();
/// let parsed = installer
///     .install_from_url(
///         "https://skills.truenorth.dev/research-assistant/SKILL.md",
///         Path::new("skills/community"),
///     )
///     .await
///     .expect("install failed");
/// println!("Installed: {}", parsed.frontmatter.name);
/// ```
#[derive(Debug, Default)]
pub struct SkillInstaller {
    parser: SkillMarkdownParser,
    validator: SkillValidator,
}

impl SkillInstaller {
    /// Creates a new `SkillInstaller`.
    pub fn new() -> Self {
        Self {
            parser: SkillMarkdownParser::new(),
            validator: SkillValidator::new(),
        }
    }

    /// Installs a skill from a source filesystem path into `target_dir`.
    ///
    /// The source can be either:
    /// - A single `SKILL.md` file, in which case it is copied directly.
    /// - A directory containing a `SKILL.md` file, in which case the entire
    ///   directory is copied (preserving any `REFERENCES.md` siblings).
    ///
    /// After copying, the skill is parsed and validated.  If validation fails
    /// the copied files are removed and the error is returned.
    ///
    /// # Errors
    ///
    /// - [`SkillError::FileNotFound`] if `source` does not exist.
    /// - [`SkillError::ValidationFailed`] if the skill fails validation.
    /// - [`SkillError::Io`] for filesystem errors.
    pub fn install_from_path(
        &self,
        source: &Path,
        target_dir: &Path,
    ) -> Result<ParsedSkill, SkillError> {
        info!(source = %source.display(), target = %target_dir.display(), "Installing skill from path");

        if !source.exists() {
            return Err(SkillError::FileNotFound {
                path: source.to_path_buf(),
            });
        }

        std::fs::create_dir_all(target_dir)?;

        let (skill_file_path, dest_path) = if source.is_dir() {
            let skill_md = source.join("SKILL.md");
            if !skill_md.exists() {
                return Err(SkillError::FileNotFound { path: skill_md });
            }
            // Copy the whole directory.
            let dir_name = source
                .file_name()
                .ok_or_else(|| SkillError::ValidationFailed {
                    name: source.display().to_string(),
                    message: "Source directory has no name".to_string(),
                })?;
            let dest = target_dir.join(dir_name);
            copy_dir_recursive(source, &dest)?;
            (dest.join("SKILL.md"), dest)
        } else {
            // Single file — copy directly.
            let file_name = source.file_name().ok_or_else(|| SkillError::Io(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Source has no file name"),
            ))?;
            let dest = target_dir.join(file_name);
            std::fs::copy(source, &dest)?;
            (dest.clone(), dest)
        };

        // Parse and validate.
        let parsed = self.parse_and_validate(&skill_file_path)?;

        info!(name = %parsed.frontmatter.name, dest = %dest_path.display(), "Skill installed from path");
        Ok(parsed)
    }

    /// Downloads a SKILL.md file from `url`, saves it to `target_dir`, parses,
    /// and validates it.
    ///
    /// The file is saved as `<skill-name>.md` where `<skill-name>` is the
    /// URL's last path segment (without extension), or `downloaded_skill.md`
    /// if the name cannot be determined.
    ///
    /// # Errors
    ///
    /// - [`SkillError::RegistryError`] if the HTTP request fails.
    /// - [`SkillError::ValidationFailed`] if the downloaded skill fails validation.
    /// - [`SkillError::Io`] for filesystem errors.
    pub async fn install_from_url(
        &self,
        url: &str,
        target_dir: &Path,
    ) -> Result<ParsedSkill, SkillError> {
        info!(url = %url, target = %target_dir.display(), "Downloading skill from URL");

        std::fs::create_dir_all(target_dir)?;

        let content = download_url(url).await?;

        // Determine target filename from the URL or skill name.
        let file_name = derive_filename_from_url(url);
        let dest_path = target_dir.join(&file_name);

        debug!(path = %dest_path.display(), "Writing downloaded skill to disk");
        std::fs::write(&dest_path, &content)?;

        // Parse to get the canonical skill name, then rename if needed.
        let parsed_name = {
            let tmp_parsed = self.parser.parse_skill_file(&dest_path).map_err(|e| {
                // Remove the bad file.
                let _ = std::fs::remove_file(&dest_path);
                SkillError::FrontmatterParseError {
                    path: dest_path.clone(),
                    message: e.to_string(),
                }
            })?;
            tmp_parsed.frontmatter.name.clone()
        };

        // Rename to a canonical slug derived from the skill name.
        let canonical_name = to_slug(&parsed_name) + ".md";
        let final_path = if canonical_name != file_name {
            let new_path = target_dir.join(&canonical_name);
            std::fs::rename(&dest_path, &new_path)?;
            new_path
        } else {
            dest_path
        };

        let parsed = self.parse_and_validate(&final_path)?;
        info!(name = %parsed.frontmatter.name, path = %final_path.display(), "Skill installed from URL");
        Ok(parsed)
    }

    /// Removes a skill's files from `skills_dir`.
    ///
    /// Searches for a `*.md` file whose `name` frontmatter field matches
    /// `skill_id` (case-insensitive), then removes it.  If the skill lives
    /// inside a directory (the directory contains only that skill), the whole
    /// directory is removed.
    ///
    /// # Errors
    ///
    /// - [`SkillError::SkillNotFound`] if no matching skill file is found.
    /// - [`SkillError::Io`] for filesystem errors.
    pub fn uninstall(&self, skill_id: &str, skills_dir: &Path) -> Result<(), SkillError> {
        info!(skill = %skill_id, dir = %skills_dir.display(), "Uninstalling skill");

        let pattern = format!("{}/**/*.md", skills_dir.display());
        let files: Vec<PathBuf> = glob::glob(&pattern)
            .map_err(|e| {
                SkillError::ValidationFailed {
                    name: skill_id.to_string(),
                    message: format!("Glob error: {}", e),
                }
            })?
            .filter_map(|e| e.ok())
            .collect();

        for path in &files {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(fm) = self.parser.parse_frontmatter(&content, path) {
                    if fm.name.to_lowercase() == skill_id.to_lowercase() {
                        // Remove the skill file.
                        std::fs::remove_file(path)?;
                        debug!(path = %path.display(), "Removed skill file");

                        // If the parent directory is now empty, remove it too.
                        if let Some(parent) = path.parent() {
                            if parent != skills_dir {
                                if let Ok(mut entries) = std::fs::read_dir(parent) {
                                    if entries.next().is_none() {
                                        let _ = std::fs::remove_dir(parent);
                                    }
                                }
                            }
                        }

                        info!(name = %skill_id, "Skill uninstalled");
                        return Ok(());
                    }
                }
            }
        }

        Err(SkillError::SkillNotFound {
            name: skill_id.to_string(),
        })
    }

    // ─── Private helpers ──────────────────────────────────────────────────────

    /// Parses a skill file and runs validation against an empty tool list.
    fn parse_and_validate(&self, path: &Path) -> Result<ParsedSkill, SkillError> {
        let parsed = self.parser.parse_skill_file(path).map_err(|e| {
            SkillError::FrontmatterParseError {
                path: path.to_path_buf(),
                message: e.to_string(),
            }
        })?;

        // Validate — pass empty tool list (we don't have the registry here).
        if let Err(errors) = self.validator.validate(&parsed, &[]) {
            let messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            return Err(SkillError::ValidationFailed {
                name: parsed.frontmatter.name.clone(),
                message: messages.join("; "),
            });
        }

        Ok(parsed)
    }
}

// ─── Filesystem helpers ───────────────────────────────────────────────────────

/// Recursively copies a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), SkillError> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }

    Ok(())
}

/// Derives a safe filename from a URL by taking the last path segment.
fn derive_filename_from_url(url: &str) -> String {
    url.split('/')
        .filter(|s| !s.is_empty())
        .last()
        .map(|s| {
            // If the last segment already has an extension, use as-is; otherwise add .md.
            if s.contains('.') {
                s.to_string()
            } else {
                format!("{}.md", s)
            }
        })
        .unwrap_or_else(|| "downloaded_skill.md".to_string())
}

/// Converts a skill name to a URL-safe slug (lowercase, spaces → hyphens).
fn to_slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ─── HTTP download helper ─────────────────────────────────────────────────────

/// Downloads the content at `url` and returns it as a UTF-8 string.
///
/// Uses `reqwest` with a 30-second timeout.
async fn download_url(url: &str) -> Result<String, SkillError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("truenorth-skills/0.1 (https://github.com/THTProtocol/truenorth)")
        .build()
        .map_err(|e| SkillError::RegistryError {
            message: format!("Failed to build HTTP client: {}", e),
        })?;

    let response = client.get(url).send().await.map_err(|e| {
        SkillError::RegistryError {
            message: format!("HTTP request to '{}' failed: {}", url, e),
        }
    })?;

    if !response.status().is_success() {
        return Err(SkillError::RegistryError {
            message: format!(
                "HTTP {} when fetching '{}': {}",
                response.status().as_u16(),
                url,
                response.status().canonical_reason().unwrap_or("unknown")
            ),
        });
    }

    let text = response.text().await.map_err(|e| SkillError::RegistryError {
        message: format!("Failed to read response body from '{}': {}", url, e),
    })?;

    Ok(text)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    const VALID_SKILL: &str = r#"---
name: Install Test
version: 1.0.0
description: A skill used to test installation.
triggers:
  - install
tools_required: []
permission_level: low
author: Test Author
sandboxed: false
tags: []
---

## Workflow

1. Install the thing.
"#;

    #[test]
    fn install_from_path_single_file() {
        let target = tempdir().unwrap();
        let mut src_file = NamedTempFile::new().unwrap();
        src_file.write_all(VALID_SKILL.as_bytes()).unwrap();

        let installer = SkillInstaller::new();
        let parsed = installer
            .install_from_path(src_file.path(), target.path())
            .unwrap();

        assert_eq!(parsed.frontmatter.name, "Install Test");
    }

    #[test]
    fn install_from_path_directory() {
        let src_dir = tempdir().unwrap();
        let target_dir = tempdir().unwrap();

        // Create a SKILL.md inside a sub-directory.
        std::fs::write(src_dir.path().join("SKILL.md"), VALID_SKILL).unwrap();

        let installer = SkillInstaller::new();
        let parsed = installer
            .install_from_path(src_dir.path(), target_dir.path())
            .unwrap();

        assert_eq!(parsed.frontmatter.name, "Install Test");
    }

    #[test]
    fn install_from_nonexistent_path_errors() {
        let target = tempdir().unwrap();
        let installer = SkillInstaller::new();
        let result = installer.install_from_path(Path::new("/does/not/exist.md"), target.path());
        assert!(matches!(result, Err(SkillError::FileNotFound { .. })));
    }

    #[test]
    fn uninstall_removes_skill() {
        let skills_dir = tempdir().unwrap();
        std::fs::write(skills_dir.path().join("install-test.md"), VALID_SKILL).unwrap();

        let installer = SkillInstaller::new();
        installer
            .uninstall("Install Test", skills_dir.path())
            .unwrap();

        assert!(!skills_dir.path().join("install-test.md").exists());
    }

    #[test]
    fn uninstall_missing_skill_errors() {
        let skills_dir = tempdir().unwrap();
        let installer = SkillInstaller::new();
        let result = installer.uninstall("Nonexistent Skill", skills_dir.path());
        assert!(matches!(result, Err(SkillError::SkillNotFound { .. })));
    }

    #[test]
    fn derive_filename_from_url_with_extension() {
        assert_eq!(derive_filename_from_url("https://example.com/skills/SKILL.md"), "SKILL.md");
    }

    #[test]
    fn derive_filename_from_url_without_extension() {
        assert_eq!(derive_filename_from_url("https://example.com/skills/research"), "research.md");
    }

    #[test]
    fn to_slug_converts_spaces() {
        assert_eq!(to_slug("Research Assistant"), "research-assistant");
    }

    #[test]
    fn to_slug_handles_special_chars() {
        assert_eq!(to_slug("My Skill (v2)"), "my-skill-v2");
    }
}
