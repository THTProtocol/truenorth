//! Skill registry — thread-safe in-memory index of all discovered skills.
//!
//! [`SkillRegistry`] is the authoritative list of installed skills.  It is
//! populated at startup by scanning the `/skills/` directory, and updated
//! whenever new skills are installed or removed.
//!
//! The registry is safe for concurrent access: internal state is protected by
//! an [`std::sync::RwLock`].

use std::sync::{Arc, RwLock};

use tracing::{debug, info};

use crate::installer::SkillInstaller;
use crate::trigger::{SkillMatch, TriggerMatcher};
use truenorth_core::traits::skill::SkillError;
use truenorth_core::types::skill::{SkillLoadLevel, SkillMetadata};

/// Thread-safe in-memory registry of all discovered skills.
///
/// Skills are stored as [`SkillMetadata`] (Level 0 / Minimal).  Higher-level
/// content is managed by [`crate::loader::DefaultSkillLoader`].
///
/// # Thread safety
///
/// All public methods are safe to call from multiple threads concurrently.
/// Read operations (list, get, find) take a shared read lock; write operations
/// (register, unregister) take an exclusive write lock.
#[derive(Debug, Clone)]
pub struct SkillRegistry {
    inner: Arc<RwLock<RegistryInner>>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    /// Skills indexed by their canonical name (case-preserving).
    skills: std::collections::HashMap<String, SkillMetadata>,
}

impl SkillRegistry {
    /// Creates an empty [`SkillRegistry`].
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner::default())),
        }
    }

    /// Registers a skill in the registry.
    ///
    /// If a skill with the same name already exists it is replaced (useful for
    /// hot-reload after the skill file changes).
    pub fn register(&self, skill: SkillMetadata) {
        let name = skill.name.clone();
        let mut guard = self.inner.write().expect("SkillRegistry lock poisoned");
        debug!(name = %name, "Registering skill");
        guard.skills.insert(name, skill);
    }

    /// Removes a skill from the registry by name.
    ///
    /// Returns `true` if the skill was present and removed, `false` if it was
    /// not found.
    pub fn unregister(&self, skill_name: &str) -> bool {
        let mut guard = self.inner.write().expect("SkillRegistry lock poisoned");
        let removed = guard.skills.remove(skill_name).is_some();
        if removed {
            debug!(name = %skill_name, "Unregistered skill");
        }
        removed
    }

    /// Returns a cloned snapshot of the skill metadata for `skill_id`, or
    /// `None` if no skill with that name is registered.
    pub fn get(&self, skill_id: &str) -> Option<SkillMetadata> {
        let guard = self.inner.read().expect("SkillRegistry lock poisoned");
        guard.skills.get(skill_id).cloned()
    }

    /// Returns a snapshot of all registered skill metadata, sorted by name.
    ///
    /// The snapshot is a `Vec` because the underlying lock is released
    /// immediately after cloning.
    pub fn list(&self) -> Vec<SkillMetadata> {
        let guard = self.inner.read().expect("SkillRegistry lock poisoned");
        let mut skills: Vec<SkillMetadata> = guard.skills.values().cloned().collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }

    /// Returns the number of skills currently registered.
    pub fn len(&self) -> usize {
        let guard = self.inner.read().expect("SkillRegistry lock poisoned");
        guard.skills.len()
    }

    /// Returns `true` when the registry contains no skills.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Finds skills whose trigger phrases match the given prompt.
    ///
    /// Delegates to [`TriggerMatcher`] and returns matches sorted by score
    /// descending.  Returns an empty `Vec` if no skills match.
    pub fn find_by_trigger(&self, prompt: &str) -> Vec<SkillMatch> {
        let skills = self.list();
        TriggerMatcher::new().match_triggers(prompt, &skills)
    }

    /// Downloads, validates, and registers a skill from a URL.
    ///
    /// The skill Markdown is saved to `target_dir/community/` and then
    /// registered in memory.
    ///
    /// # Errors
    ///
    /// Returns a [`SkillError`] if:
    /// - The HTTP request fails.
    /// - The downloaded content is not a valid SKILL.md file.
    /// - The skill fails validation.
    pub async fn install_from_url(
        &self,
        url: &str,
        target_dir: &std::path::Path,
    ) -> Result<SkillMetadata, SkillError> {
        info!(url = %url, "Installing skill from URL");

        let installer = SkillInstaller::new();
        let community_dir = target_dir.join("community");
        std::fs::create_dir_all(&community_dir)?;

        let parsed = installer.install_from_url(url, &community_dir).await?;

        let metadata = SkillMetadata {
            name: parsed.frontmatter.name.clone(),
            version: parsed.frontmatter.version.clone(),
            description: parsed.frontmatter.description.clone(),
            triggers: parsed.frontmatter.triggers.clone(),
            tags: parsed.frontmatter.tags.clone(),
            is_active: true,
            loaded_at: SkillLoadLevel::Minimal,
        };

        self.register(metadata.clone());
        info!(name = %metadata.name, "Skill installed and registered from URL");
        Ok(metadata)
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metadata(name: &str) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Description of {}", name),
            triggers: vec!["test".to_string()],
            tags: vec![],
            is_active: true,
            loaded_at: SkillLoadLevel::Minimal,
        }
    }

    #[test]
    fn register_and_get() {
        let registry = SkillRegistry::new();
        registry.register(make_metadata("Alpha"));
        assert!(registry.get("Alpha").is_some());
        assert!(registry.get("Beta").is_none());
    }

    #[test]
    fn list_sorted_by_name() {
        let registry = SkillRegistry::new();
        registry.register(make_metadata("Zeta"));
        registry.register(make_metadata("Alpha"));
        registry.register(make_metadata("Mango"));
        let list = registry.list();
        assert_eq!(list[0].name, "Alpha");
        assert_eq!(list[1].name, "Mango");
        assert_eq!(list[2].name, "Zeta");
    }

    #[test]
    fn unregister_removes_skill() {
        let registry = SkillRegistry::new();
        registry.register(make_metadata("Alpha"));
        assert!(registry.unregister("Alpha"));
        assert!(registry.get("Alpha").is_none());
        assert!(!registry.unregister("Alpha")); // already removed
    }

    #[test]
    fn find_by_trigger_delegates_to_matcher() {
        let registry = SkillRegistry::new();
        let mut meta = make_metadata("Research");
        meta.triggers = vec!["research".to_string()];
        registry.register(meta);
        let matches = registry.find_by_trigger("I want to research AI");
        assert!(!matches.is_empty());
        assert_eq!(matches[0].skill_name, "Research");
    }

    #[test]
    fn len_and_is_empty() {
        let registry = SkillRegistry::new();
        assert!(registry.is_empty());
        registry.register(make_metadata("One"));
        assert_eq!(registry.len(), 1);
        registry.register(make_metadata("Two"));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn re_register_replaces() {
        let registry = SkillRegistry::new();
        registry.register(make_metadata("Alpha"));
        let mut updated = make_metadata("Alpha");
        updated.version = "2.0.0".to_string();
        registry.register(updated);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("Alpha").unwrap().version, "2.0.0");
    }
}
