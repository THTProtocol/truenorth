/// Skill types — the SKILL.md-compatible skill system for TrueNorth.
///
/// Skills are instruction documents that guide the agent through complex
/// workflows. They follow the SKILL.md open standard for cross-agent
/// compatibility. Skills are not executed code — they are structured
/// markdown that the orchestrator reads and follows.

use serde::{Deserialize, Serialize};

/// The three levels of skill content that can be loaded progressively.
///
/// This is the Hermes/DeerFlow progressive disclosure pattern. Loading
/// skills incrementally keeps the system prompt lean: all skills are known
/// at Level 0, but only active skills are loaded at Level 1 or 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillLoadLevel {
    /// Level 0: Name, description, and triggers only (~80 tokens per skill).
    ///
    /// Always loaded for all installed skills. Used for skill selection —
    /// the LLM can see all available skills and pick the right one.
    Minimal,

    /// Level 1: Full workflow markdown body (~800–2000 tokens).
    ///
    /// Loaded when a skill is triggered. Contains the complete workflow
    /// instructions the agent follows to execute the skill.
    Full,

    /// Level 2: Reference materials, templates, and examples (variable size).
    ///
    /// Loaded on demand within an executing skill when the workflow body
    /// references additional context. May be very large for reference-heavy skills.
    Extended,
}

/// The parsed and validated frontmatter of a SKILL.md file.
///
/// All fields match the SKILL.md open standard for cross-agent compatibility.
/// TrueNorth-authored skills are SKILL.md-compatible by definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    /// The skill's canonical display name.
    pub name: String,
    /// Semantic version string (e.g., "1.0.0").
    pub version: String,
    /// One-sentence description shown in skill listings.
    pub description: String,
    /// Trigger phrases: the skill activates when user input matches any of these.
    pub triggers: Vec<String>,
    /// Tools that this skill's workflow requires. Validated at load time against
    /// the tool registry — missing required tools cause a `SkillError`.
    pub tools_required: Vec<String>,
    /// The permission level required to run this skill ("low", "medium", or "high").
    pub permission_level: String,
    /// Skill author identifier (name or organization).
    pub author: String,
    /// Whether this skill runs in the WASM sandbox.
    /// True by default for community skills; false for built-in Rust skills.
    #[serde(default)]
    pub sandboxed: bool,
    /// Tags for skill discovery and categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// URL to the skill's source in the registry (if community skill).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Minimum TrueNorth version required to run this skill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_truenorth_version: Option<String>,
}

/// A trigger definition for a skill.
///
/// Triggers are phrases that cause the skill to be automatically activated
/// when they appear in user input. They support exact match and pattern match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTrigger {
    /// The trigger phrase or pattern.
    pub phrase: String,
    /// Whether this is a regex pattern (true) or an exact/substring match (false).
    pub is_pattern: bool,
    /// Confidence threshold for fuzzy matching (0.0–1.0).
    pub confidence_threshold: f32,
}

/// Lightweight skill metadata for listing and selection.
///
/// Used when building the skill index injected into the LLM context.
/// Contains only what the LLM needs to decide which skill to use —
/// no heavy workflow body or reference materials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// The skill's canonical name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// One-sentence description.
    pub description: String,
    /// Trigger phrases for this skill.
    pub triggers: Vec<String>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Whether the skill is currently active and usable.
    pub is_active: bool,
    /// Load level at which this skill is currently loaded.
    pub loaded_at: SkillLoadLevel,
}
