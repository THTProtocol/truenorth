//! # truenorth-skills
//!
//! Skill loading, SKILL.md parsing, progressive disclosure, and community skill installation
//! for the TrueNorth agent system.
//!
//! ## Architecture
//!
//! Skills are instruction documents in the SKILL.md open standard. They are structured
//! Markdown files with YAML frontmatter that guide the agent through complex workflows.
//! This crate provides:
//!
//! - **[`parser::SkillMarkdownParser`]** — Parse SKILL.md files into typed Rust structs.
//! - **[`loader::DefaultSkillLoader`]** — Discover, load, and cache skills from the filesystem
//!   with progressive disclosure (Level 0 → 1 → 2).
//! - **[`registry::SkillRegistry`]** — Thread-safe in-memory registry of all discovered skills.
//! - **[`trigger::TriggerMatcher`]** — Match user prompts against skill trigger phrases.
//! - **[`validator::SkillValidator`]** — Validate skill files for required fields and security.
//! - **[`installer::SkillInstaller`]** — Install skills from filesystem paths or URLs.
//!
//! ## SKILL.md Format
//!
//! ```markdown
//! ---
//! name: Research Assistant
//! version: 1.0.0
//! description: Guides the agent through deep multi-source research workflows.
//! triggers:
//!   - research
//!   - investigate
//!   - deep dive
//! tools_required:
//!   - search_web
//!   - fetch_url
//! permission_level: low
//! author: TrueNorth Team
//! sandboxed: false
//! tags:
//!   - research
//!   - information
//! ---
//!
//! ## When to Use
//!
//! Use this skill when the user asks for comprehensive research on a topic...
//! ```

pub mod installer;
pub mod loader;
pub mod parser;
pub mod registry;
pub mod trigger;
pub mod validator;

// Re-export primary public types for ergonomic use.

pub use installer::SkillInstaller;
pub use loader::DefaultSkillLoader;
pub use parser::{ParsedSkill, SkillBody, SkillMarkdownParser};
pub use registry::SkillRegistry;
pub use trigger::{SkillMatch, TriggerMatcher};
pub use validator::{SkillValidator, ValidationError};

// Re-export core types used by consumers of this crate.
pub use truenorth_core::traits::skill::{LoadedSkill, Skill, SkillError, SkillLoader};
pub use truenorth_core::types::skill::{
    SkillFrontmatter, SkillLoadLevel, SkillMetadata, SkillTrigger,
};
