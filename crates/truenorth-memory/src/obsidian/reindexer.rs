//! `Reindexer` — handles bidirectional Obsidian vault synchronization.
//!
//! When the `ObsidianWatcher` detects changes in the vault directory, the
//! `Reindexer` is called to:
//!
//! 1. Parse the changed Markdown files using `MarkdownWriter::parse_markdown`.
//! 2. Re-embed the content if an embedding provider is available.
//! 3. Upsert the parsed entries into the appropriate SQLite store (project or identity).
//! 4. Update the Tantivy index and semantic vector store.
//!
//! ## TrueNorth → Obsidian direction
//!
//! New entries written by TrueNorth are handled by `MarkdownWriter` in the
//! `ProjectMemoryStore`. No reindexer involvement is needed.
//!
//! ## Obsidian → TrueNorth direction
//!
//! User edits in Obsidian trigger a filesystem event. The `Reindexer` parses the
//! edited file, extracts the `id` from the YAML frontmatter, and upserts the entry
//! with the new content into SQLite. This ensures user annotations are searchable.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::{debug, info, warn};

use truenorth_core::traits::embedding_provider::EmbeddingProvider;
use truenorth_core::traits::memory::MemoryError;
use truenorth_core::types::memory::MemoryScope;

use crate::identity::sqlite_store::IdentityMemoryStore;
use crate::obsidian::watcher::VaultChangeEvent;
use crate::project::markdown_writer::MarkdownWriter;
use crate::project::sqlite_store::ProjectMemoryStore;
use crate::search::SearchEngine;

/// Processes vault change events and keeps the memory layer in sync with Obsidian.
#[derive(Debug)]
pub struct Reindexer {
    /// Markdown writer/parser (project scope).
    project_writer: Arc<MarkdownWriter>,
    /// Markdown writer/parser (identity scope).
    identity_writer: Arc<MarkdownWriter>,
    /// Project SQLite store.
    project_store: Arc<ProjectMemoryStore>,
    /// Identity SQLite store.
    identity_store: Arc<IdentityMemoryStore>,
    /// Search engine for index updates.
    search_engine: Arc<SearchEngine>,
    /// Optional embedding provider for re-embedding edited content.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl Reindexer {
    /// Create a new `Reindexer`.
    pub fn new(
        project_writer: Arc<MarkdownWriter>,
        identity_writer: Arc<MarkdownWriter>,
        project_store: Arc<ProjectMemoryStore>,
        identity_store: Arc<IdentityMemoryStore>,
        search_engine: Arc<SearchEngine>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        Self {
            project_writer,
            identity_writer,
            project_store,
            identity_store,
            search_engine,
            embedding_provider,
        }
    }

    /// Handle a batch of vault change events.
    ///
    /// Processes all modified and deleted files. Modified files are parsed and
    /// upserted; deleted files are removed from the stores and search indices.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered. Processing continues for other files
    /// even if one fails.
    pub async fn handle_vault_change(
        &self,
        event: VaultChangeEvent,
    ) -> Result<(), MemoryError> {
        let mut errors: Vec<String> = Vec::new();

        // Handle modified/created files.
        for path in &event.modified {
            if let Err(e) = self.process_modified_file(path).await {
                let msg = format!("Failed to reindex {}: {e}", path.display());
                warn!("{}", msg);
                errors.push(msg);
            }
        }

        // Handle deleted files.
        for path in &event.deleted {
            if let Err(e) = self.process_deleted_file(path).await {
                let msg = format!("Failed to remove deleted file {}: {e}", path.display());
                warn!("{}", msg);
                errors.push(msg);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            // Return the first error as representative.
            Err(MemoryError::StorageError {
                message: errors.join("; "),
            })
        }
    }

    /// Process a single modified Markdown file.
    ///
    /// 1. Reads the file from disk.
    /// 2. Parses the YAML frontmatter to extract the entry ID and scope.
    /// 3. Re-embeds the content if an embedding provider is available.
    /// 4. Upserts the entry into the appropriate SQLite store.
    /// 5. Updates the search index.
    async fn process_modified_file(&self, path: &PathBuf) -> Result<(), MemoryError> {
        let raw = fs::read_to_string(path).map_err(|e| MemoryError::StorageError {
            message: format!("Cannot read vault file {}: {e}", path.display()),
        })?;

        // Determine which writer to use based on path.
        let writer = self.writer_for_path(path);
        let mut entry = match writer.parse_markdown(&raw) {
            Some(e) => e,
            None => {
                debug!("Reindexer: could not parse frontmatter in {}", path.display());
                return Ok(());
            }
        };

        // Re-embed the content if provider is available.
        if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&entry.content).await {
                Ok(vec) => entry.embedding = Some(vec),
                Err(e) => warn!("Reindexer: embed failed for {}: {e}", entry.id),
            }
        }

        // Upsert into the correct store.
        match entry.scope {
            MemoryScope::Project => {
                self.project_store.upsert_entry(&entry)?;
            }
            MemoryScope::Identity => {
                self.identity_store.upsert_entry(&entry)?;
            }
            MemoryScope::Session => {
                // Session entries are ephemeral; skip.
                debug!("Reindexer: skipping session-scoped file {}", path.display());
                return Ok(());
            }
        }

        // Update search index.
        self.search_engine.index_entry(&entry).await?;

        info!("Reindexer: synced {} ({:?}) from Obsidian edit", entry.id, entry.scope);
        Ok(())
    }

    /// Process a deleted Markdown file.
    ///
    /// Extracts the entry UUID from the filename (expected format: `<uuid>.md`),
    /// then removes the entry from the search index. The SQLite entry is left
    /// intact unless the user explicitly deletes it via the CLI.
    async fn process_deleted_file(&self, path: &PathBuf) -> Result<(), MemoryError> {
        let uuid = extract_uuid_from_path(path);
        if let Some(id) = uuid {
            self.search_engine.remove_entry(id).await?;
            info!("Reindexer: removed {} from search index (Obsidian delete)", id);
        } else {
            debug!("Reindexer: could not extract UUID from deleted path {}", path.display());
        }
        Ok(())
    }

    /// Determine the appropriate `MarkdownWriter` based on the file path.
    ///
    /// If the path is inside the identity writer's directory, use the identity
    /// writer; otherwise fall back to the project writer.
    fn writer_for_path(&self, path: &PathBuf) -> &MarkdownWriter {
        if path.starts_with(self.identity_writer.dir()) {
            &self.identity_writer
        } else {
            &self.project_writer
        }
    }

    /// Trigger a full re-sync of all Markdown files in both vault directories.
    ///
    /// Iterates over all `.md` files in the project and identity vault directories,
    /// processes each one, and rebuilds the search index. Used after startup to
    /// pick up any changes made while TrueNorth was offline.
    pub async fn full_resync(&self) -> Result<usize, MemoryError> {
        let mut count = 0usize;

        let project_files = self.project_writer.list_files();
        let identity_files = self.identity_writer.list_files();
        let all_files: Vec<PathBuf> = project_files.into_iter().chain(identity_files).collect();

        for path in &all_files {
            match self.process_modified_file(path).await {
                Ok(()) => count += 1,
                Err(e) => warn!("Reindexer full_resync: error on {}: {e}", path.display()),
            }
        }

        info!("Reindexer: full resync processed {} files", count);
        Ok(count)
    }
}

/// Extract a UUID from a filename of the form `<uuid>.md`.
fn extract_uuid_from_path(path: &PathBuf) -> Option<uuid::Uuid> {
    let stem = path.file_stem()?.to_str()?;
    uuid::Uuid::parse_str(stem).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_uuid_from_path() {
        let id = uuid::Uuid::new_v4();
        let path = PathBuf::from(format!("/vault/{}.md", id));
        assert_eq!(extract_uuid_from_path(&path), Some(id));
    }

    #[test]
    fn test_extract_uuid_invalid() {
        let path = PathBuf::from("/vault/not-a-uuid.md");
        assert!(extract_uuid_from_path(&path).is_none());
    }
}
