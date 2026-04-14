//! Context compaction policy — when and how to compact conversation history.
//!
//! Defines the strategies for reducing context size when approaching limits:
//! - **SlidingWindow**: Keep only the N most recent messages
//! - **ExtractiveSummary**: LLM summarizes oldest K messages into a single block
//! - **PriorityBased**: Keep messages by importance score, drop lowest-priority first

use serde::{Deserialize, Serialize};

/// The compaction strategy to apply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CompactionStrategy {
    /// Keep only the most recent N messages. Oldest messages are discarded.
    ///
    /// Fast and simple. Appropriate when older context is no longer needed.
    SlidingWindow { 
        /// Number of recent messages to keep.
        keep_last_n: usize },

    /// Summarize the oldest K messages using an LLM and replace them with
    /// a single summary block tagged `[Compacted: ...]`.
    ///
    /// Preserves information but requires an LLM call.
    ExtractiveSummary {
        /// Number of oldest messages to summarize.
        summarize_oldest_k: usize },

    /// Remove messages with the lowest importance scores first.
    ///
    /// Requires messages to have associated importance scores.
    /// Falls back to `SlidingWindow` if no scores are available.
    PriorityBased {
        /// Target context utilization ratio (0.0–1.0).
        target_utilization: f32 },
}

impl Default for CompactionStrategy {
    fn default() -> Self {
        Self::ExtractiveSummary { summarize_oldest_k: 10 }
    }
}

/// Configuration for the compaction policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPolicy {
    /// The strategy to use for compaction.
    pub strategy: CompactionStrategy,
    /// Messages to never compact (e.g., the last N messages, tool results).
    pub protect_last_n: usize,
    /// Never compact messages tagged as critical context.
    pub protect_critical: bool,
    /// The utilization target after compaction (fraction, 0.0–1.0).
    pub target_utilization: f32,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            strategy: CompactionStrategy::default(),
            protect_last_n: 5,
            protect_critical: true,
            target_utilization: 0.50, // After compaction, aim for 50% utilization
        }
    }
}

impl CompactionPolicy {
    /// Applies the sliding window policy to a message list.
    ///
    /// Returns the indices of messages to REMOVE.
    pub fn sliding_window_removals(
        messages: &[serde_json::Value],
        keep_last_n: usize,
        protect_last_n: usize,
    ) -> Vec<usize> {
        let total = messages.len();
        let effective_keep = keep_last_n.max(protect_last_n);
        if total <= effective_keep {
            return vec![];
        }
        (0..total.saturating_sub(effective_keep)).collect()
    }

    /// Applies extractive summary policy: identifies messages to summarize.
    ///
    /// Returns the range `[start, end)` of messages that should be summarized.
    pub fn extractive_summary_range(
        messages: &[serde_json::Value],
        summarize_oldest_k: usize,
        protect_last_n: usize,
    ) -> Option<(usize, usize)> {
        let total = messages.len();
        let available_to_summarize = total.saturating_sub(protect_last_n);
        if available_to_summarize == 0 {
            return None;
        }
        let k = summarize_oldest_k.min(available_to_summarize);
        if k == 0 {
            return None;
        }
        Some((0, k))
    }

    /// Estimates how many tokens would be freed by removing the given message indices.
    ///
    /// Uses the rough 4 chars = 1 token estimate.
    pub fn estimate_freed_tokens(
        messages: &[serde_json::Value],
        remove_indices: &[usize],
    ) -> usize {
        remove_indices.iter()
            .filter_map(|&i| messages.get(i))
            .map(|m| m.to_string().len() / 4)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_messages(n: usize) -> Vec<serde_json::Value> {
        (0..n).map(|i| serde_json::json!({"role": "user", "content": format!("message {}", i)})).collect()
    }

    #[test]
    fn sliding_window_removes_oldest() {
        let msgs = make_messages(20);
        let removals = CompactionPolicy::sliding_window_removals(&msgs, 10, 5);
        assert_eq!(removals.len(), 10);
        assert_eq!(removals[0], 0);
        assert_eq!(removals[9], 9);
    }

    #[test]
    fn sliding_window_keeps_all_if_few_messages() {
        let msgs = make_messages(5);
        let removals = CompactionPolicy::sliding_window_removals(&msgs, 10, 5);
        assert!(removals.is_empty());
    }

    #[test]
    fn extractive_summary_range_correct() {
        let msgs = make_messages(20);
        let range = CompactionPolicy::extractive_summary_range(&msgs, 5, 5);
        assert_eq!(range, Some((0, 5)));
    }

    #[test]
    fn extractive_summary_range_none_when_protected() {
        let msgs = make_messages(4);
        let range = CompactionPolicy::extractive_summary_range(&msgs, 5, 5);
        assert_eq!(range, None);
    }
}
