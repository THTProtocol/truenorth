//! `Deduplicator` — semantic deduplication for memory entries.
//!
//! Before writing a new entry to the project or identity store, the deduplicator
//! computes cosine similarity between the new entry's embedding and all existing
//! entry embeddings. If the maximum similarity exceeds a configurable threshold
//! (default: 0.85), the new entry is considered a duplicate of the most similar
//! existing entry.
//!
//! # Algorithm
//!
//! 1. For each existing entry with a non-null embedding, compute cosine similarity
//!    with the new entry's embedding.
//! 2. Find the maximum similarity and the ID of the best match.
//! 3. If max_similarity ≥ threshold → return `Some(best_match_id)`.
//! 4. Otherwise → return `None` (no duplicate found).

use uuid::Uuid;
use truenorth_core::types::memory::MemoryEntry;

/// Default similarity threshold above which entries are considered duplicates.
pub const DEFAULT_DEDUP_THRESHOLD: f32 = 0.85;

/// Semantic deduplication checker.
///
/// Compares embedding vectors using cosine similarity to detect near-duplicate
/// memory entries before writing. This prevents the project and identity stores
/// from accumulating redundant entries over time.
#[derive(Debug, Clone)]
pub struct Deduplicator {
    /// Cosine similarity threshold (0.0–1.0). Entries at or above this value
    /// are considered duplicates of an existing entry.
    threshold: f32,
}

impl Deduplicator {
    /// Create a `Deduplicator` with the given similarity threshold.
    ///
    /// Typical values:
    /// - `0.85` — strict deduplication, catches paraphrases
    /// - `0.90` — moderate deduplication, catches near-identical content
    /// - `0.95` — loose deduplication, only catches near-exact duplicates
    pub fn new(threshold: f32) -> Self {
        Self { threshold: threshold.clamp(0.0, 1.0) }
    }

    /// Create a `Deduplicator` with the default threshold (0.85).
    pub fn default() -> Self {
        Self::new(DEFAULT_DEDUP_THRESHOLD)
    }

    /// Check whether `candidate` is semantically similar to any entry in `existing`.
    ///
    /// Returns the ID of the most similar existing entry if similarity ≥ threshold,
    /// or `None` if no duplicate is found.
    ///
    /// # Arguments
    ///
    /// * `candidate` - The new entry to check. Must have `embedding` set.
    /// * `existing` - Slice of existing entries to compare against. Entries without
    ///   embeddings are skipped.
    ///
    /// # Returns
    ///
    /// `Some(Uuid)` — the ID of the duplicate entry, or `None` if no duplicate found.
    pub fn find_duplicate(
        &self,
        candidate: &MemoryEntry,
        existing: &[MemoryEntry],
    ) -> Option<Uuid> {
        let candidate_embedding = candidate.embedding.as_deref()?;

        let mut best_score = 0.0_f32;
        let mut best_id: Option<Uuid> = None;

        for entry in existing {
            // Skip entries without embeddings and skip self-comparison.
            let emb = match entry.embedding.as_deref() {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            if entry.id == candidate.id {
                continue;
            }

            let score = cosine_similarity(candidate_embedding, emb);
            if score > best_score {
                best_score = score;
                best_id = Some(entry.id);
            }
        }

        if best_score >= self.threshold {
            tracing::debug!(
                "Deduplicator: candidate similar to {} (score={:.3})",
                best_id.unwrap(),
                best_score
            );
            best_id
        } else {
            None
        }
    }

    /// Identify all pairs within `entries` that exceed the threshold.
    ///
    /// Returns a list of `(id_a, id_b, similarity)` tuples where both entries
    /// are considered duplicates. The pair is ordered so that `id_a < id_b`
    /// (lexicographic UUID string order) to avoid duplicates in the result.
    ///
    /// Used by the consolidation pruner to find and merge redundant entries.
    pub fn find_all_duplicates(
        &self,
        entries: &[MemoryEntry],
    ) -> Vec<(Uuid, Uuid, f32)> {
        let mut pairs = Vec::new();

        for (i, a) in entries.iter().enumerate() {
            let emb_a = match a.embedding.as_deref() {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            for b in &entries[i + 1..] {
                let emb_b = match b.embedding.as_deref() {
                    Some(v) if !v.is_empty() => v,
                    _ => continue,
                };
                let score = cosine_similarity(emb_a, emb_b);
                if score >= self.threshold {
                    // Canonical ordering to avoid duplicate pairs.
                    let (id_a, id_b) = if a.id.to_string() < b.id.to_string() {
                        (a.id, b.id)
                    } else {
                        (b.id, a.id)
                    };
                    pairs.push((id_a, id_b, score));
                }
            }
        }

        // Sort by similarity descending.
        pairs.sort_by(|x, y| y.2.partial_cmp(&x.2).unwrap_or(std::cmp::Ordering::Equal));
        pairs
    }

    /// Return the configured similarity threshold.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }
}

/// Compute cosine similarity between two f32 slices.
///
/// Returns 0.0 if either vector is zero-length or has zero norm.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use truenorth_core::types::memory::MemoryScope;

    fn make_entry_with_embedding(embedding: Vec<f32>) -> MemoryEntry {
        let now = Utc::now();
        MemoryEntry {
            id: Uuid::new_v4(),
            scope: MemoryScope::Project,
            content: "test content".to_string(),
            metadata: HashMap::new(),
            embedding: Some(embedding),
            created_at: now,
            updated_at: now,
            importance: 0.5,
            retrieval_count: 0,
        }
    }

    #[test]
    fn test_identical_embedding_is_duplicate() {
        let dedup = Deduplicator::new(0.85);
        let existing = make_entry_with_embedding(vec![1.0, 0.0, 0.0]);
        let candidate = make_entry_with_embedding(vec![1.0, 0.0, 0.0]);
        let result = dedup.find_duplicate(&candidate, &[existing.clone()]);
        assert_eq!(result, Some(existing.id));
    }

    #[test]
    fn test_orthogonal_vectors_not_duplicate() {
        let dedup = Deduplicator::new(0.85);
        let existing = make_entry_with_embedding(vec![1.0, 0.0, 0.0]);
        let candidate = make_entry_with_embedding(vec![0.0, 1.0, 0.0]);
        let result = dedup.find_duplicate(&candidate, &[existing]);
        assert!(result.is_none());
    }

    #[test]
    fn test_cosine_similarity_values() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-5);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]) - 0.0).abs() < 1e-5);
        let score = cosine_similarity(&[1.0, 1.0], &[1.0, 0.0]);
        assert!((score - 0.7071).abs() < 1e-3);
    }

    #[test]
    fn test_find_all_duplicates() {
        let dedup = Deduplicator::new(0.85);
        let a = make_entry_with_embedding(vec![1.0, 0.0]);
        let b = make_entry_with_embedding(vec![0.99, 0.14]); // ~cos 0.99
        let c = make_entry_with_embedding(vec![0.0, 1.0]); // orthogonal
        let pairs = dedup.find_all_duplicates(&[a, b, c]);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].2 > 0.85);
    }
}
