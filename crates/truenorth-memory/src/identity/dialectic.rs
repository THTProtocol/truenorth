//! `HonchoDialecticModeler` — observes interactions and infers user patterns.
//!
//! The dialectic modeler is inspired by Honcho's user model. It observes
//! conversation turns and tool results to build a probabilistic model of user
//! preferences, communication style, and workflow patterns.
//!
//! ## How it works
//!
//! 1. **Observe**: The modeler receives each conversation turn or tool result.
//! 2. **Infer**: Pattern detectors look for signals (e.g., "user asked for code
//!    in Rust three times" → infer "prefers Rust").
//! 3. **Nudge**: After accumulating enough evidence, the modeler generates a
//!    nudge question to confirm the inference with the user.
//! 4. **Store**: Confirmed patterns are written to the identity memory store.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use truenorth_core::types::memory::MemoryEntry;
use crate::identity::profile::UserProfile;
use crate::identity::sqlite_store::IdentityMemoryStore;

/// A suggested nudge question for user confirmation.
#[derive(Debug, Clone)]
pub struct NudgeQuestion {
    /// Short key identifying the pattern being confirmed.
    pub pattern_key: String,
    /// The human-readable question to show the user.
    pub question: String,
    /// Confidence in the inferred pattern (0.0–1.0).
    pub confidence: f32,
}

/// Internal observation extracted from a conversation turn.
#[derive(Debug, Clone)]
struct Observation {
    /// The signal type detected.
    signal: SignalType,
    /// How strongly this observation supports the signal.
    strength: f32,
}

/// Taxonomy of detectable user signals.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SignalType {
    /// User tends to use code blocks / ask for code directly.
    CodeFirstStyle,
    /// User's messages frequently use bullet lists.
    BulletPointStyle,
    /// User's messages are flowing prose.
    ProseStyle,
    /// User mentions or works with a specific domain (key = domain name).
    DomainKeyword(String),
    /// User appears to follow test-driven development.
    TddWorkflow,
    /// User prefers concise responses.
    ConcisenessPreference,
    /// User uses Rust programming language.
    RustDeveloper,
    /// User uses blockchain / web3 tooling.
    BlockchainDeveloper,
}

/// Dialectic modeler that observes interactions and updates `UserProfile`.
///
/// Holds a mutable `UserProfile` internally and syncs confirmed patterns to
/// the `IdentityMemoryStore` after each update.
#[derive(Debug)]
pub struct HonchoDialecticModeler {
    /// Current user profile (protected for concurrent reads during a session).
    profile: Arc<RwLock<UserProfile>>,
    /// Backing identity store for persistence.
    store: Arc<IdentityMemoryStore>,
    /// Accumulator for unprocessed observations.
    observations: Arc<RwLock<Vec<Observation>>>,
    /// Minimum observations required before generating a nudge.
    nudge_threshold: u32,
}

impl HonchoDialecticModeler {
    /// Create a new `HonchoDialecticModeler`.
    ///
    /// Loads the existing profile from the `store` (if one exists), or creates
    /// a new empty profile.
    pub async fn new(store: Arc<IdentityMemoryStore>, nudge_threshold: u32) -> Self {
        let profile = store.load_profile().await.unwrap_or_else(|_| UserProfile::new());
        Self {
            profile: Arc::new(RwLock::new(profile)),
            store,
            observations: Arc::new(RwLock::new(Vec::new())),
            nudge_threshold,
        }
    }

    /// Observe a user message and extract signals.
    ///
    /// Call this for every user-authored message in the conversation. The modeler
    /// analyses the text for communication style signals and domain keywords.
    pub async fn observe_user_message(&self, message: &str) {
        let observations = self.extract_signals(message);
        if observations.is_empty() {
            return;
        }

        let mut obs_lock = self.observations.write().await;
        obs_lock.extend(observations.clone());

        // Update profile with the new signals.
        let mut profile = self.profile.write().await;
        for obs in &observations {
            self.apply_observation_to_profile(&mut profile, obs);
        }
        debug!("HonchoDialecticModeler: processed {} signals", observations.len());
    }

    /// Observe a tool result entry from the session.
    ///
    /// Tool results can reveal domain signals (e.g., a `cargo build` call suggests Rust).
    pub async fn observe_tool_result(&self, entry: &MemoryEntry) {
        let signals = self.extract_tool_signals(entry);
        let mut profile = self.profile.write().await;
        for obs in &signals {
            self.apply_observation_to_profile(&mut profile, obs);
        }
    }

    /// Generate nudge questions for pending high-confidence patterns.
    ///
    /// Returns a list of questions that can be injected into the conversation
    /// to confirm or reject inferred patterns. Should be called periodically
    /// (e.g., at the end of a task).
    pub async fn generate_nudge_questions(&self) -> Vec<NudgeQuestion> {
        let profile = self.profile.read().await;
        let pending = profile.pending_nudge_patterns();

        pending
            .iter()
            .map(|(key, pattern)| NudgeQuestion {
                pattern_key: key.to_string(),
                question: format!(
                    "I've noticed {}. Is that accurate?",
                    pattern.description
                ),
                confidence: pattern.confidence,
            })
            .collect()
    }

    /// Confirm a pattern by key (called after the user responds positively to a nudge).
    ///
    /// Persists the confirmed pattern to the identity store as a memory entry.
    pub async fn confirm_pattern(&self, key: &str) -> Result<(), truenorth_core::traits::memory::MemoryError> {
        {
            let mut profile = self.profile.write().await;
            profile.confirm_pattern(key);
        }
        self.persist_profile().await?;
        info!("Confirmed identity pattern: {}", key);
        Ok(())
    }

    /// Reject a pattern by key (called after the user responds negatively to a nudge).
    pub async fn reject_pattern(&self, key: &str) -> Result<(), truenorth_core::traits::memory::MemoryError> {
        {
            let mut profile = self.profile.write().await;
            profile.reject_pattern(key);
        }
        self.persist_profile().await?;
        info!("Rejected identity pattern: {}", key);
        Ok(())
    }

    /// Get a read-only snapshot of the current user profile.
    pub async fn profile_snapshot(&self) -> UserProfile {
        self.profile.read().await.clone()
    }

    /// Flush any pending profile updates to the identity store.
    pub async fn flush(&self) -> Result<(), truenorth_core::traits::memory::MemoryError> {
        self.persist_profile().await
    }

    /// Persist the current profile to the identity store.
    async fn persist_profile(&self) -> Result<(), truenorth_core::traits::memory::MemoryError> {
        let profile = self.profile.read().await.clone();
        self.store.save_profile(&profile).await
    }

    /// Extract signals from a user message text.
    fn extract_signals(&self, text: &str) -> Vec<Observation> {
        let mut observations = Vec::new();
        let lower = text.to_lowercase();

        // Communication style signals.
        let bullet_indicators = ["- ", "* ", "• ", "\n1. ", "\n2. "];
        if bullet_indicators.iter().any(|s| lower.contains(s)) {
            observations.push(Observation { signal: SignalType::BulletPointStyle, strength: 0.3 });
        }

        if lower.contains("```") {
            observations.push(Observation { signal: SignalType::CodeFirstStyle, strength: 0.4 });
        }

        // Domain keywords.
        let rust_keywords = ["cargo", "rustc", "impl ", "fn main", ".rs", "tokio", "clippy"];
        if rust_keywords.iter().any(|k| lower.contains(k)) {
            observations.push(Observation { signal: SignalType::RustDeveloper, strength: 0.5 });
        }

        let blockchain_keywords = ["solidity", "ethereum", "web3", "wallet", "nft", "defi",
                                   "smart contract", "blockchain", "solana", "anchor"];
        if blockchain_keywords.iter().any(|k| lower.contains(k)) {
            observations.push(Observation { signal: SignalType::BlockchainDeveloper, strength: 0.5 });
        }

        let tdd_keywords = ["#[test]", "#[cfg(test)]", "assert_eq!", "unit test", "integration test", "test-driven"];
        if tdd_keywords.iter().any(|k| lower.contains(k)) {
            observations.push(Observation { signal: SignalType::TddWorkflow, strength: 0.4 });
        }

        if lower.contains("be brief") || lower.contains("concise") || lower.contains("tldr") {
            observations.push(Observation { signal: SignalType::ConcisenessPreference, strength: 0.6 });
        }

        observations
    }

    /// Extract signals from a tool result memory entry.
    fn extract_tool_signals(&self, entry: &MemoryEntry) -> Vec<Observation> {
        let mut obs = Vec::new();
        let content_lower = entry.content.to_lowercase();

        // Tool name signals.
        if let Some(tool) = entry.metadata.get("tool_name").and_then(|v| v.as_str()) {
            match tool {
                "cargo_run" | "cargo_build" | "cargo_test" | "clippy" => {
                    obs.push(Observation { signal: SignalType::RustDeveloper, strength: 0.6 });
                }
                "forge" | "hardhat" | "truffle" | "anchor_build" => {
                    obs.push(Observation { signal: SignalType::BlockchainDeveloper, strength: 0.6 });
                }
                _ => {}
            }
        }

        // Content signals.
        if content_lower.contains("test passed") || content_lower.contains("test ok") {
            obs.push(Observation { signal: SignalType::TddWorkflow, strength: 0.3 });
        }

        obs
    }

    /// Apply an observation to the mutable profile.
    fn apply_observation_to_profile(&self, profile: &mut UserProfile, obs: &Observation) {
        match &obs.signal {
            SignalType::BulletPointStyle => {
                profile.observe_pattern(
                    "bullet-point-style",
                    "you prefer responses formatted as bullet points",
                    obs.strength,
                );
            }
            SignalType::CodeFirstStyle => {
                profile.observe_pattern(
                    "code-first-style",
                    "you prefer concise code blocks with minimal surrounding prose",
                    obs.strength,
                );
            }
            SignalType::ProseStyle => {
                profile.observe_pattern(
                    "prose-style",
                    "you prefer flowing prose over bullet points",
                    obs.strength,
                );
            }
            SignalType::RustDeveloper => {
                profile.observe_pattern(
                    "rust-developer",
                    "you work primarily in Rust",
                    obs.strength,
                );
                profile.add_role("rust-developer");
            }
            SignalType::BlockchainDeveloper => {
                profile.observe_pattern(
                    "blockchain-developer",
                    "you work in blockchain / Web3",
                    obs.strength,
                );
                profile.add_role("blockchain-developer");
            }
            SignalType::TddWorkflow => {
                profile.observe_pattern(
                    "tdd-workflow",
                    "you follow test-driven development practices",
                    obs.strength,
                );
            }
            SignalType::ConcisenessPreference => {
                profile.observe_pattern(
                    "concise-responses",
                    "you prefer concise, direct responses",
                    obs.strength,
                );
                profile.set_preference("verbosity", "concise");
            }
            SignalType::DomainKeyword(domain) => {
                let key = format!("domain-{}", domain.to_lowercase().replace(' ', "-"));
                let desc = format!("you work in the {} domain", domain);
                profile.observe_pattern(key, desc, obs.strength);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn make_modeler(tmp: &TempDir) -> HonchoDialecticModeler {
        let store = Arc::new(
            IdentityMemoryStore::new(
                tmp.path().join("identity.db"),
                None,
                0.85,
            )
            .unwrap(),
        );
        HonchoDialecticModeler::new(store, 2).await
    }

    #[tokio::test]
    async fn test_rust_signal_detection() {
        let tmp = TempDir::new().unwrap();
        let modeler = make_modeler(&tmp).await;
        modeler.observe_user_message("I use cargo build and tokio for async Rust.").await;
        modeler.observe_user_message("Let me run clippy on my .rs files.").await;

        let profile = modeler.profile_snapshot().await;
        assert!(profile.workflow_patterns.contains_key("rust-developer"));
        assert!(profile.roles.contains(&"rust-developer".to_string()));
    }

    #[tokio::test]
    async fn test_nudge_generation() {
        let tmp = TempDir::new().unwrap();
        let modeler = make_modeler(&tmp).await;

        // Observe enough times to cross nudge threshold.
        for _ in 0..3 {
            modeler.observe_user_message("cargo build && cargo test #[test] assert_eq!").await;
        }
        let nudges = modeler.generate_nudge_questions().await;
        // At least one nudge should be generated.
        assert!(!nudges.is_empty());
    }
}
