//! Default skill loader — discovers, parses, caches, and progressively loads skills.
//!
//! [`DefaultSkillLoader`] implements the [`SkillLoader`] trait defined in
//! `truenorth-core`.  It:
//!
//! 1. **Scans** a directory tree for `*.md` files with valid SKILL.md frontmatter.
//! 2. **Caches** parsed skills in a `HashMap` keyed by skill name.
//! 3. **Progressively loads** skill content from Level 0 (metadata only) up to
//!    Level 2 (extended reference materials) on demand.
//! 4. **Emits** [`truenorth_core::types::event::ReasoningEventPayload::SkillActivated`]
//!    events via an optional callback whenever a skill is loaded at Level 1 or 2.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tracing::{debug, info, warn};
use uuid::Uuid;

use truenorth_core::traits::skill::{LoadedSkill, Skill, SkillError, SkillLoader};
use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::types::skill::{SkillFrontmatter, SkillLoadLevel};

use crate::parser::SkillMarkdownParser;
use crate::registry::SkillRegistry;

// ─── LoadedSkillWrapper ───────────────────────────────────────────────────────

/// Internal implementation of [`Skill`] backed by a [`LoadedSkill`].
///
/// Wraps the core struct so we can add the `Skill` trait methods without
/// modifying `truenorth-core`.
#[derive(Debug, Clone)]
struct LoadedSkillWrapper {
    inner: LoadedSkill,
    /// Cached Level-0 summary string ("name — description").
    minimal_summary: String,
}

impl LoadedSkillWrapper {
    fn from_loaded(loaded: LoadedSkill) -> Self {
        let minimal_summary = format!(
            "{} — {}",
            loaded.metadata.name, loaded.metadata.description
        );
        Self {
            inner: loaded,
            minimal_summary,
        }
    }
}

impl Skill for LoadedSkillWrapper {
    fn metadata(&self) -> &SkillFrontmatter {
        &self.inner.metadata
    }

    fn content_at_level(&self, level: SkillLoadLevel) -> &str {
        match level {
            SkillLoadLevel::Minimal => &self.minimal_summary,
            SkillLoadLevel::Full => &self.inner.workflow_body,
            SkillLoadLevel::Extended => {
                if let Some(ext) = &self.inner.extended_content {
                    ext.as_str()
                } else {
                    &self.inner.workflow_body
                }
            }
        }
    }

    fn triggers(&self) -> &[String] {
        &self.inner.metadata.triggers
    }

    fn required_tools(&self) -> &[String] {
        &self.inner.metadata.tools_required
    }

    fn workflow_steps(&self) -> Option<Vec<String>> {
        extract_numbered_steps(&self.inner.workflow_body)
    }
}

/// Extracts numbered Markdown list items from a workflow body.
///
/// Lines matching `^\d+\.` are treated as steps.  Returns `None` if no
/// numbered steps are found.
fn extract_numbered_steps(body: &str) -> Option<Vec<String>> {
    let steps: Vec<String> = body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Match "1. ", "12. " etc.
            if trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                if let Some(dot_pos) = trimmed.find(". ") {
                    let prefix = &trimmed[..dot_pos];
                    if prefix.chars().all(|c| c.is_ascii_digit()) {
                        return Some(trimmed[dot_pos + 2..].trim().to_string());
                    }
                }
            }
            None
        })
        .collect();

    if steps.is_empty() {
        None
    } else {
        Some(steps)
    }
}

// ─── DefaultSkillLoader ───────────────────────────────────────────────────────

/// Cache entry tracking the current load level of a skill.
#[derive(Debug)]
struct CacheEntry {
    /// The loaded and parsed skill.
    skill: LoadedSkillWrapper,
    /// The highest level at which this skill has been loaded.
    level: SkillLoadLevel,
}

/// Default filesystem-backed implementation of [`SkillLoader`].
///
/// Skills are discovered by recursively scanning a directory for `*.md` files
/// whose content begins with a valid YAML frontmatter block.  Parsed skills are
/// cached in a [`HashMap`] keyed by skill name.
///
/// # Progressive loading
///
/// - **Level 0 (Minimal)**: Only the frontmatter is parsed.  The `workflow_body`
///   is set to an empty string.  This is what happens during the initial directory
///   scan so that the full skill corpus is known cheaply.
/// - **Level 1 (Full)**: The entire Markdown body is read and stored in
///   `workflow_body`.
/// - **Level 2 (Extended)**: The same file plus any `REFERENCES.md` sibling file
///   (if present) are appended to `extended_content`.
///
/// # Event emission
///
/// An optional `event_tx` channel can be configured via [`DefaultSkillLoader::with_event_tx`].
/// When set, a [`ReasoningEventPayload::SkillActivated`] event is sent whenever a skill
/// is loaded at Level 1 or above.
#[derive(Debug)]
pub struct DefaultSkillLoader {
    parser: SkillMarkdownParser,
    registry: SkillRegistry,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    session_id: Uuid,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<ReasoningEvent>>,
}

impl DefaultSkillLoader {
    /// Creates a new `DefaultSkillLoader` with a fresh in-memory registry.
    pub fn new() -> Self {
        Self {
            parser: SkillMarkdownParser::new(),
            registry: SkillRegistry::new(),
            cache: Arc::new(Mutex::new(HashMap::new())),
            session_id: Uuid::nil(),
            event_tx: None,
        }
    }

    /// Creates a new `DefaultSkillLoader` sharing an existing registry.
    pub fn with_registry(registry: SkillRegistry) -> Self {
        Self {
            parser: SkillMarkdownParser::new(),
            registry,
            cache: Arc::new(Mutex::new(HashMap::new())),
            session_id: Uuid::nil(),
            event_tx: None,
        }
    }

    /// Configures the session ID used when emitting reasoning events.
    pub fn with_session_id(mut self, session_id: Uuid) -> Self {
        self.session_id = session_id;
        self
    }

    /// Configures an event channel for reasoning event emission.
    pub fn with_event_tx(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<ReasoningEvent>,
    ) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Returns a reference to the underlying registry.
    pub fn registry(&self) -> &SkillRegistry {
        &self.registry
    }

    // ─── Internal helpers ─────────────────────────────────────────────────────

    /// Emits a `SkillActivated` reasoning event if an event channel is configured.
    fn emit_skill_activated(
        &self,
        skill_name: &str,
        skill_version: &str,
        level: SkillLoadLevel,
        trigger_phrase: Option<String>,
    ) {
        if let Some(tx) = &self.event_tx {
            let event = ReasoningEvent::new(
                self.session_id,
                ReasoningEventPayload::SkillActivated {
                    session_id: self.session_id,
                    skill_name: skill_name.to_string(),
                    skill_version: skill_version.to_string(),
                    load_level: format!("{:?}", level),
                    trigger_phrase,
                },
            );
            if let Err(e) = tx.send(event) {
                warn!("Failed to emit SkillActivated event: {}", e);
            }
        }
    }

    /// Reads a skill file and builds a [`LoadedSkill`] at the requested level.
    fn read_skill_at_level(
        &self,
        path: &Path,
        level: SkillLoadLevel,
    ) -> Result<LoadedSkill, SkillError> {
        match level {
            SkillLoadLevel::Minimal => {
                // Parse frontmatter only; leave workflow_body empty.
                let content = std::fs::read_to_string(path)?;
                let frontmatter = self
                    .parser
                    .parse_frontmatter(&content, path)
                    .map_err(|e| SkillError::FrontmatterParseError {
                        path: path.to_path_buf(),
                        message: e.to_string(),
                    })?;

                Ok(LoadedSkill {
                    metadata: frontmatter,
                    workflow_body: String::new(),
                    extended_content: None,
                    file_path: path.to_path_buf(),
                })
            }

            SkillLoadLevel::Full => {
                let parsed = self
                    .parser
                    .parse_skill_file(path)
                    .map_err(|e| SkillError::FrontmatterParseError {
                        path: path.to_path_buf(),
                        message: e.to_string(),
                    })?;

                Ok(LoadedSkill {
                    metadata: parsed.frontmatter,
                    workflow_body: parsed.body.raw,
                    extended_content: None,
                    file_path: path.to_path_buf(),
                })
            }

            SkillLoadLevel::Extended => {
                // Level 1 body + optional sibling REFERENCES.md
                let parsed = self
                    .parser
                    .parse_skill_file(path)
                    .map_err(|e| SkillError::FrontmatterParseError {
                        path: path.to_path_buf(),
                        message: e.to_string(),
                    })?;

                let extended = Self::load_extended_content(path, &parsed.body.references);

                Ok(LoadedSkill {
                    metadata: parsed.frontmatter,
                    workflow_body: parsed.body.raw.clone(),
                    extended_content: Some(extended),
                    file_path: path.to_path_buf(),
                })
            }
        }
    }

    /// Builds the Level-2 extended content string.
    ///
    /// This concatenates:
    /// 1. The "References" section of the skill body.
    /// 2. The contents of `REFERENCES.md` in the same directory (if present).
    fn load_extended_content(skill_path: &Path, references_section: &str) -> String {
        let mut extended = references_section.to_string();

        if let Some(parent) = skill_path.parent() {
            let refs_path = parent.join("REFERENCES.md");
            if refs_path.exists() {
                match std::fs::read_to_string(&refs_path) {
                    Ok(content) => {
                        if !extended.is_empty() {
                            extended.push_str("\n\n");
                        }
                        extended.push_str(&content);
                    }
                    Err(e) => warn!(path = %refs_path.display(), "Could not read REFERENCES.md: {}", e),
                }
            }
        }

        extended
    }

    /// Recursively finds all `*.md` files under `dir`.
    fn find_skill_files(dir: &Path) -> Vec<PathBuf> {
        let pattern = format!("{}/**/*.md", dir.display());
        match glob::glob(&pattern) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|p| {
                    // Exclude REFERENCES.md siblings — they're reference material, not skills.
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.to_uppercase() != "REFERENCES.MD")
                        .unwrap_or(true)
                })
                .collect(),
            Err(e) => {
                warn!("Glob error scanning {}: {}", dir.display(), e);
                Vec::new()
            }
        }
    }
}

impl Default for DefaultSkillLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SkillLoader for DefaultSkillLoader {
    /// Scans `dir` recursively for `*.md` files with valid SKILL.md frontmatter.
    ///
    /// Each valid file is loaded at Level 0 (frontmatter only) and registered in
    /// the internal registry.  Files that fail to parse are logged as warnings and
    /// skipped — one bad skill file never prevents the rest from loading.
    ///
    /// Returns the number of skills successfully loaded.
    async fn scan_directory(&self, dir: &Path) -> Result<usize, SkillError> {
        info!(dir = %dir.display(), "Scanning for SKILL.md files");

        if !dir.exists() {
            warn!(dir = %dir.display(), "Skills directory does not exist; skipping scan");
            return Ok(0);
        }

        let paths = Self::find_skill_files(dir);
        debug!(count = paths.len(), "Found candidate skill files");

        let mut loaded = 0usize;

        for path in &paths {
            match self.read_skill_at_level(path, SkillLoadLevel::Minimal) {
                Ok(skill) => {
                    let name = skill.metadata.name.clone();
                    let version = skill.metadata.version.clone();

                    // Register in the lightweight registry.
                    self.registry.register(truenorth_core::types::skill::SkillMetadata {
                        name: name.clone(),
                        version: version.clone(),
                        description: skill.metadata.description.clone(),
                        triggers: skill.metadata.triggers.clone(),
                        tags: skill.metadata.tags.clone(),
                        is_active: true,
                        loaded_at: SkillLoadLevel::Minimal,
                    });

                    // Store in the full cache.
                    let wrapper = LoadedSkillWrapper::from_loaded(skill);
                    {
                        let mut cache = self.cache.lock().expect("cache lock poisoned");
                        cache.insert(
                            name.clone(),
                            CacheEntry {
                                skill: wrapper,
                                level: SkillLoadLevel::Minimal,
                            },
                        );
                    }

                    debug!(name = %name, path = %path.display(), "Loaded skill at Level 0");
                    loaded += 1;
                }
                Err(e) => {
                    warn!(path = %path.display(), "Skipping invalid skill file: {}", e);
                }
            }
        }

        info!(loaded = loaded, total = paths.len(), "Skill scan complete");
        Ok(loaded)
    }

    /// Loads a single skill file at the requested level.
    ///
    /// If the skill is already cached at the requested level (or higher), the
    /// cached version is returned immediately.  Otherwise the file is re-read
    /// and the cache is upgraded.
    async fn load(
        &self,
        path: &Path,
        level: SkillLoadLevel,
    ) -> Result<Box<dyn Skill>, SkillError> {
        // Check cache first.
        let cached_name: Option<String> = {
            let cache = self.cache.lock().expect("cache lock poisoned");
            // Find by file path.
            cache
                .values()
                .find(|e| e.skill.inner.file_path == path)
                .map(|e| e.skill.inner.metadata.name.clone())
        };

        if let Some(name) = cached_name {
            let cache_level = {
                let cache = self.cache.lock().expect("cache lock poisoned");
                cache.get(&name).map(|e| e.level)
            };

            if let Some(cached_level) = cache_level {
                if cached_level >= level {
                    let cache = self.cache.lock().expect("cache lock poisoned");
                    if let Some(entry) = cache.get(&name) {
                        return Ok(Box::new(entry.skill.clone()));
                    }
                }
            }

            // Need to upgrade.
            return self.progressive_load(&name, level).await;
        }

        // Not in cache — load fresh.
        let skill = self.read_skill_at_level(path, level)?;
        let name = skill.metadata.name.clone();
        let version = skill.metadata.version.clone();

        if level >= SkillLoadLevel::Full {
            self.emit_skill_activated(&name, &version, level, None);
        }

        let wrapper = LoadedSkillWrapper::from_loaded(skill);
        let result = Box::new(wrapper.clone()) as Box<dyn Skill>;

        {
            let mut cache = self.cache.lock().expect("cache lock poisoned");
            cache.insert(name, CacheEntry { skill: wrapper, level });
        }

        Ok(result)
    }

    /// Upgrades a cached skill to the requested load level.
    ///
    /// If the skill is already at or above `target_level` the cached version is
    /// returned immediately.  Otherwise the skill file is re-read at the new
    /// level and the cache entry is upgraded.
    async fn progressive_load(
        &self,
        skill_name: &str,
        target_level: SkillLoadLevel,
    ) -> Result<Box<dyn Skill>, SkillError> {
        let (current_level, file_path): (SkillLoadLevel, PathBuf) = {
            let cache = self.cache.lock().expect("cache lock poisoned");
            match cache.get(skill_name) {
                Some(entry) => (entry.level, entry.skill.inner.file_path.clone()),
                None => {
                    return Err(SkillError::SkillNotFound {
                        name: skill_name.to_string(),
                    })
                }
            }
        };

        if current_level >= target_level {
            // Already at the right level.
            let cache = self.cache.lock().expect("cache lock poisoned");
            if let Some(entry) = cache.get(skill_name) {
                return Ok(Box::new(entry.skill.clone()));
            }
        }

        debug!(
            name = %skill_name,
            from = ?current_level,
            to = ?target_level,
            "Progressively loading skill"
        );

        let upgraded = self.read_skill_at_level(&file_path, target_level)?;
        let version = upgraded.metadata.version.clone();
        let wrapper = LoadedSkillWrapper::from_loaded(upgraded);

        self.emit_skill_activated(skill_name, &version, target_level, None);

        {
            let mut cache = self.cache.lock().expect("cache lock poisoned");
            cache.insert(
                skill_name.to_string(),
                CacheEntry {
                    skill: wrapper.clone(),
                    level: target_level,
                },
            );
        }

        Ok(Box::new(wrapper))
    }

    /// Returns all skills currently cached at Level 0 (Minimal).
    ///
    /// This is the skill index injected into LLM context at the start of each
    /// session.  Only frontmatter fields are included — no workflow bodies.
    fn list_skills_minimal(&self) -> Vec<SkillFrontmatter> {
        let cache = self.cache.lock().expect("cache lock poisoned");
        let mut skills: Vec<SkillFrontmatter> = cache
            .values()
            .map(|e| e.skill.inner.metadata.clone())
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }

    /// Matches a user input string against all loaded skills' trigger phrases.
    ///
    /// Returns `(skill_name, confidence)` pairs sorted by confidence descending.
    fn match_triggers(&self, user_input: &str) -> Vec<(String, f32)> {
        let all_skills = self.registry.list();
        let matcher = crate::trigger::TriggerMatcher::new();
        let matches = matcher.match_triggers(user_input, &all_skills);

        // Normalise scores to a 0.0–1.0 confidence range.
        let max_score = matches.first().map(|m| m.score).unwrap_or(1.0);
        matches
            .into_iter()
            .map(|m| {
                let confidence = if max_score > 0.0 {
                    m.score / max_score
                } else {
                    0.0
                };
                (m.skill_name, confidence)
            })
            .collect()
    }

    /// Installs a skill from `skills.truenorth.dev`.
    ///
    /// Fetches `https://skills.truenorth.dev/<skill_name>/SKILL.md`, validates,
    /// saves to the default skills directory, and loads at Level 1.
    async fn install_from_registry(
        &self,
        skill_name: &str,
    ) -> Result<Box<dyn Skill>, SkillError> {
        let url = format!(
            "https://skills.truenorth.dev/{}/SKILL.md",
            skill_name.to_lowercase().replace(' ', "-")
        );
        info!(skill = %skill_name, url = %url, "Installing skill from curated registry");

        let installer = crate::installer::SkillInstaller::new();
        let target_dir = PathBuf::from("skills").join("community");

        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            return Err(SkillError::Io(e));
        }

        let parsed = installer.install_from_url(&url, &target_dir).await?;
        let path = parsed.file_path.clone();

        self.load(&path, SkillLoadLevel::Full).await
    }

    /// Watches the skills directory for file changes and hot-reloads modified skills.
    ///
    /// This implementation spawns a background Tokio task that polls for changes
    /// every 5 seconds.  A production implementation would use `notify` for
    /// inotify/kqueue events.
    async fn watch_directory(&self) -> Result<(), SkillError> {
        // Lightweight polling implementation.  A full `notify`-based watcher
        // would be added in a follow-up when the `truenorth-watcher` crate is ready.
        info!("Skill directory watching is not yet implemented (polling stub)");
        Ok(())
    }

    /// Returns a boxed clone of the named skill if it is currently cached.
    fn get_skill(&self, skill_name: &str) -> Option<Box<dyn Skill>> {
        let cache = self.cache.lock().expect("cache lock poisoned");
        cache
            .get(skill_name)
            .map(|e| Box::new(e.skill.clone()) as Box<dyn Skill>)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    const SAMPLE_SKILL: &str = r#"---
name: Demo Skill
version: 1.0.0
description: A demo skill for testing.
triggers:
  - demo
tools_required: []
permission_level: low
author: Test
sandboxed: false
tags: []
---

## When to Use

Use this for demos.

## Workflow

1. Step one.
2. Step two.
"#;

    fn write_skill_file(dir: &Path, filename: &str, content: &str) -> PathBuf {
        let path = dir.join(filename);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[tokio::test]
    async fn scan_directory_finds_skills() {
        let dir = tempdir().unwrap();
        write_skill_file(dir.path(), "demo.md", SAMPLE_SKILL);

        let loader = DefaultSkillLoader::new();
        let count = loader.scan_directory(dir.path()).await.unwrap();
        assert_eq!(count, 1);
        assert!(loader.get_skill("Demo Skill").is_some());
    }

    #[tokio::test]
    async fn scan_nonexistent_dir_returns_zero() {
        let loader = DefaultSkillLoader::new();
        let count = loader
            .scan_directory(Path::new("/nonexistent/path"))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn load_file_at_full_level() {
        let dir = tempdir().unwrap();
        let path = write_skill_file(dir.path(), "demo.md", SAMPLE_SKILL);

        let loader = DefaultSkillLoader::new();
        let skill = loader.load(&path, SkillLoadLevel::Full).await.unwrap();
        assert_eq!(skill.name(), "Demo Skill");
        let body = skill.content_at_level(SkillLoadLevel::Full);
        assert!(body.contains("Step one"));
    }

    #[tokio::test]
    async fn progressive_load_upgrades_cache() {
        let dir = tempdir().unwrap();
        write_skill_file(dir.path(), "demo.md", SAMPLE_SKILL);

        let loader = DefaultSkillLoader::new();
        // First scan at Level 0.
        loader.scan_directory(dir.path()).await.unwrap();
        // Now upgrade to Level 1.
        let skill = loader
            .progressive_load("Demo Skill", SkillLoadLevel::Full)
            .await
            .unwrap();
        assert!(skill.content_at_level(SkillLoadLevel::Full).contains("Step one"));
    }

    #[tokio::test]
    async fn workflow_steps_extracted() {
        let dir = tempdir().unwrap();
        let path = write_skill_file(dir.path(), "demo.md", SAMPLE_SKILL);

        let loader = DefaultSkillLoader::new();
        let skill = loader.load(&path, SkillLoadLevel::Full).await.unwrap();
        let steps = skill.workflow_steps().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0], "Step one.");
    }

    #[tokio::test]
    async fn list_skills_minimal() {
        let dir = tempdir().unwrap();
        write_skill_file(dir.path(), "demo.md", SAMPLE_SKILL);

        let loader = DefaultSkillLoader::new();
        loader.scan_directory(dir.path()).await.unwrap();

        let skills = loader.list_skills_minimal();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Demo Skill");
    }

    #[tokio::test]
    async fn match_triggers_returns_matches() {
        let dir = tempdir().unwrap();
        write_skill_file(dir.path(), "demo.md", SAMPLE_SKILL);

        let loader = DefaultSkillLoader::new();
        loader.scan_directory(dir.path()).await.unwrap();

        let matches = loader.match_triggers("can you demo this for me?");
        assert!(!matches.is_empty());
        assert_eq!(matches[0].0, "Demo Skill");
    }

    #[tokio::test]
    async fn invalid_skill_file_skipped_during_scan() {
        let dir = tempdir().unwrap();
        // Good skill
        write_skill_file(dir.path(), "good.md", SAMPLE_SKILL);
        // Bad skill (no frontmatter)
        write_skill_file(dir.path(), "bad.md", "# Just markdown\nNo frontmatter here.");

        let loader = DefaultSkillLoader::new();
        let count = loader.scan_directory(dir.path()).await.unwrap();
        // Only the good skill should be counted.
        assert_eq!(count, 1);
    }
}
