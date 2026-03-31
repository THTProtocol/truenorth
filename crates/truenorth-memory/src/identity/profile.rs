//! `UserProfile` — structured representation of persistent user preferences.
//!
//! The profile is the schema for identity-tier memory. It is serialized to JSON
//! and stored in the identity SQLite database. The dialectic modeler updates
//! the profile as it observes and confirms user patterns.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Communication style observed from user interactions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommunicationStyle {
    /// User prefers responses in bullet-point or list format.
    BulletPoints,
    /// User prefers flowing prose responses.
    Prose,
    /// User prefers code blocks with minimal surrounding text.
    CodeFirst,
    /// User prefers step-by-step numbered lists.
    Numbered,
    /// Style not yet determined.
    Unknown,
}

impl Default for CommunicationStyle {
    fn default() -> Self {
        Self::Unknown
    }
}

/// A single inferred workflow pattern.
///
/// Patterns are observational: the dialectic modeler injects them when it
/// detects recurring behaviors. The `confirmed` flag is set to `true` only
/// after the user explicitly agrees with the inference via a nudge question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPattern {
    /// Short description of the inferred pattern (e.g., "prefers TDD").
    pub description: String,
    /// Whether the user has confirmed this pattern via a nudge question.
    pub confirmed: bool,
    /// When this pattern was first inferred.
    pub inferred_at: DateTime<Utc>,
    /// When the user confirmed (or rejected) this pattern. None if pending.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// Confidence score from 0.0 (speculative) to 1.0 (highly confident).
    pub confidence: f32,
    /// Number of observations that support this inference.
    pub observation_count: u32,
}

impl WorkflowPattern {
    /// Create a new unconfirmed workflow pattern.
    pub fn new(description: impl Into<String>, confidence: f32) -> Self {
        Self {
            description: description.into(),
            confirmed: false,
            inferred_at: Utc::now(),
            confirmed_at: None,
            confidence: confidence.clamp(0.0, 1.0),
            observation_count: 1,
        }
    }

    /// Mark this pattern as confirmed by the user.
    pub fn confirm(&mut self) {
        self.confirmed = true;
        self.confirmed_at = Some(Utc::now());
        self.confidence = 1.0;
        debug!("WorkflowPattern confirmed: {}", self.description);
    }

    /// Mark this pattern as rejected by the user.
    ///
    /// Rejected patterns retain their data for audit purposes but are excluded
    /// from active context injection.
    pub fn reject(&mut self) {
        self.confirmed = false;
        self.confirmed_at = Some(Utc::now());
        self.confidence = 0.0;
        debug!("WorkflowPattern rejected: {}", self.description);
    }

    /// Record an additional observation supporting this pattern.
    pub fn observe(&mut self) {
        self.observation_count += 1;
        // Increase confidence by 10% per additional observation, capped at 0.9 until confirmed.
        self.confidence = (self.confidence + 0.1).min(0.9);
    }
}

/// Persistent user profile for the identity memory tier.
///
/// Serialized as a JSON blob and stored in the identity SQLite database. The
/// profile is loaded at session start and updated throughout the session by
/// the dialectic modeler.
///
/// All fields are optional so the profile can be incrementally built up over
/// time without requiring a complete initial setup.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserProfile {
    /// User's display name (if set via `truenorth profile set-name`).
    pub name: Option<String>,

    /// General preferences as key-value pairs.
    ///
    /// Examples:
    /// - `"output_format"` → `"markdown"`
    /// - `"code_language"` → `"rust"`
    /// - `"timezone"` → `"Europe/Lisbon"`
    pub preferences: HashMap<String, String>,

    /// Inferred communication style.
    pub communication_style: CommunicationStyle,

    /// Inferred workflow patterns, keyed by a short slug.
    ///
    /// Key convention: lowercase, hyphen-separated (e.g., `"tdd-preferred"`).
    pub workflow_patterns: HashMap<String, WorkflowPattern>,

    /// User's inferred roles (e.g., "rust-developer", "blockchain-engineer").
    pub roles: Vec<String>,

    /// When the profile was first created.
    pub created_at: DateTime<Utc>,

    /// When the profile was last updated.
    pub updated_at: DateTime<Utc>,

    /// Total number of sessions observed, used for pattern inference weighting.
    pub sessions_observed: u32,
}

impl UserProfile {
    /// Create a new empty user profile.
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            updated_at: now,
            ..Default::default()
        }
    }

    /// Set or update a general preference.
    pub fn set_preference(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.preferences.insert(key.into(), value.into());
        self.updated_at = Utc::now();
    }

    /// Get a general preference value.
    pub fn get_preference(&self, key: &str) -> Option<&str> {
        self.preferences.get(key).map(String::as_str)
    }

    /// Update the communication style inference.
    pub fn set_communication_style(&mut self, style: CommunicationStyle) {
        self.communication_style = style;
        self.updated_at = Utc::now();
    }

    /// Add or update a workflow pattern.
    ///
    /// If a pattern with the same key already exists:
    /// - If it's confirmed, the existing one is kept unchanged.
    /// - If it's unconfirmed, the observation count is incremented.
    pub fn observe_pattern(&mut self, key: impl Into<String>, description: impl Into<String>, confidence: f32) {
        let key = key.into();
        let description = description.into();
        self.updated_at = Utc::now();

        let entry = self.workflow_patterns.entry(key).or_insert_with(|| {
            WorkflowPattern::new(description.clone(), confidence)
        });

        if !entry.confirmed {
            entry.observe();
        }
    }

    /// Confirm a workflow pattern by key.
    ///
    /// Returns `false` if the pattern key doesn't exist.
    pub fn confirm_pattern(&mut self, key: &str) -> bool {
        if let Some(p) = self.workflow_patterns.get_mut(key) {
            p.confirm();
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Reject a workflow pattern by key.
    ///
    /// Returns `false` if the pattern key doesn't exist.
    pub fn reject_pattern(&mut self, key: &str) -> bool {
        if let Some(p) = self.workflow_patterns.get_mut(key) {
            p.reject();
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Add a role to the user's profile if not already present.
    pub fn add_role(&mut self, role: impl Into<String>) {
        let role = role.into();
        if !self.roles.contains(&role) {
            self.roles.push(role);
            self.updated_at = Utc::now();
        }
    }

    /// Return all confirmed workflow patterns.
    pub fn confirmed_patterns(&self) -> Vec<&WorkflowPattern> {
        self.workflow_patterns
            .values()
            .filter(|p| p.confirmed)
            .collect()
    }

    /// Return all pending (unconfirmed, high-confidence) patterns suitable for
    /// nudge questions.
    ///
    /// Patterns with `confidence >= 0.6` and `observation_count >= 2` are returned.
    pub fn pending_nudge_patterns(&self) -> Vec<(&str, &WorkflowPattern)> {
        self.workflow_patterns
            .iter()
            .filter(|(_, p)| !p.confirmed && p.confidence >= 0.6 && p.observation_count >= 2)
            .map(|(k, p)| (k.as_str(), p))
            .collect()
    }

    /// Record a new session observation and increment the session counter.
    pub fn record_session(&mut self) {
        self.sessions_observed += 1;
        self.updated_at = Utc::now();
    }

    /// Serialize the profile to JSON bytes.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a profile from JSON bytes.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_roundtrip() {
        let mut p = UserProfile::new();
        p.set_preference("code_language", "rust");
        p.add_role("rust-developer");
        p.observe_pattern("tdd", "prefers TDD", 0.7);
        p.observe_pattern("tdd", "prefers TDD", 0.7);
        p.confirm_pattern("tdd");

        let json = p.to_json().unwrap();
        let p2 = UserProfile::from_json(&json).unwrap();

        assert_eq!(p2.get_preference("code_language"), Some("rust"));
        assert!(p2.roles.contains(&"rust-developer".to_string()));
        assert!(p2.workflow_patterns["tdd"].confirmed);
    }

    #[test]
    fn test_pending_nudges() {
        let mut p = UserProfile::new();
        p.observe_pattern("bullet-points", "prefers bullet points", 0.3);
        // Not enough confidence yet.
        assert!(p.pending_nudge_patterns().is_empty());

        // Observe more to cross threshold.
        p.observe_pattern("bullet-points", "prefers bullet points", 0.3);
        p.observe_pattern("bullet-points", "prefers bullet points", 0.3);
        // Now confidence should be >= 0.6 and count >= 2
        let nudges = p.pending_nudge_patterns();
        assert!(!nudges.is_empty());
    }
}
