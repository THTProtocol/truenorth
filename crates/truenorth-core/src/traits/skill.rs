/// Skill and SkillLoader traits — the skill system contract.
///
/// Skills are instruction documents (SKILL.md format) that guide the agent
/// through complex workflows. The Skill trait exposes content at progressive
/// load levels. The SkillLoader manages discovery, loading, and caching.

use async_trait::async_trait;
use std::path::Path;
use thiserror::Error;

use crate::types::skill::{SkillFrontmatter, SkillLoadLevel};

/// A fully loaded skill with all content levels available.
///
/// The orchestrator works with `Box<dyn Skill>` throughout — it never needs
/// to know whether a skill was loaded from a file, a network registry, or
/// defined inline in Rust.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// Parsed frontmatter (identity and metadata).
    pub metadata: SkillFrontmatter,
    /// The workflow markdown body (Level 1 content).
    pub workflow_body: String,
    /// Extended reference materials (Level 2 content). None until explicitly loaded.
    pub extended_content: Option<String>,
    /// Filesystem path to the skill file (for hot reload on change).
    pub file_path: std::path::PathBuf,
}

/// Errors from skill loading and validation.
#[derive(Debug, Error)]
pub enum SkillError {
    /// The skill file was not found at the specified path.
    #[error("Skill file not found: {path}")]
    FileNotFound { path: std::path::PathBuf },

    /// The SKILL.md frontmatter could not be parsed.
    #[error("Failed to parse skill frontmatter in {path}: {message}")]
    FrontmatterParseError {
        path: std::path::PathBuf,
        message: String,
    },

    /// The skill requires a tool that is not registered.
    #[error("Skill '{name}' requires tool '{tool_name}' which is not registered")]
    MissingRequiredTool { name: String, tool_name: String },

    /// The skill declares an unrecognized permission level.
    #[error("Skill '{name}' has invalid permission level: '{level}'")]
    InvalidPermissionLevel { name: String, level: String },

    /// The skill failed general validation.
    #[error("Skill '{name}' failed validation: {message}")]
    ValidationFailed { name: String, message: String },

    /// Failed to connect to the skill registry.
    #[error("Skill registry connection failed: {message}")]
    RegistryError { message: String },

    /// I/O error while reading a skill file.
    #[error("I/O error reading skill: {0}")]
    Io(#[from] std::io::Error),

    /// No skill was found matching the given name.
    #[error("Skill '{name}' not found")]
    SkillNotFound { name: String },
}

/// The core skill trait. Implemented by `LoadedSkill` and by any programmatic
/// skill defined in Rust (for built-in skills).
///
/// Design rationale: skills are instruction documents, not code.
/// The `Skill` trait reflects this: it exposes the skill's instructions at each
/// load level, rather than an `execute()` method. The orchestrator reads the
/// instructions and drives execution through the existing tool registry and
/// agent loop. The skill is a template for behavior, not an executable unit.
pub trait Skill: Send + Sync + std::fmt::Debug {
    /// Returns the skill's parsed frontmatter (identity and metadata).
    fn metadata(&self) -> &SkillFrontmatter;

    /// Returns the content at the requested load level.
    ///
    /// - Level 0 (Minimal): Returns the "name — description" string (~80 tokens).
    /// - Level 1 (Full): Returns the full workflow markdown body.
    /// - Level 2 (Extended): Returns extended reference content, falling back to Level 1.
    ///
    /// The orchestrator injects this content into the LLM context at the appropriate
    /// point in the reasoning loop.
    fn content_at_level(&self, level: SkillLoadLevel) -> &str;

    /// Returns the trigger phrases for this skill.
    ///
    /// Used by the trigger matcher to select the right skill for a user input.
    fn triggers(&self) -> &[String];

    /// Returns the names of tools this skill requires.
    ///
    /// The orchestrator verifies all required tools are available before invoking a skill.
    fn required_tools(&self) -> &[String];

    /// Returns the workflow content as a structured series of numbered steps.
    ///
    /// The orchestrator uses this to drive sequential execution mode.
    /// Returns None for skills that don't follow the numbered-steps format.
    fn workflow_steps(&self) -> Option<Vec<String>>;

    /// Returns the skill's canonical name (convenience for `metadata().name`).
    fn name(&self) -> &str {
        &self.metadata().name
    }

    /// Returns the skill's version (convenience for `metadata().version`).
    fn version(&self) -> &str {
        &self.metadata().version
    }
}

/// The skill loader trait: responsible for discovering, loading, and caching skills.
///
/// Skills are loaded from the filesystem (the skills directory) and optionally
/// from the TrueNorth curated registry. The loader caches parsed skills to avoid
/// repeated parsing overhead.
#[async_trait]
pub trait SkillLoader: Send + Sync + std::fmt::Debug {
    /// Scans a directory for `.md` files with valid SKILL.md frontmatter.
    ///
    /// Returns the number of skills found and successfully validated.
    /// Invalid files are logged as warnings, not errors — one bad skill
    /// should not prevent all others from loading.
    async fn scan_directory(&self, dir: &Path) -> Result<usize, SkillError>;

    /// Loads a single skill file and returns a fully parsed `Box<dyn Skill>`.
    ///
    /// Validates frontmatter, checks required tools against the registry,
    /// and caches the result for subsequent requests.
    async fn load(
        &self,
        path: &Path,
        level: SkillLoadLevel,
    ) -> Result<Box<dyn Skill>, SkillError>;

    /// Performs progressive loading: upgrades a skill from Level 0 to the target level.
    ///
    /// Called when the orchestrator triggers a skill that was previously only known
    /// at the Minimal level. Loads and caches the additional content.
    async fn progressive_load(
        &self,
        skill_name: &str,
        target_level: SkillLoadLevel,
    ) -> Result<Box<dyn Skill>, SkillError>;

    /// Returns all currently loaded skills at Level 0 (Minimal).
    ///
    /// Called by the orchestrator to inject skill listings into LLM context.
    /// This is the skill index that tells the LLM what skills are available.
    fn list_skills_minimal(&self) -> Vec<SkillFrontmatter>;

    /// Matches a user input string against skill triggers.
    ///
    /// Returns matched skills ranked by trigger confidence (0.0–1.0).
    /// The tuple is (skill_name, confidence_score).
    fn match_triggers(&self, user_input: &str) -> Vec<(String, f32)>;

    /// Installs a skill from the TrueNorth curated registry.
    ///
    /// Fetches the skill file from `skills.truenorth.dev`, validates it,
    /// and writes it to the local skills directory.
    async fn install_from_registry(
        &self,
        skill_name: &str,
    ) -> Result<Box<dyn Skill>, SkillError>;

    /// Watches the skills directory for changes and reloads modified skills.
    ///
    /// Uses the `notify` crate internally. Non-blocking — spawns a background task.
    async fn watch_directory(&self) -> Result<(), SkillError>;

    /// Returns a skill by name if it is currently loaded.
    fn get_skill(&self, skill_name: &str) -> Option<Box<dyn Skill>>;
}
