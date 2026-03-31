//! `ObsidianWatcher` — filesystem watcher for the Obsidian vault directory.
//!
//! Uses the `notify` crate to watch for file creation, modification, and deletion
//! events in the vault directory. Events are debounced (500 ms) to avoid
//! rapid-fire reindexing when an editor saves multiple times in quick succession.
//!
//! The watcher runs as a background `tokio` task. When vault changes are detected,
//! it sends the changed file paths to the `Reindexer` via a channel.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use truenorth_core::traits::memory::MemoryError;
use crate::obsidian::reindexer::Reindexer;

/// Debounce interval: events within this window are coalesced into one.
const DEBOUNCE_MS: u64 = 500;

/// Vault file change event.
#[derive(Debug, Clone)]
pub struct VaultChangeEvent {
    /// Paths that were created or modified.
    pub modified: Vec<PathBuf>,
    /// Paths that were deleted.
    pub deleted: Vec<PathBuf>,
}

/// Background Obsidian vault watcher.
///
/// Watches the vault directory for filesystem events and triggers `Reindexer`
/// when Markdown files change. The watcher runs as a background tokio task
/// and can be stopped by dropping the handle.
#[derive(Debug)]
pub struct ObsidianWatcher {
    /// Path to the vault root directory being watched.
    vault_dir: PathBuf,
    /// Reindexer called when changes are detected.
    reindexer: Arc<Reindexer>,
}

impl ObsidianWatcher {
    /// Create a new `ObsidianWatcher` for `vault_dir`.
    ///
    /// Does not start watching until [`start`] is called.
    pub fn new(vault_dir: PathBuf, reindexer: Arc<Reindexer>) -> Self {
        Self { vault_dir, reindexer }
    }

    /// Start the vault watcher as a background tokio task.
    ///
    /// The watcher runs until the returned `tokio::task::JoinHandle` is dropped
    /// or the watcher encounters a fatal error.
    ///
    /// # Errors
    ///
    /// Returns `MemoryError::StorageError` if the `notify` watcher cannot be
    /// initialized for `vault_dir`.
    pub fn start(&self) -> Result<tokio::task::JoinHandle<()>, MemoryError> {
        let vault_dir = self.vault_dir.clone();
        let reindexer = self.reindexer.clone();

        if !vault_dir.exists() {
            return Err(MemoryError::StorageError {
                message: format!("Vault dir does not exist: {}", vault_dir.display()),
            });
        }

        // Channel from the synchronous notify callback to the async handler.
        let (tx, mut rx) = mpsc::unbounded_channel::<notify::Event>();

        // Build the notify watcher (synchronous).
        let tx_clone = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(event) => {
                    if let Err(e) = tx_clone.send(event) {
                        error!("ObsidianWatcher: failed to send event: {e}");
                    }
                }
                Err(e) => warn!("ObsidianWatcher: watch error: {e}"),
            }
        })
        .map_err(|e| MemoryError::StorageError {
            message: format!("Cannot create notify watcher: {e}"),
        })?;

        watcher
            .watch(&vault_dir, RecursiveMode::Recursive)
            .map_err(|e| MemoryError::StorageError {
                message: format!("Cannot watch vault dir: {e}"),
            })?;

        info!("ObsidianWatcher started for {}", vault_dir.display());

        // Spawn the async event loop.
        let handle = tokio::spawn(async move {
            // Keep the watcher alive inside the task.
            let _watcher = watcher;

            let mut pending_modified: Vec<PathBuf> = Vec::new();
            let mut pending_deleted: Vec<PathBuf> = Vec::new();
            let debounce = Duration::from_millis(DEBOUNCE_MS);

            loop {
                // Wait for an event or the debounce timer to expire.
                let timed_out = tokio::time::timeout(debounce, rx.recv()).await;

                match timed_out {
                    Ok(Some(event)) => {
                        // Accumulate events.
                        Self::categorize_event(event, &mut pending_modified, &mut pending_deleted);
                    }
                    Ok(None) => {
                        // Channel closed — watcher is done.
                        break;
                    }
                    Err(_) => {
                        // Debounce timer expired — flush pending events.
                        if !pending_modified.is_empty() || !pending_deleted.is_empty() {
                            let modified = std::mem::take(&mut pending_modified);
                            let deleted = std::mem::take(&mut pending_deleted);
                            let event = VaultChangeEvent { modified, deleted };
                            debug!("ObsidianWatcher: flushing {} modified, {} deleted",
                                event.modified.len(), event.deleted.len());
                            if let Err(e) = reindexer.handle_vault_change(event).await {
                                warn!("ObsidianWatcher: reindexer error: {e}");
                            }
                        }
                    }
                }
            }

            info!("ObsidianWatcher task exited");
        });

        Ok(handle)
    }

    /// Categorize a notify event into the modified/deleted buckets.
    ///
    /// Only processes Markdown files (`.md` extension).
    fn categorize_event(
        event: notify::Event,
        modified: &mut Vec<PathBuf>,
        deleted: &mut Vec<PathBuf>,
    ) {
        let md_paths: Vec<PathBuf> = event
            .paths
            .into_iter()
            .filter(|p| p.extension().map(|e| e == "md").unwrap_or(false))
            .collect();

        if md_paths.is_empty() {
            return;
        }

        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for p in md_paths {
                    if !modified.contains(&p) {
                        modified.push(p);
                    }
                }
            }
            EventKind::Remove(_) => {
                for p in md_paths {
                    // Remove from modified if it was there, add to deleted.
                    modified.retain(|m| m != &p);
                    if !deleted.contains(&p) {
                        deleted.push(p);
                    }
                }
            }
            _ => {} // Access events, etc. — ignore.
        }
    }

    /// Return the vault directory being watched.
    pub fn vault_dir(&self) -> &PathBuf {
        &self.vault_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_categorize_creates() {
        let path_md = PathBuf::from("/vault/test.md");
        let path_txt = PathBuf::from("/vault/test.txt");

        let event = notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::File),
            paths: vec![path_md.clone(), path_txt],
            attrs: Default::default(),
        };

        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        ObsidianWatcher::categorize_event(event, &mut modified, &mut deleted);

        // Only the .md file should be tracked.
        assert_eq!(modified, vec![path_md]);
        assert!(deleted.is_empty());
    }
}
