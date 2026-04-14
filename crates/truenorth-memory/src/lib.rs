//! `truenorth-memory` — Three-tier memory layer for TrueNorth.
//!
//! This crate provides the complete memory infrastructure for TrueNorth, organized
//! as three distinct tiers:
//!
//! - **Session** (`MemoryScope::Session`): Ephemeral in-memory storage for the
//!   active conversation. Cleared on session end (optionally persisted to SQLite
//!   for consolidation). Fast `Arc<RwLock<HashMap>>` backend.
//!
//! - **Project** (`MemoryScope::Project`): Persistent SQLite + Markdown storage
//!   scoped to a single project. Survives session restarts. Synced bidirectionally
//!   with an Obsidian vault via the filesystem watcher.
//!
//! - **Identity** (`MemoryScope::Identity`): Persistent SQLite storage shared
//!   across all projects. Holds user preferences, communication style, and
//!   long-term workflow patterns discovered by the dialectic modeler.
//!
//! ## Key subsystems
//!
//! - [`search`] — Full-text (Tantivy/BM25), semantic (cosine), and hybrid (RRF) search.
//! - [`obsidian`] — Vault file watcher and bidirectional sync with Obsidian Markdown.
//! - [`consolidation`] — AutoDream-style Orient → Gather → Consolidate → Prune cycle.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use truenorth_memory::MemoryLayer;
//! use truenorth_memory::MemoryLayerConfig;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = MemoryLayerConfig::default();
//! let memory = MemoryLayer::builder()
//!     .with_config(config)
//!     .build()
//!     .await?;
//! # Ok(())
//! # }
//! ```

pub mod consolidation;
pub mod identity;
pub mod obsidian;
pub mod project;
pub mod search;
pub mod session;

// Re-export the core types that callers need without importing truenorth-core directly.
pub use truenorth_core::traits::memory::{
    CompactionResult, ConsolidationReport, MemoryError, MemoryStore,
};
pub use truenorth_core::types::memory::{
    MemoryEntry, MemoryMetadata, MemoryQuery, MemoryScope, MemorySearchResult, MemorySearchType,
};

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::consolidation::scheduler::ConsolidationScheduler;
use crate::consolidation::consolidator::AutoDreamConsolidator;
use crate::identity::sqlite_store::IdentityMemoryStore;
use crate::project::sqlite_store::ProjectMemoryStore;
use crate::search::SearchEngine;
use crate::session::store::SessionMemoryStore;

/// Configuration for the complete three-tier memory layer.
#[derive(Debug, Clone)]
pub struct MemoryLayerConfig {
    /// Root directory for SQLite databases and Markdown vault files.
    /// Defaults to `./memory` relative to the current working directory.
    pub memory_root: PathBuf,

    /// Path to the SQLite database file for project memory.
    /// Defaults to `<memory_root>/project.db`.
    pub project_db_path: Option<PathBuf>,

    /// Path to the SQLite database file for identity memory.
    /// Defaults to `<memory_root>/identity.db`.
    pub identity_db_path: Option<PathBuf>,

    /// Directory where Obsidian-compatible Markdown files are written.
    /// Defaults to `<memory_root>/vault`.
    pub vault_dir: Option<PathBuf>,

    /// Cosine similarity threshold for semantic deduplication (0.0–1.0).
    /// Entries above this threshold are considered duplicates. Default: 0.85.
    pub dedup_threshold: f32,

    /// Minimum hours between automatic consolidation runs.
    pub consolidation_min_interval_hours: u64,

    /// Minimum new sessions required before triggering consolidation.
    pub consolidation_min_sessions: usize,

    /// Whether to start the Obsidian vault watcher automatically.
    pub watch_vault: bool,
}

impl Default for MemoryLayerConfig {
    fn default() -> Self {
        Self {
            memory_root: PathBuf::from("memory"),
            project_db_path: None,
            identity_db_path: None,
            vault_dir: None,
            dedup_threshold: 0.85,
            consolidation_min_interval_hours: 8,
            consolidation_min_sessions: 1,
            watch_vault: false,
        }
    }
}

/// Builder for [`MemoryLayer`].
#[derive(Default)]
pub struct MemoryLayerBuilder {
    config: MemoryLayerConfig,
    embedding_provider: Option<Arc<dyn truenorth_core::traits::embedding_provider::EmbeddingProvider>>,
    #[allow(dead_code)]
    event_tx: Option<tokio::sync::broadcast::Sender<truenorth_core::types::event::ReasoningEvent>>,
}

impl MemoryLayerBuilder {
    /// Set the configuration for the memory layer.
    pub fn with_config(mut self, config: MemoryLayerConfig) -> Self {
        self.config = config;
        self
    }

    /// Provide an embedding provider for semantic search and deduplication.
    pub fn with_embedding_provider(
        mut self,
        provider: Arc<dyn truenorth_core::traits::embedding_provider::EmbeddingProvider>,
    ) -> Self {
        self.embedding_provider = Some(provider);
        self
    }

    /// Provide a broadcast sender for emitting `ReasoningEvent`s.
    pub fn with_event_sender(
        mut self,
        tx: tokio::sync::broadcast::Sender<truenorth_core::types::event::ReasoningEvent>,
    ) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Consume the builder and construct the [`MemoryLayer`].
    ///
    /// Opens SQLite connections, builds Tantivy indices, and optionally starts
    /// the Obsidian vault watcher.
    pub async fn build(self) -> Result<MemoryLayer, MemoryError> {
        let config = self.config;
        let root = &config.memory_root;

        // Ensure root directory exists.
        std::fs::create_dir_all(root).map_err(|e| MemoryError::StorageError {
            message: format!("Failed to create memory root dir: {e}"),
        })?;

        let project_db = config
            .project_db_path
            .clone()
            .unwrap_or_else(|| root.join("project.db"));

        let identity_db = config
            .identity_db_path
            .clone()
            .unwrap_or_else(|| root.join("identity.db"));

        let vault_dir = config
            .vault_dir
            .clone()
            .unwrap_or_else(|| root.join("vault"));

        std::fs::create_dir_all(&vault_dir).map_err(|e| MemoryError::StorageError {
            message: format!("Failed to create vault dir: {e}"),
        })?;

        let tantivy_index_dir = root.join("tantivy_index");
        std::fs::create_dir_all(&tantivy_index_dir).map_err(|e| MemoryError::StorageError {
            message: format!("Failed to create tantivy dir: {e}"),
        })?;

        // Build sub-stores.
        let session_store = Arc::new(SessionMemoryStore::new(
            project_db.clone(),
            self.embedding_provider.clone(),
        )?);

        let project_store = Arc::new(ProjectMemoryStore::new(
            project_db,
            vault_dir.clone(),
            self.embedding_provider.clone(),
            config.dedup_threshold,
        )?);

        let identity_store = Arc::new(IdentityMemoryStore::new(
            identity_db,
            self.embedding_provider.clone(),
            config.dedup_threshold,
        )?);

        // Build the search engine (Tantivy + semantic).
        let search_engine = Arc::new(
            SearchEngine::new(tantivy_index_dir, self.embedding_provider.clone())
                .map_err(|e| MemoryError::SearchIndexError {
                    message: format!("Failed to build search engine: {e}"),
                })?,
        );

        let event_tx = self.event_tx;

        // Build consolidator and scheduler.
        let consolidator = Arc::new(AutoDreamConsolidator::new(
            session_store.clone(),
            project_store.clone(),
            identity_store.clone(),
            search_engine.clone(),
            event_tx.clone(),
        ));

        let scheduler = ConsolidationScheduler::new(
            consolidator.clone(),
            config.consolidation_min_interval_hours,
            config.consolidation_min_sessions,
        );

        let layer = MemoryLayer {
            config: config.clone(),
            session_store,
            project_store,
            identity_store,
            search_engine,
            consolidator,
            scheduler: Arc::new(RwLock::new(scheduler)),
            vault_dir,
            event_tx,
        };

        info!("MemoryLayer initialized at {}", config.memory_root.display());
        Ok(layer)
    }
}

/// The three-tier memory layer — facade over session, project, and identity stores.
///
/// This is the primary entry point for all memory operations. Callers use this
/// struct directly rather than interacting with the individual tier stores.
///
/// # Threading
///
/// `MemoryLayer` is `Clone`, `Send`, and `Sync`. All state is protected by
/// `Arc<>` wrappers internally.
#[derive(Clone, Debug)]
pub struct MemoryLayer {
    config: MemoryLayerConfig,
    /// In-memory session tier.
    pub session_store: Arc<SessionMemoryStore>,
    /// SQLite-backed project tier.
    pub project_store: Arc<ProjectMemoryStore>,
    /// SQLite-backed identity tier.
    pub identity_store: Arc<IdentityMemoryStore>,
    /// Unified search engine (Tantivy + semantic + hybrid).
    pub search_engine: Arc<SearchEngine>,
    /// AutoDream consolidator.
    pub consolidator: Arc<AutoDreamConsolidator>,
    /// Background consolidation scheduler.
    scheduler: Arc<RwLock<ConsolidationScheduler>>,
    /// Root directory of the Obsidian vault.
    vault_dir: PathBuf,
    /// Optional broadcast sender for emitting reasoning events.
    #[allow(dead_code)]
    event_tx: Option<tokio::sync::broadcast::Sender<truenorth_core::types::event::ReasoningEvent>>,
}

impl MemoryLayer {
    /// Create a new [`MemoryLayerBuilder`].
    pub fn builder() -> MemoryLayerBuilder {
        MemoryLayerBuilder::default()
    }

    /// Returns a reference to the current configuration.
    pub fn config(&self) -> &MemoryLayerConfig {
        &self.config
    }

    /// Route a write operation to the correct tier based on scope.
    #[instrument(skip(self, content, metadata), fields(scope = ?scope))]
    pub async fn write(
        &self,
        content: String,
        scope: MemoryScope,
        metadata: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<MemoryEntry, MemoryError> {
        debug!("MemoryLayer::write scope={:?}", scope);
        let entry = match scope {
            MemoryScope::Session => {
                self.session_store
                    .write_entry(content, metadata)
                    .await?
            }
            MemoryScope::Project => {
                self.project_store
                    .write_entry(content, metadata)
                    .await?
            }
            MemoryScope::Identity => {
                self.identity_store
                    .write_entry(content, metadata)
                    .await?
            }
        };

        // Index the entry in Tantivy for full-text search.
        if let Err(e) = self.search_engine.index_entry(&entry).await {
            tracing::warn!("Failed to index memory entry in Tantivy: {e}");
        }

        Ok(entry)
    }

    /// Read a single entry by ID, searching across all tiers.
    #[instrument(skip(self), fields(id = %id))]
    pub async fn read(&self, id: Uuid) -> Result<MemoryEntry, MemoryError> {
        // Try each tier in order.
        if let Ok(e) = self.session_store.get_entry(id).await {
            return Ok(e);
        }
        if let Ok(e) = self.project_store.get_entry(id).await {
            return Ok(e);
        }
        if let Ok(e) = self.identity_store.get_entry(id).await {
            return Ok(e);
        }
        Err(MemoryError::EntryNotFound { id })
    }

    /// Perform a full-text search (Tantivy BM25) within the specified scope.
    #[instrument(skip(self), fields(query = query, scope = ?scope))]
    pub async fn search_text(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        self.search_engine.fulltext_search(query, scope, limit).await
    }

    /// Perform a semantic similarity search within the specified scope.
    #[instrument(skip(self), fields(query = query, scope = ?scope))]
    pub async fn search_semantic(
        &self,
        query: &str,
        scope: MemoryScope,
        top_k: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        self.search_engine.semantic_search(query, scope, top_k).await
    }

    /// Perform a hybrid search (RRF of fulltext + semantic) within the specified scope.
    #[instrument(skip(self), fields(query = query, scope = ?scope))]
    pub async fn search_hybrid(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, MemoryError> {
        self.search_engine.hybrid_search(query, scope, limit).await
    }

    /// List recent entries for a scope, ordered by `created_at` descending.
    pub async fn list_recent(
        &self,
        scope: MemoryScope,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        match scope {
            MemoryScope::Session => self.session_store.list_recent(since, limit).await,
            MemoryScope::Project => self.project_store.list_recent(since, limit).await,
            MemoryScope::Identity => self.identity_store.list_recent(since, limit).await,
        }
    }

    /// Compact a session's conversation history via LLM summarization.
    pub async fn compact_session(
        &self,
        session_id: Uuid,
        budget_hint: usize,
    ) -> Result<CompactionResult, MemoryError> {
        self.session_store.compact(session_id, budget_hint).await
    }

    /// Run an immediate consolidation cycle for the given scope.
    pub async fn consolidate_now(
        &self,
        scope: MemoryScope,
    ) -> Result<ConsolidationReport, MemoryError> {
        self.consolidator.run(scope).await
    }

    /// Signal the consolidation scheduler that a session has ended.
    ///
    /// The scheduler will check its gates and may trigger a background
    /// consolidation cycle.
    pub async fn notify_session_end(&self, session_id: Uuid) {
        let mut sched = self.scheduler.write().await;
        sched.on_session_end(session_id).await;
    }

    /// Delete a single memory entry by ID across all tiers.
    pub async fn delete(&self, id: Uuid) -> Result<(), MemoryError> {
        // Attempt deletion in each tier; ignore NotFound errors.
        let _ = self.session_store.delete_entry(id).await;
        let _ = self.project_store.delete_entry(id).await;
        let _ = self.identity_store.delete_entry(id).await;
        // Remove from Tantivy index.
        self.search_engine.remove_entry(id).await?;
        Ok(())
    }

    /// Record a retrieval event to bump the entry's importance score.
    pub async fn record_retrieval(&self, id: Uuid) -> Result<(), MemoryError> {
        // Try each tier.
        let r1 = self.session_store.record_retrieval(id).await;
        let r2 = self.project_store.record_retrieval(id).await;
        let r3 = self.identity_store.record_retrieval(id).await;
        // Return success if at least one tier found and updated the entry.
        if r1.is_ok() || r2.is_ok() || r3.is_ok() {
            Ok(())
        } else {
            Err(MemoryError::EntryNotFound { id })
        }
    }

    /// Return the path to the Obsidian vault directory.
    pub fn vault_dir(&self) -> &PathBuf {
        &self.vault_dir
    }

    /// Start the background Obsidian vault watcher and consolidation scheduler.
    ///
    /// Should be called once after construction when the embedding provider and
    /// event bus are ready.
    pub async fn start_background_tasks(&self) -> Result<(), MemoryError> {
        info!("Starting MemoryLayer background tasks");
        // Start the consolidation scheduler loop.
        let scheduler = self.scheduler.clone();
        tokio::spawn(async move {
            let sched = scheduler.read().await;
            sched.run_loop().await;
        });
        Ok(())
    }
}
