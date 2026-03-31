//! Semantic similarity-based infinite loop detection.
//!
//! Computes cosine similarity between consecutive step outputs.
//! When similarity exceeds the threshold (default: 0.9), the agent
//! is considered to be looping and execution is halted.
//!
//! Uses a TF-IDF inspired bag-of-words representation as a fallback
//! when embedding providers are not available.

use std::collections::HashMap;

use truenorth_core::traits::execution::ExecutionError;
use uuid::Uuid;

/// Default similarity threshold for loop detection (90%).
const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.9;

/// Computes cosine similarity between consecutive outputs to detect loops.
///
/// Maintains a sliding window of the last N outputs and flags when
/// the similarity between the current and previous output exceeds
/// the threshold.
#[derive(Debug)]
pub struct SemanticSimilarityGuard {
    task_id: Uuid,
    threshold: f64,
    history: Vec<String>,
    max_history: usize,
    consecutive_similar: usize,
    max_consecutive_similar: usize,
}

impl SemanticSimilarityGuard {
    /// Creates a new similarity guard with the given threshold.
    pub fn new(task_id: Uuid, threshold: f64) -> Self {
        Self {
            task_id,
            threshold,
            history: Vec::new(),
            max_history: 5,
            consecutive_similar: 0,
            max_consecutive_similar: 2,
        }
    }

    /// Checks the new output against previous outputs for similarity.
    ///
    /// Returns `Err(InfiniteLoopDetected)` if similarity exceeds the threshold
    /// for `max_consecutive_similar` consecutive comparisons.
    pub fn check(&mut self, output: &str) -> Result<(), ExecutionError> {
        if let Some(prev) = self.history.last() {
            let similarity = Self::cosine_similarity(prev, output);

            if similarity >= self.threshold {
                self.consecutive_similar += 1;
                if self.consecutive_similar >= self.max_consecutive_similar {
                    return Err(ExecutionError::InfiniteLoopDetected {
                        task_id: self.task_id,
                        evidence: format!(
                            "Output similarity {:.2} >= threshold {:.2} for {} consecutive steps",
                            similarity, self.threshold, self.consecutive_similar
                        ),
                    });
                }
            } else {
                self.consecutive_similar = 0;
            }
        }

        // Add to history (sliding window)
        self.history.push(output.to_string());
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }

        Ok(())
    }

    /// Computes the cosine similarity between two text strings.
    ///
    /// Uses TF (term frequency) vectors without IDF, as IDF is not
    /// computable without a document corpus. This provides a good
    /// approximation of semantic similarity for short outputs.
    pub fn cosine_similarity(a: &str, b: &str) -> f64 {
        if a.is_empty() && b.is_empty() {
            return 1.0;
        }
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let vec_a = Self::term_frequency(a);
        let vec_b = Self::term_frequency(b);

        let dot_product: f64 = vec_a.iter()
            .filter_map(|(term, &freq_a)| {
                vec_b.get(term).map(|&freq_b| freq_a * freq_b)
            })
            .sum();

        let magnitude_a: f64 = vec_a.values().map(|&f| f * f).sum::<f64>().sqrt();
        let magnitude_b: f64 = vec_b.values().map(|&f| f * f).sum::<f64>().sqrt();

        if magnitude_a == 0.0 || magnitude_b == 0.0 {
            return 0.0;
        }

        dot_product / (magnitude_a * magnitude_b)
    }

    /// Builds a term frequency vector from a text string.
    fn term_frequency(text: &str) -> HashMap<String, f64> {
        let mut freq: HashMap<String, f64> = HashMap::new();
        let words: Vec<String> = text
            .split_whitespace()
            .map(|w| w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| w.len() > 1)
            .collect();

        let total = words.len() as f64;
        if total == 0.0 {
            return freq;
        }

        for word in words {
            *freq.entry(word).or_insert(0.0) += 1.0 / total;
        }
        freq
    }

    /// Returns the number of consecutive similar outputs detected.
    pub fn consecutive_similar_count(&self) -> usize {
        self.consecutive_similar
    }

    /// Resets the guard (clears history and counters).
    pub fn reset(&mut self) {
        self.history.clear();
        self.consecutive_similar = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_similarity_is_one() {
        let s = "This is the output from the search operation about Rust programming.";
        let sim = SemanticSimilarityGuard::cosine_similarity(s, s);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn unrelated_strings_similarity_is_low() {
        let a = "Rust programming language async await futures";
        let b = "chocolate cake recipe butter sugar flour eggs";
        let sim = SemanticSimilarityGuard::cosine_similarity(a, b);
        assert!(sim < 0.2);
    }

    #[test]
    fn empty_strings_similarity() {
        let sim = SemanticSimilarityGuard::cosine_similarity("", "");
        assert_eq!(sim, 1.0);
        let sim2 = SemanticSimilarityGuard::cosine_similarity("hello", "");
        assert_eq!(sim2, 0.0);
    }

    #[test]
    fn no_loop_for_different_outputs() {
        let task_id = Uuid::new_v4();
        let mut guard = SemanticSimilarityGuard::new(task_id, 0.9);
        guard.check("Searched the web for Rust documentation").unwrap();
        guard.check("Analyzed the codebase and found three bugs").unwrap();
        guard.check("Created a test file and ran the test suite").unwrap();
    }

    #[test]
    fn loop_detected_for_identical_outputs() {
        let task_id = Uuid::new_v4();
        let mut guard = SemanticSimilarityGuard::new(task_id, 0.9);
        let output = "The function returned the same result as before without making progress on the task.";
        guard.check(output).unwrap();
        guard.check(output).unwrap(); // First similar: count = 1
        let result = guard.check(output); // Second similar: count = 2, triggers error
        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutionError::InfiniteLoopDetected { .. } => {}
            _ => panic!("Expected InfiniteLoopDetected"),
        }
    }

    #[test]
    fn reset_clears_state() {
        let task_id = Uuid::new_v4();
        let mut guard = SemanticSimilarityGuard::new(task_id, 0.9);
        let output = "Same output repeated again and again for loop detection test purposes.";
        guard.check(output).unwrap();
        guard.check(output).unwrap();
        guard.reset();
        assert_eq!(guard.consecutive_similar_count(), 0);
        // After reset, should be able to add more outputs without triggering
        guard.check(output).unwrap();
        guard.check(output).unwrap();
    }
}
