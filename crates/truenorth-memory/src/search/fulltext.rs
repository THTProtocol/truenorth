//! `TantivySearch` — BM25 full-text search over memory entries.
//!
//! Builds and maintains a Tantivy index with the following fields:
//! - `id` (STORED, not indexed): entry UUID string.
//! - `content` (TEXT, BM25 indexed): the primary content of the entry.
//! - `scope` (TEXT, STORED, FAST): entry scope for filtering.
//! - `metadata_text` (TEXT, indexed): flattened metadata values for search.
//!
//! Tantivy operations are synchronous (Tantivy does not use async I/O). All calls
//! to this struct should be wrapped in `tokio::task::spawn_blocking` at the
//! call site (the `SearchEngine` facade handles this automatically).

use std::path::PathBuf;
use std::sync::Arc;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, Schema, SchemaBuilder, Value, FAST, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};
use tracing::{debug, info, warn};
use uuid::Uuid;

use truenorth_core::types::memory::{MemoryEntry, MemoryScope, MemorySearchResult, MemorySearchType};

/// Field names in the Tantivy schema.
const FIELD_ID: &str = "id";
const FIELD_CONTENT: &str = "content";
const FIELD_SCOPE: &str = "scope";
const FIELD_METADATA_TEXT: &str = "metadata_text";

/// Tantivy-based full-text search engine.
///
/// Wraps the Tantivy index with safe concurrent read access via `IndexReader`
/// and a mutex-protected `IndexWriter` for incremental updates.
pub struct TantivySearch {
    index: Index,
    reader: IndexReader,
    writer: parking_lot::Mutex<IndexWriter>,
    // Field handles cached from schema.
    f_id: Field,
    f_content: Field,
    f_scope: Field,
    f_metadata: Field,
}

impl std::fmt::Debug for TantivySearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TantivySearch")
            .field("f_id", &self.f_id)
            .field("f_content", &self.f_content)
            .field("f_scope", &self.f_scope)
            .finish()
    }
}

impl TantivySearch {
    /// Create or open a Tantivy index at `index_dir`.
    ///
    /// If the directory is empty, a new index is created with the fixed schema.
    /// If an existing index is found, it is opened.
    ///
    /// # Errors
    ///
    /// Returns a `String` error if the index cannot be opened or the schema
    /// is incompatible.
    pub fn new(index_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&index_dir)
            .map_err(|e| format!("Cannot create Tantivy dir: {e}"))?;

        let (schema, f_id, f_content, f_scope, f_metadata) = build_schema();

        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(&index_dir).map_err(|e| format!("Cannot open Tantivy index: {e}"))?
        } else {
            Index::create_in_dir(&index_dir, schema.clone())
                .map_err(|e| format!("Cannot create Tantivy index: {e}"))?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("Cannot create Tantivy reader: {e}"))?;

        // 50 MB write buffer.
        let writer = index
            .writer(50_000_000)
            .map_err(|e| format!("Cannot create Tantivy writer: {e}"))?;

        info!("TantivySearch initialized at {}", index_dir.display());
        Ok(Self {
            index,
            reader,
            writer: parking_lot::Mutex::new(writer),
            f_id,
            f_content,
            f_scope,
            f_metadata,
        })
    }

    /// Index a single memory entry.
    ///
    /// If an entry with the same ID already exists, it is replaced (delete + add).
    ///
    /// # Errors
    ///
    /// Returns a `String` error if the Tantivy write fails.
    pub fn index_entry(&self, entry: &MemoryEntry) -> Result<(), String> {
        let id_str = entry.id.to_string();
        let metadata_text = flatten_metadata_to_text(&entry.metadata);
        let scope_str = format!("{:?}", entry.scope);

        // Build document.
        let mut doc = TantivyDocument::default();
        doc.add_text(self.f_id, &id_str);
        doc.add_text(self.f_content, &entry.content);
        doc.add_text(self.f_scope, &scope_str);
        doc.add_text(self.f_metadata, &metadata_text);

        let mut writer = self.writer.lock();

        // Delete existing document with this ID to avoid duplicates.
        let id_term = tantivy::Term::from_field_text(self.f_id, &id_str);
        writer.delete_term(id_term);

        writer.add_document(doc)
            .map_err(|e| format!("Tantivy add_document failed: {e}"))?;
        writer.commit()
            .map_err(|e| format!("Tantivy commit failed: {e}"))?;

        debug!("Tantivy indexed entry {}", entry.id);
        Ok(())
    }

    /// Remove a single entry from the index by ID.
    pub fn remove_entry(&self, id: Uuid) -> Result<(), String> {
        let id_str = id.to_string();
        let mut writer = self.writer.lock();
        let id_term = tantivy::Term::from_field_text(self.f_id, &id_str);
        writer.delete_term(id_term);
        writer.commit()
            .map_err(|e| format!("Tantivy commit (delete) failed: {e}"))?;
        debug!("Tantivy removed entry {}", id);
        Ok(())
    }

    /// Search the index with a BM25-ranked query, filtering by scope.
    ///
    /// Returns up to `limit` results with normalized scores (0.0–1.0).
    ///
    /// # Arguments
    ///
    /// * `query` - The query string. Supports Tantivy query syntax
    ///   (e.g., `"rust tokio"` or `content:async`).
    /// * `scope` - Filter results to this memory scope.
    /// * `limit` - Maximum number of results.
    pub fn search(
        &self,
        query: &str,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();

        // Build a combined query: content AND scope filter.
        // We search content + metadata_text fields and post-filter by scope.
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.f_content, self.f_metadata]);

        let parsed_query = query_parser
            .parse_query(query)
            .unwrap_or_else(|_| {
                // Fall back to a fuzzy/partial match by treating the query as a phrase.
                let safe = sanitize_query(query);
                query_parser.parse_query(&safe).unwrap_or_else(|_| {
                    Box::new(tantivy::query::AllQuery) as Box<dyn tantivy::query::Query>
                })
            });

        let scope_str = format!("{:?}", scope);
        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(limit * 4))
            .map_err(|e| format!("Tantivy search failed: {e}"))?;

        let mut results: Vec<MemorySearchResult> = Vec::new();
        let mut max_score = 0.0_f32;

        for (score, doc_address) in &top_docs {
            let doc: TantivyDocument = searcher
                .doc(*doc_address)
                .map_err(|e| format!("Tantivy retrieve doc failed: {e}"))?;

            // Extract scope field.
            let doc_scope = doc
                .get_first(self.f_scope)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if doc_scope != scope_str {
                continue; // Filter out other scopes.
            }

            let id_str = doc
                .get_first(self.f_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let content = doc
                .get_first(self.f_content)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let id = match Uuid::parse_str(&id_str) {
                Ok(id) => id,
                Err(_) => {
                    warn!("Tantivy: invalid UUID in index: {}", id_str);
                    continue;
                }
            };

            let raw_score = *score;
            if raw_score > max_score {
                max_score = raw_score;
            }

            // Build a minimal MemoryEntry from the indexed fields.
            // (Full metadata is not stored in Tantivy; callers needing full entries
            //  should look up by ID in SQLite.)
            let entry = MemoryEntry {
                id,
                scope,
                content,
                metadata: Default::default(),
                embedding: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                importance: 0.5,
                retrieval_count: 0,
            };

            results.push(MemorySearchResult {
                entry,
                score: raw_score,
                search_type: MemorySearchType::FullText,
            });
        }

        // Normalize scores to [0, 1].
        if max_score > 0.0 {
            for r in &mut results {
                r.score /= max_score;
            }
        }

        results.truncate(limit);
        Ok(results)
    }

    /// Force the reader to reload, making recently committed writes visible.
    ///
    /// Normally the reader reloads on a background schedule, but tests and
    /// latency-sensitive code paths can call this explicitly.
    pub fn reload(&self) {
        self.reader.reload().ok();
    }

    /// Rebuild the entire index from a batch of entries.
    ///
    /// Deletes all existing documents and re-adds all entries in `entries`.
    /// Used after a consolidation cycle or vault re-sync.
    pub fn reindex_all(&self, entries: &[MemoryEntry]) -> Result<(), String> {
        let mut writer = self.writer.lock();
        writer.delete_all_documents()
            .map_err(|e| format!("Tantivy delete_all failed: {e}"))?;
        writer.commit()
            .map_err(|e| format!("Tantivy commit (clear) failed: {e}"))?;
        drop(writer);

        for entry in entries {
            self.index_entry(entry)?;
        }
        info!("Tantivy: reindexed {} entries", entries.len());
        Ok(())
    }
}

/// Build the Tantivy schema and return field handles.
fn build_schema() -> (Schema, Field, Field, Field, Field) {
    let mut builder = SchemaBuilder::new();
    let f_id = builder.add_text_field(FIELD_ID, STRING | STORED);
    let f_content = builder.add_text_field(FIELD_CONTENT, TEXT | STORED);
    let f_scope = builder.add_text_field(FIELD_SCOPE, STRING | STORED | FAST);
    let f_metadata = builder.add_text_field(FIELD_METADATA_TEXT, TEXT);
    let schema = builder.build();
    (schema, f_id, f_content, f_scope, f_metadata)
}

/// Flatten a JSON metadata map to a searchable text string.
///
/// Concatenates all string values in the metadata map, separated by spaces.
fn flatten_metadata_to_text(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    metadata
        .values()
        .filter_map(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sanitize a query string to prevent Tantivy query parse errors.
///
/// Escapes special characters that have syntactic meaning in Tantivy's query syntax.
fn sanitize_query(query: &str) -> String {
    query
        .chars()
        .map(|c| match c {
            '+' | '-' | '&' | '|' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '"'
            | '~' | '*' | '?' | ':' | '\\' | '/' => ' ',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_entry(id: Uuid, content: &str, scope: MemoryScope) -> MemoryEntry {
        let now = chrono::Utc::now();
        MemoryEntry {
            id,
            scope,
            content: content.to_string(),
            metadata: HashMap::new(),
            embedding: None,
            created_at: now,
            updated_at: now,
            importance: 0.5,
            retrieval_count: 0,
        }
    }

    #[test]
    fn test_index_and_search() {
        let tmp = TempDir::new().unwrap();
        let ts = TantivySearch::new(tmp.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        let entry = make_entry(id, "Rust async programming with Tokio", MemoryScope::Project);
        ts.index_entry(&entry).unwrap();
        ts.reload(); // Force reader to see committed writes.

        let results = ts.search("async Tokio", MemoryScope::Project, 5).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.id, id);
    }

    #[test]
    fn test_scope_filter() {
        let tmp = TempDir::new().unwrap();
        let ts = TantivySearch::new(tmp.path().to_path_buf()).unwrap();
        let id_proj = Uuid::new_v4();
        let id_id = Uuid::new_v4();
        ts.index_entry(&make_entry(id_proj, "blockchain development", MemoryScope::Project)).unwrap();
        ts.index_entry(&make_entry(id_id, "blockchain development", MemoryScope::Identity)).unwrap();
        ts.reload();

        let results = ts.search("blockchain", MemoryScope::Project, 5).unwrap();
        assert!(results.iter().all(|r| r.entry.scope == MemoryScope::Project));
    }
}
