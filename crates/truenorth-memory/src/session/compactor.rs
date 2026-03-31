//! `ContextCompactor` — LLM-driven conversation summarization.
//!
//! When a session's context budget approaches the 70% threshold, the compactor
//! takes the accumulated conversation history and produces a concise summary.
//! The summary replaces the verbose history in the active context window, freeing
//! token budget for the remainder of the task.
//!
//! # Compaction strategy
//!
//! 1. Estimate the token count of the current history (rough heuristic: 4 chars/token).
//! 2. If the history token count exceeds `budget_hint * 0.6`, compact the oldest 60%
//!    of entries down to a single summary entry.
//! 3. The compaction is performed by a caller-supplied LLM closure. If no LLM is
//!    available, the compactor falls back to extractive summarization (first sentence
//!    of each entry).

use std::collections::HashMap;

use chrono::Utc;
use tracing::{debug, info};
use uuid::Uuid;

use truenorth_core::traits::memory::{CompactionResult, MemoryError};
use truenorth_core::types::memory::MemoryEntry;

/// Approximate tokens per character (conservative estimate).
const CHARS_PER_TOKEN: usize = 4;
/// Fraction of the oldest entries to compact.
const COMPACT_FRACTION: f32 = 0.6;

/// LLM summarization function signature.
///
/// Takes the text to summarize and a requested max token length for the summary,
/// returns the summary string or an error message.
pub type SummarizeFn = Box<
    dyn Fn(String, usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Context compactor: summarizes conversation history to reclaim token budget.
///
/// The compactor is stateless — all context comes from the entries passed to
/// [`compact`]. A single `ContextCompactor` instance can be shared across sessions.
pub struct ContextCompactor {
    /// Optional LLM summarization function.
    ///
    /// If `None`, the compactor uses extractive fallback summarization.
    /// Call [`ContextCompactor::with_summarize_fn`] to inject an LLM.
    summarize_fn: Option<SummarizeFn>,
}

impl std::fmt::Debug for ContextCompactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextCompactor")
            .field("has_summarize_fn", &self.summarize_fn.is_some())
            .finish()
    }
}

impl ContextCompactor {
    /// Create a new `ContextCompactor` without an LLM summarization backend.
    ///
    /// Compaction will use extractive summarization (first sentence of each entry).
    pub fn new() -> Self {
        Self { summarize_fn: None }
    }

    /// Create a `ContextCompactor` with an LLM summarization function.
    ///
    /// The `summarize_fn` closure is called with the full text to summarize
    /// and a target token budget. It should return a concise summary string.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use truenorth_memory::session::compactor::ContextCompactor;
    ///
    /// let compactor = ContextCompactor::with_summarize_fn(|text, max_tokens| {
    ///     Box::pin(async move {
    ///         // Call your LLM here
    ///         Ok(format!("Summary of {} chars in ~{} tokens", text.len(), max_tokens))
    ///     })
    /// });
    /// ```
    pub fn with_summarize_fn<F, Fut>(summarize_fn: F) -> Self
    where
        F: Fn(String, usize) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        Self {
            summarize_fn: Some(Box::new(move |text, max_tokens| {
                Box::pin(summarize_fn(text, max_tokens))
            })),
        }
    }

    /// Compact a session's history.
    ///
    /// Selects the oldest `COMPACT_FRACTION` of entries for summarization.
    /// The remaining (most recent) entries are preserved verbatim.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session being compacted (for the `CompactionResult`).
    /// * `entries` - All current entries in the session, in insertion order.
    /// * `budget_hint` - Approximate total token budget for the session context.
    ///
    /// # Returns
    ///
    /// A `CompactionResult` with the summary text and token count estimates.
    pub async fn compact(
        &self,
        session_id: Uuid,
        entries: &[MemoryEntry],
        budget_hint: usize,
    ) -> Result<CompactionResult, MemoryError> {
        if entries.is_empty() {
            return Ok(CompactionResult {
                summary: String::new(),
                tokens_before: 0,
                tokens_after: 0,
                messages_removed: 0,
                session_id,
            });
        }

        // Estimate token count of full history.
        let total_chars: usize = entries.iter().map(|e| e.content.len()).sum();
        let tokens_before = total_chars / CHARS_PER_TOKEN;

        // Determine how many entries to compact (oldest fraction).
        let compact_count = ((entries.len() as f32 * COMPACT_FRACTION).ceil() as usize)
            .max(1)
            .min(entries.len());

        // Split into entries to compact vs. entries to keep.
        let (to_compact, _to_keep) = entries.split_at(compact_count);

        // Build the text to summarize.
        let mut text_to_summarize = String::with_capacity(
            to_compact.iter().map(|e| e.content.len() + 2).sum::<usize>(),
        );
        for entry in to_compact {
            text_to_summarize.push_str(&entry.content);
            text_to_summarize.push_str("\n\n");
        }

        // Target summary size: leave room for the retained entries.
        let retained_chars: usize = entries[compact_count..].iter().map(|e| e.content.len()).sum();
        let retained_tokens = retained_chars / CHARS_PER_TOKEN;
        let budget_for_summary = if budget_hint > retained_tokens + 200 {
            budget_hint - retained_tokens - 200
        } else {
            300
        };
        let target_summary_tokens = budget_for_summary.min(500);

        debug!(
            session_id = %session_id,
            compact_count,
            tokens_before,
            target_summary_tokens,
            "Compacting session history"
        );

        // Generate summary.
        let summary = self.summarize(text_to_summarize, target_summary_tokens).await?;

        let tokens_after = (summary.len() / CHARS_PER_TOKEN) + retained_tokens;

        info!(
            session_id = %session_id,
            tokens_before,
            tokens_after,
            messages_removed = compact_count,
            "Session compaction complete"
        );

        Ok(CompactionResult {
            summary,
            tokens_before,
            tokens_after,
            messages_removed: compact_count,
            session_id,
        })
    }

    /// Summarize the given text within a target token budget.
    ///
    /// Uses the configured LLM function if available; otherwise falls back to
    /// extractive summarization (first sentence of each paragraph).
    async fn summarize(
        &self,
        text: String,
        max_tokens: usize,
    ) -> Result<String, MemoryError> {
        if let Some(ref f) = self.summarize_fn {
            f(text, max_tokens).await.map_err(|e| MemoryError::CompactionError {
                message: format!("LLM summarization failed: {e}"),
            })
        } else {
            // Extractive fallback: take the first sentence of each paragraph.
            Ok(extractive_summary(&text, max_tokens))
        }
    }
}

impl Default for ContextCompactor {
    fn default() -> Self {
        Self::new()
    }
}

/// Extractive fallback summarization.
///
/// Takes the first sentence of each paragraph, limited by the target token
/// budget. Sentence detection uses a simple period/exclamation/question heuristic.
fn extractive_summary(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    let mut summary = String::with_capacity(max_chars.min(2048));

    for paragraph in text.split("\n\n") {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Extract first sentence.
        let sentence = first_sentence(trimmed);
        if summary.len() + sentence.len() + 2 > max_chars {
            break;
        }
        if !summary.is_empty() {
            summary.push(' ');
        }
        summary.push_str(sentence);
    }

    if summary.is_empty() {
        // Absolute fallback: truncate the text directly.
        text.chars().take(max_chars).collect()
    } else {
        summary
    }
}

/// Extract the first sentence from a block of text.
fn first_sentence(text: &str) -> &str {
    for (i, ch) in text.char_indices() {
        if ch == '.' || ch == '!' || ch == '?' {
            return &text[..=i];
        }
    }
    // No sentence terminator found — return the whole text up to a newline.
    text.split('\n').next().unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_compact_empty() {
        let compactor = ContextCompactor::new();
        let result = compactor.compact(Uuid::new_v4(), &[], 4096).await.unwrap();
        assert_eq!(result.messages_removed, 0);
        assert!(result.summary.is_empty());
    }

    #[tokio::test]
    async fn test_compact_extractive() {
        let compactor = ContextCompactor::new();
        let now = Utc::now();
        let entries: Vec<MemoryEntry> = (0..5)
            .map(|i| MemoryEntry {
                id: Uuid::new_v4(),
                scope: truenorth_core::types::memory::MemoryScope::Session,
                content: format!("Entry {}. More details about entry {}.", i, i),
                metadata: HashMap::new(),
                embedding: None,
                created_at: now,
                updated_at: now,
                importance: 0.5,
                retrieval_count: 0,
            })
            .collect();

        let result = compactor.compact(Uuid::new_v4(), &entries, 4096).await.unwrap();
        assert!(result.messages_removed > 0);
        assert!(!result.summary.is_empty());
    }

    #[test]
    fn test_extractive_summary() {
        let text = "First paragraph first sentence. More info.\n\nSecond paragraph here. Details.";
        let summary = extractive_summary(text, 100);
        assert!(summary.contains("First paragraph first sentence."));
    }
}
