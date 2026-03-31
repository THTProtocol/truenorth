//! `HybridSearch` — Reciprocal Rank Fusion (RRF) of fulltext + semantic results.
//!
//! Reciprocal Rank Fusion is a well-established rank-aggregation algorithm that
//! combines multiple ranked lists into a single unified ranking without requiring
//! score normalization between the lists. It was introduced by Cormack et al.
//! (SIGIR 2009) and is widely used in hybrid retrieval systems.
//!
//! ## RRF formula
//!
//! ```text
//! RRF_score(d) = Σ_r [ weight_r / (k + rank_r(d)) ]
//! ```
//!
//! Where:
//! - `d` is a document.
//! - `r` ranges over the result lists (fulltext, semantic).
//! - `rank_r(d)` is the 1-based rank of `d` in list `r` (∞ if absent).
//! - `k` is a smoothing constant (default: 60, from the original paper).
//! - `weight_r` is the per-list weight (default: 0.5 for both).
//!
//! Documents absent from one list still receive a contribution from the other.

use std::collections::HashMap;
use uuid::Uuid;

use truenorth_core::types::memory::{MemoryEntry, MemorySearchResult, MemorySearchType};

/// Smoothing constant from the original RRF paper (Cormack 2009).
const RRF_K: f32 = 60.0;

/// Reciprocal Rank Fusion combiner for hybrid search.
///
/// Merges fulltext and semantic result lists into a single unified ranking.
/// The `ft_weight` and `sem_weight` parameters control the relative influence
/// of each source (they should sum to 1.0 but need not — they are applied
/// multiplicatively to the RRF contribution from each source).
#[derive(Debug, Clone)]
pub struct HybridSearch {
    /// Weight applied to contributions from the fulltext result list.
    ft_weight: f32,
    /// Weight applied to contributions from the semantic result list.
    sem_weight: f32,
}

impl HybridSearch {
    /// Create a new `HybridSearch` with the given per-source weights.
    ///
    /// # Arguments
    ///
    /// * `ft_weight` - Weight for the fulltext (BM25) source. Typical: 0.5.
    /// * `sem_weight` - Weight for the semantic (vector) source. Typical: 0.5.
    pub fn new(ft_weight: f32, sem_weight: f32) -> Self {
        Self { ft_weight, sem_weight }
    }

    /// Merge two ranked result lists into a single hybrid ranking using RRF.
    ///
    /// # Arguments
    ///
    /// * `fulltext_results` - BM25-ranked results from Tantivy.
    /// * `semantic_results` - Cosine-similarity-ranked results.
    /// * `limit` - Maximum number of results to return.
    ///
    /// # Returns
    ///
    /// A merged list of `MemorySearchResult` with `search_type = Hybrid`,
    /// ordered by RRF score descending. Entries are deduplicated by UUID.
    pub fn merge(
        &self,
        fulltext_results: Vec<MemorySearchResult>,
        semantic_results: Vec<MemorySearchResult>,
        limit: usize,
    ) -> Vec<MemorySearchResult> {
        // Accumulate RRF scores per entry ID.
        let mut rrf_scores: HashMap<Uuid, f32> = HashMap::new();
        // Collect the entry data (prefer semantic entry since it has embeddings).
        let mut entries: HashMap<Uuid, MemoryEntry> = HashMap::new();

        // Contributions from the fulltext list.
        for (rank, result) in fulltext_results.iter().enumerate() {
            let id = result.entry.id;
            let contribution = self.ft_weight / (RRF_K + (rank as f32 + 1.0));
            *rrf_scores.entry(id).or_insert(0.0) += contribution;
            entries.entry(id).or_insert_with(|| result.entry.clone());
        }

        // Contributions from the semantic list.
        for (rank, result) in semantic_results.iter().enumerate() {
            let id = result.entry.id;
            let contribution = self.sem_weight / (RRF_K + (rank as f32 + 1.0));
            *rrf_scores.entry(id).or_insert(0.0) += contribution;
            // Prefer the semantic entry (it has embeddings; fulltext entry may not).
            entries.insert(id, result.entry.clone());
        }

        // Build sorted result list.
        let mut merged: Vec<(Uuid, f32)> = rrf_scores.into_iter().collect();
        merged.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        merged.truncate(limit);

        // Normalize RRF scores to [0, 1].
        let max_score = merged.first().map(|(_, s)| *s).unwrap_or(1.0);
        let max_score = if max_score > 0.0 { max_score } else { 1.0 };

        merged
            .into_iter()
            .filter_map(|(id, raw_score)| {
                entries.get(&id).map(|entry| MemorySearchResult {
                    entry: entry.clone(),
                    score: raw_score / max_score,
                    search_type: MemorySearchType::Hybrid,
                })
            })
            .collect()
    }

    /// Merge with equal weights (0.5 / 0.5).
    ///
    /// Convenience method for the default case.
    pub fn merge_equal(
        fulltext_results: Vec<MemorySearchResult>,
        semantic_results: Vec<MemorySearchResult>,
        limit: usize,
    ) -> Vec<MemorySearchResult> {
        let hybrid = Self::new(0.5, 0.5);
        hybrid.merge(fulltext_results, semantic_results, limit)
    }

    /// Return the configured fulltext weight.
    pub fn ft_weight(&self) -> f32 {
        self.ft_weight
    }

    /// Return the configured semantic weight.
    pub fn sem_weight(&self) -> f32 {
        self.sem_weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use truenorth_core::types::memory::MemoryScope;

    fn make_result(id: Uuid, score: f32, search_type: MemorySearchType) -> MemorySearchResult {
        let now = chrono::Utc::now();
        MemorySearchResult {
            entry: MemoryEntry {
                id,
                scope: MemoryScope::Project,
                content: format!("content for {}", id),
                metadata: HashMap::new(),
                embedding: None,
                created_at: now,
                updated_at: now,
                importance: 0.5,
                retrieval_count: 0,
            },
            score,
            search_type,
        }
    }

    #[test]
    fn test_merge_deduplicates() {
        let hybrid = HybridSearch::new(0.5, 0.5);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let ft = vec![
            make_result(id1, 0.9, MemorySearchType::FullText),
            make_result(id2, 0.6, MemorySearchType::FullText),
        ];
        let sem = vec![
            make_result(id1, 0.85, MemorySearchType::Semantic),
        ];

        let merged = hybrid.merge(ft, sem, 10);
        let ids: Vec<Uuid> = merged.iter().map(|r| r.entry.id).collect();
        // id1 appears once despite being in both lists.
        assert_eq!(ids.iter().filter(|&&id| id == id1).count(), 1);
        assert!(merged.len() <= 2);
    }

    #[test]
    fn test_top_ranked_in_both_wins() {
        let hybrid = HybridSearch::new(0.5, 0.5);
        let id_both = Uuid::new_v4();
        let id_only_ft = Uuid::new_v4();

        let ft = vec![
            make_result(id_both, 0.9, MemorySearchType::FullText),
            make_result(id_only_ft, 0.8, MemorySearchType::FullText),
        ];
        let sem = vec![
            make_result(id_both, 0.9, MemorySearchType::Semantic),
        ];

        let merged = hybrid.merge(ft, sem, 5);
        // id_both gets contributions from both lists, should rank #1.
        assert_eq!(merged[0].entry.id, id_both);
    }

    #[test]
    fn test_normalized_scores() {
        let hybrid = HybridSearch::new(0.5, 0.5);
        let id = Uuid::new_v4();
        let ft = vec![make_result(id, 1.0, MemorySearchType::FullText)];
        let merged = hybrid.merge(ft, vec![], 5);
        assert!(!merged.is_empty());
        // Top result should have a score of 1.0 after normalization.
        assert!((merged[0].score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_empty_inputs() {
        let hybrid = HybridSearch::new(0.5, 0.5);
        let merged = hybrid.merge(vec![], vec![], 10);
        assert!(merged.is_empty());
    }
}
