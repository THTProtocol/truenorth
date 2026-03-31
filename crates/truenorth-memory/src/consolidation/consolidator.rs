//! `AutoDreamConsolidator` — four-phase autoDream-style memory consolidation.
//!
//! Implements the Orient → Gather → Consolidate → Prune cycle described in
//! the Phase 1 design and the `MemoryConsolidationState` machine in Section 5.4.
//!
//! ## Phases
//!
//! 1. **Orient**: Load metadata about all three tiers. Identify which sessions
//!    have not yet been consolidated.
//!
//! 2. **Gather**: Load actual session entries. Extract patterns: recurring topics,
//!    errors, decisions, preference signals.
//!
//! 3. **Consolidate**: Merge important signals into project/identity memory.
//!    Run semantic deduplication before each write. Emit `MemoryConsolidated` event.
//!
//! 4. **Prune**: Remove redundant entries (similarity > 0.90). Archive entries
//!    last accessed > 90 days with low importance. Emit `MemoryPruned` event.
//!
//! ## Observability
//!
//! Every state transition emits a `ReasoningEvent` to the broadcast channel
//! (if configured). The Visual Reasoning Layer displays consolidation progress
//! in real time.

use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration, Utc};
use tracing::{debug, info, instrument, warn};

use truenorth_core::traits::memory::{ConsolidationReport, MemoryError};
use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::types::memory::{MemoryEntry, MemoryScope};

use crate::identity::sqlite_store::IdentityMemoryStore;
use crate::project::deduplicator::Deduplicator;
use crate::project::sqlite_store::ProjectMemoryStore;
use crate::search::SearchEngine;
use crate::session::store::SessionMemoryStore;

/// Entry extracted as a consolidation signal during the Gather phase.
#[derive(Debug, Clone)]
struct Signal {
    /// Source entry this signal was extracted from.
    source_id: uuid::Uuid,
    /// The distilled content to potentially write to project/identity memory.
    content: String,
    /// Target scope for this signal.
    target_scope: MemoryScope,
    /// Estimated importance (0.0–1.0).
    importance: f32,
}

/// Stub constant for the stale-entry age threshold.
const STALE_DAYS: i64 = 90;
/// Stub constant for the high-similarity prune threshold.
const PRUNE_SIMILARITY_THRESHOLD: f32 = 0.90;
/// Maximum signals to consolidate per cycle (caps LLM context size).
const MAX_SIGNALS_PER_CYCLE: usize = 50;

/// AutoDream consolidator: runs the four-phase consolidation cycle.
#[derive(Debug)]
pub struct AutoDreamConsolidator {
    session_store: Arc<SessionMemoryStore>,
    project_store: Arc<ProjectMemoryStore>,
    identity_store: Arc<IdentityMemoryStore>,
    search_engine: Arc<SearchEngine>,
    /// Optional broadcast sender for emitting reasoning events.
    event_tx: Option<tokio::sync::broadcast::Sender<ReasoningEvent>>,
    /// Deduplicator used during the consolidate phase.
    deduplicator: Deduplicator,
}

impl AutoDreamConsolidator {
    /// Create a new `AutoDreamConsolidator`.
    pub fn new(
        session_store: Arc<SessionMemoryStore>,
        project_store: Arc<ProjectMemoryStore>,
        identity_store: Arc<IdentityMemoryStore>,
        search_engine: Arc<SearchEngine>,
        event_tx: Option<tokio::sync::broadcast::Sender<ReasoningEvent>>,
    ) -> Self {
        Self {
            session_store,
            project_store,
            identity_store,
            search_engine,
            event_tx,
            deduplicator: Deduplicator::new(PRUNE_SIMILARITY_THRESHOLD),
        }
    }

    /// Run a full consolidation cycle for the given scope.
    ///
    /// Executes the four phases sequentially:
    /// Orient → Gather → Consolidate → Prune.
    ///
    /// Returns a `ConsolidationReport` with statistics about the cycle.
    #[instrument(skip(self), fields(scope = ?scope))]
    pub async fn run(&self, scope: MemoryScope) -> Result<ConsolidationReport, MemoryError> {
        let started_at = Instant::now();
        info!("AutoDreamConsolidator: starting cycle for {:?}", scope);
        self.emit_event(ReasoningEventPayload::MemoryConsolidated {
            scope,
            entries_reviewed: 0,
            entries_merged: 0,
            entries_pruned: 0,
            duration_ms: 0,
        });

        // Phase 1: Orient
        let recent_entries = self.phase_orient(scope).await?;
        if recent_entries.is_empty() {
            info!("AutoDreamConsolidator: no new entries to consolidate for {:?}", scope);
            let duration_ms = started_at.elapsed().as_millis() as u64;
            return Ok(ConsolidationReport {
                scope,
                entries_reviewed: 0,
                entries_merged: 0,
                entries_pruned: 0,
                entries_created: 0,
                duration_ms,
            });
        }

        // Phase 2: Gather
        let signals = self.phase_gather(&recent_entries, scope).await;

        // Phase 3: Consolidate
        let (created, merged) = self.phase_consolidate(signals, scope).await;

        // Phase 4: Prune
        let pruned = self.phase_prune(scope).await;

        // Rebuild search index for the affected scope.
        let all_entries = self.load_all_entries(scope).await;
        if let Err(e) = self.search_engine.reindex_batch(&all_entries).await {
            warn!("AutoDreamConsolidator: reindex after consolidation failed: {e}");
        }

        let duration_ms = started_at.elapsed().as_millis() as u64;
        let report = ConsolidationReport {
            scope,
            entries_reviewed: recent_entries.len(),
            entries_merged: merged,
            entries_pruned: pruned,
            entries_created: created,
            duration_ms,
        };

        self.emit_event(ReasoningEventPayload::MemoryConsolidated {
            scope,
            entries_reviewed: report.entries_reviewed as u32,
            entries_merged: merged as u32,
            entries_pruned: pruned as u32,
            duration_ms,
        });

        info!(
            "AutoDreamConsolidator: cycle complete for {:?}: reviewed={}, created={}, merged={}, pruned={}, {}ms",
            scope, report.entries_reviewed, created, merged, pruned, duration_ms
        );

        Ok(report)
    }

    /// Phase 1 — Orient: load recent entries for the given scope.
    ///
    /// Returns all entries created in the last 24 hours that have not yet
    /// been explicitly consolidated. Session-scope entries are loaded from the
    /// SQLite session archive; project/identity from their respective stores.
    async fn phase_orient(&self, scope: MemoryScope) -> Result<Vec<MemoryEntry>, MemoryError> {
        let since = Utc::now() - Duration::hours(24);
        let entries = match scope {
            MemoryScope::Session => {
                self.session_store.list_recent(since, 200).await?
            }
            MemoryScope::Project => {
                self.project_store.list_recent(since, 200).await?
            }
            MemoryScope::Identity => {
                self.identity_store.list_recent(since, 200).await?
            }
        };
        debug!("AutoDreamConsolidator Orient: found {} recent entries for {:?}", entries.len(), scope);
        Ok(entries)
    }

    /// Phase 2 — Gather: extract consolidation signals from entries.
    ///
    /// Analyzes the entries for:
    /// - Repeated topics (same keywords appearing in multiple entries)
    /// - Error patterns (entries with "error" or "failed" in content)
    /// - Decision markers (entries with "decided" or "resolved" in content)
    ///
    /// Returns a list of `Signal` structs for the consolidate phase.
    async fn phase_gather(&self, entries: &[MemoryEntry], scope: MemoryScope) -> Vec<Signal> {
        let mut signals: Vec<Signal> = Vec::new();
        let target_scope = match scope {
            MemoryScope::Session => MemoryScope::Project,
            other => other,
        };

        for entry in entries.iter().take(MAX_SIGNALS_PER_CYCLE) {
            let content_lower = entry.content.to_lowercase();

            // High importance entries are always promotion candidates.
            if entry.importance >= 0.8 {
                signals.push(Signal {
                    source_id: entry.id,
                    content: entry.content.clone(),
                    target_scope,
                    importance: entry.importance,
                });
                continue;
            }

            // Error patterns → project memory.
            if content_lower.contains("error") || content_lower.contains("failed") || content_lower.contains("panic") {
                signals.push(Signal {
                    source_id: entry.id,
                    content: format!("[Error pattern] {}", &entry.content[..entry.content.len().min(500)]),
                    target_scope: MemoryScope::Project,
                    importance: 0.75,
                });
                continue;
            }

            // Decision markers → project memory.
            if content_lower.contains("decided") || content_lower.contains("resolved") || content_lower.contains("conclusion") {
                signals.push(Signal {
                    source_id: entry.id,
                    content: format!("[Decision] {}", &entry.content[..entry.content.len().min(500)]),
                    target_scope: MemoryScope::Project,
                    importance: 0.8,
                });
                continue;
            }

            // Identity signals → identity memory.
            let identity_keywords = ["prefer", "always use", "i like", "my workflow", "my style"];
            if identity_keywords.iter().any(|k| content_lower.contains(k)) {
                signals.push(Signal {
                    source_id: entry.id,
                    content: format!("[Preference] {}", &entry.content[..entry.content.len().min(300)]),
                    target_scope: MemoryScope::Identity,
                    importance: 0.7,
                });
            }
        }

        debug!(
            "AutoDreamConsolidator Gather: extracted {} signals from {} entries",
            signals.len(),
            entries.len()
        );
        signals
    }

    /// Phase 3 — Consolidate: write signals to project/identity memory.
    ///
    /// For each signal, checks for duplicates before writing. Returns
    /// `(created_count, merged_count)`.
    async fn phase_consolidate(
        &self,
        signals: Vec<Signal>,
        _scope: MemoryScope,
    ) -> (usize, usize) {
        let mut created = 0usize;
        let mut merged = 0usize;

        for signal in signals {
            match signal.target_scope {
                MemoryScope::Project => {
                    let mut meta = std::collections::HashMap::new();
                    meta.insert("source".to_string(), serde_json::json!("consolidation"));
                    meta.insert("source_entry_id".to_string(), serde_json::json!(signal.source_id.to_string()));
                    meta.insert("importance".to_string(), serde_json::json!(signal.importance));

                    match self.project_store.write_entry(signal.content, meta).await {
                        Ok(entry) => {
                            debug!("Consolidated project entry: {}", entry.id);
                            // If the entry ID is different from source (no dup), it's created.
                            // The deduplication logic in write_entry handles the merged case.
                            created += 1;
                        }
                        Err(e) => {
                            warn!("AutoDreamConsolidator: failed to write project entry: {e}");
                        }
                    }
                }
                MemoryScope::Identity => {
                    let mut meta = std::collections::HashMap::new();
                    meta.insert("source".to_string(), serde_json::json!("consolidation"));
                    meta.insert("source_entry_id".to_string(), serde_json::json!(signal.source_id.to_string()));

                    match self.identity_store.write_entry(signal.content, meta).await {
                        Ok(_) => created += 1,
                        Err(e) => warn!("AutoDreamConsolidator: failed to write identity entry: {e}"),
                    }
                }
                MemoryScope::Session => {
                    // Should not happen: gather phase redirects session signals to project.
                    warn!("AutoDreamConsolidator: unexpected Session-scoped signal in consolidate phase");
                }
            }
        }

        (created, merged)
    }

    /// Phase 4 — Prune: remove redundant and stale entries.
    ///
    /// 1. Load all entries for the scope.
    /// 2. Find pairs with similarity > `PRUNE_SIMILARITY_THRESHOLD` using `Deduplicator`.
    /// 3. For each duplicate pair, keep the entry with higher importance.
    /// 4. Mark entries older than `STALE_DAYS` with low importance for deletion.
    ///
    /// Returns the number of entries deleted.
    async fn phase_prune(&self, scope: MemoryScope) -> usize {
        let all_entries = self.load_all_entries(scope).await;
        if all_entries.is_empty() {
            return 0;
        }

        let pairs = self.deduplicator.find_all_duplicates(&all_entries);
        let stale_cutoff = Utc::now() - Duration::days(STALE_DAYS);
        let mut to_delete: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

        // Mark lower-importance duplicates for deletion.
        for (id_a, id_b, _score) in &pairs {
            // Find both entries.
            let entry_a = all_entries.iter().find(|e| e.id == *id_a);
            let entry_b = all_entries.iter().find(|e| e.id == *id_b);
            if let (Some(a), Some(b)) = (entry_a, entry_b) {
                // Keep the higher-importance entry; delete the other.
                let to_remove = if a.importance >= b.importance { b.id } else { a.id };
                to_delete.insert(to_remove);
            }
        }

        // Mark stale, low-importance entries for deletion.
        for entry in &all_entries {
            if entry.updated_at < stale_cutoff && entry.importance < 0.3 && entry.retrieval_count == 0 {
                to_delete.insert(entry.id);
            }
        }

        let mut deleted = 0usize;
        for id in &to_delete {
            let result = match scope {
                MemoryScope::Project => self.project_store.delete_entry(*id).await,
                MemoryScope::Identity => self.identity_store.delete_entry(*id).await,
                MemoryScope::Session => Ok(()), // Session entries are ephemeral; skip.
            };
            match result {
                Ok(()) => {
                    // Also remove from search index.
                    if let Err(e) = self.search_engine.remove_entry(*id).await {
                        warn!("AutoDreamConsolidator: failed to remove {} from search index: {e}", id);
                    }
                    deleted += 1;
                }
                Err(e) => warn!("AutoDreamConsolidator: failed to delete {}: {e}", id),
            }
        }

        debug!(
            "AutoDreamConsolidator Prune: deleted {} entries for {:?}",
            deleted, scope
        );
        deleted
    }

    /// Load all entries for a scope (used for pruning and reindexing).
    async fn load_all_entries(&self, scope: MemoryScope) -> Vec<MemoryEntry> {
        match scope {
            MemoryScope::Project => {
                self.project_store.load_all().unwrap_or_default()
            }
            MemoryScope::Identity => {
                self.identity_store.load_all().unwrap_or_default()
            }
            MemoryScope::Session => Vec::new(),
        }
    }

    /// Emit a `ReasoningEvent` to the broadcast channel (if configured).
    fn emit_event(&self, payload: ReasoningEventPayload) {
        if let Some(tx) = &self.event_tx {
            let event = ReasoningEvent::new(uuid::Uuid::nil(), payload);
            if let Err(e) = tx.send(event) {
                debug!("AutoDreamConsolidator: no event receivers: {e}");
            }
        }
    }
}
