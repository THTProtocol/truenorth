//! Trigger matching — map user prompts to matching skills.
//!
//! [`TriggerMatcher`] scans a user prompt against the trigger phrases of every
//! known skill and returns a ranked list of [`SkillMatch`] results.
//!
//! ## Scoring
//!
//! - Each trigger keyword found in the prompt contributes to the skill's score.
//! - Longer trigger phrases score higher (specificity weighting).
//! - Matching is case-insensitive.
//! - Glob-style wildcards (`*`) are supported in trigger phrases:
//!   `"research*"` matches `"researching"`, `"researcher"`, etc.

use truenorth_core::types::skill::SkillMetadata;

/// A skill whose trigger phrases matched (at least partially) against a prompt.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    /// The canonical name of the matched skill.
    pub skill_name: String,

    /// The skill's one-sentence description.
    pub description: String,

    /// Numeric match score (higher = better match).
    ///
    /// The score is the sum of the character lengths of all matched trigger
    /// phrases, so multi-word triggers contribute more than single words.
    pub score: f32,

    /// The specific trigger phrases that matched the prompt.
    pub matched_triggers: Vec<String>,
}

/// Matches user input against a collection of skill trigger phrases.
///
/// # Example
///
/// ```rust
/// use truenorth_skills::trigger::TriggerMatcher;
/// use truenorth_core::types::skill::{SkillMetadata, SkillLoadLevel};
///
/// let skills = vec![
///     SkillMetadata {
///         name: "Research Assistant".to_string(),
///         version: "1.0.0".to_string(),
///         description: "Deep research workflows".to_string(),
///         triggers: vec!["research".to_string(), "investigate".to_string()],
///         tags: vec![],
///         is_active: true,
///         loaded_at: SkillLoadLevel::Minimal,
///     },
/// ];
///
/// let matcher = TriggerMatcher::new();
/// let matches = matcher.match_triggers("I need to research climate change", &skills);
/// assert!(!matches.is_empty());
/// assert_eq!(matches[0].skill_name, "Research Assistant");
/// ```
#[derive(Debug, Default)]
pub struct TriggerMatcher;

impl TriggerMatcher {
    /// Creates a new `TriggerMatcher`.
    pub fn new() -> Self {
        Self
    }

    /// Matches a user prompt against all skills' trigger phrases.
    ///
    /// Returns a [`Vec<SkillMatch>`] sorted by score descending (best match first).
    /// Skills with a score of 0 are excluded from the output.
    ///
    /// Matching is:
    /// - **Case-insensitive**: `"Research"` matches trigger `"research"`.
    /// - **Substring**: a trigger `"research"` matches the prompt word `"researching"`.
    /// - **Glob**: a trigger `"research*"` uses wildcard expansion.
    /// - **Specificity-weighted**: longer triggers contribute more to the score.
    pub fn match_triggers(
        &self,
        prompt: &str,
        skills: &[SkillMetadata],
    ) -> Vec<SkillMatch> {
        let prompt_lower = prompt.to_lowercase();

        let mut matches: Vec<SkillMatch> = skills
            .iter()
            .filter_map(|skill| {
                let mut score = 0.0f32;
                let mut matched: Vec<String> = Vec::new();

                for trigger in &skill.triggers {
                    if self.trigger_matches_prompt(trigger, &prompt_lower) {
                        // Longer triggers are more specific → higher weight.
                        let weight = trigger.len() as f32;
                        score += weight;
                        matched.push(trigger.clone());
                    }
                }

                if score > 0.0 {
                    Some(SkillMatch {
                        skill_name: skill.name.clone(),
                        description: skill.description.clone(),
                        score,
                        matched_triggers: matched,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending.
        matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        matches
    }

    // ─── Private helpers ──────────────────────────────────────────────────────

    /// Returns `true` if `trigger` matches anywhere in the lower-cased `prompt`.
    ///
    /// If the trigger contains `*` it is treated as a glob pattern; otherwise a
    /// simple substring / word-boundary check is performed.
    fn trigger_matches_prompt(&self, trigger: &str, prompt_lower: &str) -> bool {
        let trigger_lower = trigger.to_lowercase();

        if trigger_lower.contains('*') {
            self.glob_matches(&trigger_lower, prompt_lower)
        } else {
            // Substring match: the trigger appears anywhere in the prompt.
            prompt_lower.contains(&trigger_lower)
        }
    }

    /// Tests whether any word in `prompt_lower` matches the glob `pattern`.
    ///
    /// The glob `*` wildcard matches any sequence of non-whitespace characters.
    /// The match is applied to every whitespace-separated token in the prompt,
    /// so `"research*"` matches the token `"researching"` but not a completely
    /// unrelated word.
    fn glob_matches(&self, pattern: &str, prompt_lower: &str) -> bool {
        // Split the glob pattern on `*` to get fixed prefix/suffix pieces.
        let parts: Vec<&str> = pattern.splitn(2, '*').collect();

        match parts.as_slice() {
            [prefix, suffix] => {
                // For every word in the prompt, check prefix + suffix.
                for word in prompt_lower.split_whitespace() {
                    if word.starts_with(prefix) && word.ends_with(suffix) {
                        // Ensure the word is long enough to accommodate both ends.
                        if word.len() >= prefix.len() + suffix.len() {
                            return true;
                        }
                    }
                }
                false
            }
            // No * in pattern — fall back to plain substring.
            _ => prompt_lower.contains(pattern),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::skill::SkillLoadLevel;

    fn make_skill(name: &str, triggers: &[&str]) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: format!("Description of {}", name),
            triggers: triggers.iter().map(|s| s.to_string()).collect(),
            tags: vec![],
            is_active: true,
            loaded_at: SkillLoadLevel::Minimal,
        }
    }

    #[test]
    fn basic_substring_match() {
        let skills = vec![make_skill("Research Assistant", &["research", "investigate"])];
        let matcher = TriggerMatcher::new();
        let matches = matcher.match_triggers("I want to research AI agents", &skills);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].skill_name, "Research Assistant");
        assert!(matches[0].matched_triggers.contains(&"research".to_string()));
    }

    #[test]
    fn case_insensitive_match() {
        let skills = vec![make_skill("Research Assistant", &["research"])];
        let matcher = TriggerMatcher::new();
        let matches = matcher.match_triggers("RESEARCH this topic", &skills);
        assert!(!matches.is_empty());
    }

    #[test]
    fn glob_wildcard_match() {
        let skills = vec![make_skill("Research Assistant", &["research*"])];
        let matcher = TriggerMatcher::new();
        let matches = matcher.match_triggers("I am researching climate change", &skills);
        assert!(!matches.is_empty());
    }

    #[test]
    fn no_match_returns_empty() {
        let skills = vec![make_skill("Research Assistant", &["research"])];
        let matcher = TriggerMatcher::new();
        let matches = matcher.match_triggers("write me a poem about trees", &skills);
        assert!(matches.is_empty());
    }

    #[test]
    fn longer_trigger_scores_higher() {
        let skills = vec![
            make_skill("Skill A", &["code"]),
            make_skill("Skill B", &["code review"]),
        ];
        let matcher = TriggerMatcher::new();
        let matches = matcher.match_triggers("please do a code review", &skills);
        assert_eq!(matches.len(), 2);
        // "code review" (11 chars) > "code" (4 chars)
        assert_eq!(matches[0].skill_name, "Skill B");
    }

    #[test]
    fn sorted_descending() {
        let skills = vec![
            make_skill("Short Trigger", &["x"]),
            make_skill("Long Trigger", &["longer trigger phrase"]),
        ];
        let matcher = TriggerMatcher::new();
        let matches = matcher.match_triggers("longer trigger phrase works", &skills);
        assert_eq!(matches[0].skill_name, "Long Trigger");
    }
}
