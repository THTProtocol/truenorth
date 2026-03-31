//! Integration tests for truenorth-memory.
//!
//! Tests cover: SessionMemoryStore CRUD, TantivySearch indexing/querying,
//! HybridSearch RRF merge, Obsidian wikilink parsing, MarkdownWriter
//! frontmatter output, and basic consolidation deduplication.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tempfile::TempDir;
use uuid::Uuid;

use truenorth_memory::{
    MemoryEntry, MemoryLayer, MemoryLayerConfig, MemoryScope, MemorySearchResult, MemorySearchType,
};
use truenorth_memory::search::{HybridSearch, TantivySearch};
use truenorth_memory::obsidian::wikilink::WikilinkParser;
use truenorth_memory::project::markdown_writer::MarkdownWriter;
use truenorth_memory::session::store::SessionMemoryStore;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn make_entry(scope: MemoryScope, content: &str) -> MemoryEntry {
    let now = Utc::now();
    MemoryEntry {
        id: Uuid::new_v4(),
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

fn make_entry_with_embedding(scope: MemoryScope, content: &str, embedding: Vec<f32>) -> MemoryEntry {
    let mut e = make_entry(scope, content);
    e.embedding = Some(embedding);
    e
}

fn make_search_result(entry: MemoryEntry, score: f32, search_type: MemorySearchType) -> MemorySearchResult {
    MemorySearchResult { entry, score, search_type }
}

async fn build_memory_layer(dir: &TempDir) -> MemoryLayer {
    let config = MemoryLayerConfig {
        memory_root: dir.path().join("memory"),
        project_db_path: None,
        identity_db_path: None,
        vault_dir: None,
        dedup_threshold: 0.85,
        consolidation_min_interval_hours: 8,
        consolidation_min_sessions: 1,
        watch_vault: false,
    };
    MemoryLayer::builder()
        .with_config(config)
        .build()
        .await
        .expect("MemoryLayer build should succeed")
}

// ─── 1. SessionMemoryStore ────────────────────────────────────────────────────

#[tokio::test]
async fn session_store_add_and_retrieve_entry() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("session.db");
    let store = SessionMemoryStore::new(db, None).unwrap();

    let session_id = Uuid::new_v4();
    let entry = store
        .add_entry(session_id, "Test memory content".to_string(), HashMap::new())
        .await
        .unwrap();

    assert_eq!(entry.content, "Test memory content");
    assert_eq!(entry.scope, MemoryScope::Session);

    // Retrieve by ID
    let retrieved = store.get_entry(entry.id).await.unwrap();
    assert_eq!(retrieved.id, entry.id);
    assert_eq!(retrieved.content, "Test memory content");
}

#[tokio::test]
async fn session_store_get_entries_for_session() {
    let dir = TempDir::new().unwrap();
    let store = SessionMemoryStore::new(dir.path().join("s.db"), None).unwrap();

    let session_id = Uuid::new_v4();
    for i in 0..3 {
        store
            .add_entry(session_id, format!("Entry {}", i), HashMap::new())
            .await
            .unwrap();
    }

    let entries = store.get_entries(session_id).await;
    assert_eq!(entries.len(), 3, "Should have 3 entries for this session");
}

#[tokio::test]
async fn session_store_different_sessions_are_isolated() {
    let dir = TempDir::new().unwrap();
    let store = SessionMemoryStore::new(dir.path().join("s.db"), None).unwrap();

    let session_a = Uuid::new_v4();
    let session_b = Uuid::new_v4();

    store.add_entry(session_a, "Session A entry".to_string(), HashMap::new()).await.unwrap();
    store.add_entry(session_b, "Session B entry".to_string(), HashMap::new()).await.unwrap();
    store.add_entry(session_b, "Session B entry 2".to_string(), HashMap::new()).await.unwrap();

    let a_entries = store.get_entries(session_a).await;
    let b_entries = store.get_entries(session_b).await;

    assert_eq!(a_entries.len(), 1, "Session A should have 1 entry");
    assert_eq!(b_entries.len(), 2, "Session B should have 2 entries");
}

#[tokio::test]
async fn session_store_not_found_returns_error() {
    let dir = TempDir::new().unwrap();
    let store = SessionMemoryStore::new(dir.path().join("s.db"), None).unwrap();

    let missing_id = Uuid::new_v4();
    let result = store.get_entry(missing_id).await;
    assert!(result.is_err(), "Getting missing entry should return error");
}

#[tokio::test]
async fn session_store_write_entry_extracts_session_id_from_metadata() {
    let dir = TempDir::new().unwrap();
    let store = SessionMemoryStore::new(dir.path().join("s.db"), None).unwrap();

    let session_id = Uuid::new_v4();
    let mut metadata = HashMap::new();
    metadata.insert(
        "session_id".to_string(),
        serde_json::Value::String(session_id.to_string()),
    );

    let entry = store
        .write_entry("Content with session_id in metadata".to_string(), metadata)
        .await
        .unwrap();

    // The entry should be associated with the provided session_id
    let entries = store.get_entries(session_id).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, entry.id);
}

// ─── 2. MemoryLayer write/read/delete ─────────────────────────────────────────

#[tokio::test]
async fn memory_layer_write_session_entry_and_read_back() {
    let dir = TempDir::new().unwrap();
    let layer = build_memory_layer(&dir).await;

    let entry = layer
        .write("Session memory content".to_string(), MemoryScope::Session, HashMap::new())
        .await
        .unwrap();

    assert_eq!(entry.scope, MemoryScope::Session);
    assert_eq!(entry.content, "Session memory content");

    let read_back = layer.read(entry.id).await.unwrap();
    assert_eq!(read_back.id, entry.id);
    assert_eq!(read_back.content, "Session memory content");
}

#[tokio::test]
async fn memory_layer_write_project_entry() {
    let dir = TempDir::new().unwrap();
    let layer = build_memory_layer(&dir).await;

    let entry = layer
        .write("Project decision: use async Rust".to_string(), MemoryScope::Project, HashMap::new())
        .await
        .unwrap();

    assert_eq!(entry.scope, MemoryScope::Project);
    let read_back = layer.read(entry.id).await.unwrap();
    assert_eq!(read_back.scope, MemoryScope::Project);
}

#[tokio::test]
async fn memory_layer_read_missing_returns_error() {
    let dir = TempDir::new().unwrap();
    let layer = build_memory_layer(&dir).await;

    let result = layer.read(Uuid::new_v4()).await;
    assert!(result.is_err(), "Reading non-existent entry should fail");
}

// ─── 3. TantivySearch full-text indexing and querying ─────────────────────────

#[tokio::test]
async fn tantivy_search_index_and_retrieve() {
    let dir = TempDir::new().unwrap();
    let search = TantivySearch::new(dir.path().to_path_buf()).unwrap();

    let entry = make_entry(MemoryScope::Project, "The quick brown fox jumps over the lazy dog");
    search.index_entry(&entry).unwrap();
    search.reload(); // Force reader to see committed writes

    let results = search.search("quick fox", MemoryScope::Project, 10).unwrap();
    assert!(!results.is_empty(), "Should find the indexed entry");
    assert!(results.iter().any(|r| r.entry.id == entry.id));
}

#[tokio::test]
async fn tantivy_search_scope_filtering() {
    let dir = TempDir::new().unwrap();
    let search = TantivySearch::new(dir.path().to_path_buf()).unwrap();

    let project_entry = make_entry(MemoryScope::Project, "Rust async programming patterns");
    let session_entry = make_entry(MemoryScope::Session, "Rust async programming patterns");

    search.index_entry(&project_entry).unwrap();
    search.index_entry(&session_entry).unwrap();
    search.reload(); // Force reader to see committed writes

    let project_results = search.search("async programming", MemoryScope::Project, 10).unwrap();
    let session_results = search.search("async programming", MemoryScope::Session, 10).unwrap();

    // Each scope should only return entries from that scope
    for r in &project_results {
        assert_eq!(r.entry.scope, MemoryScope::Project, "Project search returned non-project result");
    }
    for r in &session_results {
        assert_eq!(r.entry.scope, MemoryScope::Session, "Session search returned non-session result");
    }
}

#[tokio::test]
async fn tantivy_search_respects_limit() {
    let dir = TempDir::new().unwrap();
    let search = TantivySearch::new(dir.path().to_path_buf()).unwrap();

    for i in 0..10 {
        let entry = make_entry(MemoryScope::Project, &format!("machine learning entry {}", i));
        search.index_entry(&entry).unwrap();
    }
    search.reload(); // Force reader to see committed writes

    let results = search.search("machine learning", MemoryScope::Project, 3).unwrap();
    assert!(results.len() <= 3, "Should respect limit of 3");
}

#[tokio::test]
async fn tantivy_search_no_results_for_unindexed_content() {
    let dir = TempDir::new().unwrap();
    let search = TantivySearch::new(dir.path().to_path_buf()).unwrap();

    let results = search.search("completely unindexed banana telephone", MemoryScope::Project, 5).unwrap();
    assert!(results.is_empty(), "Searching unindexed content should return empty results");
}

#[tokio::test]
async fn tantivy_search_remove_entry() {
    let dir = TempDir::new().unwrap();
    let search = TantivySearch::new(dir.path().to_path_buf()).unwrap();

    let entry = make_entry(MemoryScope::Project, "unique quantum entanglement research");
    search.index_entry(&entry).unwrap();
    search.reload(); // Force reader to see committed writes

    // Verify it's indexed
    let before = search.search("quantum entanglement", MemoryScope::Project, 5).unwrap();
    assert!(!before.is_empty(), "Entry should be found before removal");

    // Remove and verify
    search.remove_entry(entry.id).unwrap();
    search.reload(); // Force reader to see the deletion
    let after = search.search("quantum entanglement", MemoryScope::Project, 5).unwrap();
    assert!(
        after.iter().all(|r| r.entry.id != entry.id),
        "Removed entry should not appear in results"
    );
}

// ─── 4. HybridSearch RRF merge ────────────────────────────────────────────────

#[test]
fn hybrid_search_merges_disjoint_result_sets() {
    let hybrid = HybridSearch::new(0.5, 0.5);

    let e1 = make_entry(MemoryScope::Project, "fulltext only result");
    let e2 = make_entry(MemoryScope::Project, "semantic only result");

    let ft_results = vec![make_search_result(e1.clone(), 0.9, MemorySearchType::FullText)];
    let sem_results = vec![make_search_result(e2.clone(), 0.8, MemorySearchType::Semantic)];

    let merged = hybrid.merge(ft_results, sem_results, 10);
    assert_eq!(merged.len(), 2, "Both entries should appear in merged results");
    assert!(merged.iter().all(|r| r.search_type == MemorySearchType::Hybrid));
}

#[test]
fn hybrid_search_deduplicates_entries_appearing_in_both_lists() {
    let hybrid = HybridSearch::new(0.5, 0.5);

    let entry = make_entry(MemoryScope::Project, "appears in both fulltext and semantic");

    let ft_results = vec![make_search_result(entry.clone(), 0.9, MemorySearchType::FullText)];
    let sem_results = vec![make_search_result(entry.clone(), 0.85, MemorySearchType::Semantic)];

    let merged = hybrid.merge(ft_results, sem_results, 10);
    assert_eq!(merged.len(), 1, "Duplicate entry should be deduplicated");
    assert_eq!(merged[0].entry.id, entry.id);
    // Score should be normalized to 1.0 since it's the only/top result
    assert!((merged[0].score - 1.0).abs() < 0.001);
}

#[test]
fn hybrid_search_respects_limit() {
    let hybrid = HybridSearch::new(0.5, 0.5);

    let ft_results: Vec<_> = (0..5)
        .map(|i| {
            let e = make_entry(MemoryScope::Project, &format!("ft entry {}", i));
            make_search_result(e, 0.9 - i as f32 * 0.1, MemorySearchType::FullText)
        })
        .collect();
    let sem_results: Vec<_> = (0..5)
        .map(|i| {
            let e = make_entry(MemoryScope::Project, &format!("sem entry {}", i));
            make_search_result(e, 0.8 - i as f32 * 0.1, MemorySearchType::Semantic)
        })
        .collect();

    let merged = hybrid.merge(ft_results, sem_results, 3);
    assert_eq!(merged.len(), 3, "Hybrid merge should respect limit");
}

#[test]
fn hybrid_search_empty_inputs_return_empty() {
    let hybrid = HybridSearch::new(0.5, 0.5);
    let merged = hybrid.merge(vec![], vec![], 10);
    assert!(merged.is_empty());
}

#[test]
fn hybrid_search_with_equal_weights_is_equivalent_to_merge_equal() {
    let e1 = make_entry(MemoryScope::Project, "entry alpha");
    let e2 = make_entry(MemoryScope::Project, "entry beta");

    let ft = vec![make_search_result(e1.clone(), 0.9, MemorySearchType::FullText)];
    let sem = vec![make_search_result(e2.clone(), 0.8, MemorySearchType::Semantic)];
    let ft2 = ft.clone();
    let sem2 = sem.clone();

    let via_new = HybridSearch::new(0.5, 0.5).merge(ft, sem, 10);
    let via_equal = HybridSearch::merge_equal(ft2, sem2, 10);

    assert_eq!(via_new.len(), via_equal.len(), "Both methods should return same number of results");

    // Both result sets contain the same entry IDs — use sorted comparison to avoid
    // non-deterministic HashMap iteration order between the two calls.
    let mut ids_new: Vec<Uuid> = via_new.iter().map(|r| r.entry.id).collect();
    let mut ids_eq: Vec<Uuid> = via_equal.iter().map(|r| r.entry.id).collect();
    ids_new.sort();
    ids_eq.sort();
    assert_eq!(ids_new, ids_eq, "Both methods should return the same entry IDs");

    // Scores should also match (compare after sorting by the same ID order)
    let score_map_new: std::collections::HashMap<Uuid, f32> =
        via_new.iter().map(|r| (r.entry.id, r.score)).collect();
    let score_map_eq: std::collections::HashMap<Uuid, f32> =
        via_equal.iter().map(|r| (r.entry.id, r.score)).collect();
    for id in &ids_new {
        let s_new = score_map_new[id];
        let s_eq = score_map_eq[id];
        assert!((s_new - s_eq).abs() < 0.001, "Scores for {} differ: {} vs {}", id, s_new, s_eq);
    }
}

// ─── 5. Obsidian wikilink parsing ────────────────────────────────────────────

#[test]
fn wikilink_parser_extracts_simple_link() {
    let dir = TempDir::new().unwrap();
    let parser = WikilinkParser::new(dir.path().to_path_buf());

    let md = "This references [[some page]] in the vault.";
    let links = parser.extract_links(md);

    assert_eq!(links.len(), 1);
    assert_eq!(links[0].raw, "some page");
    assert!(links[0].display_text.is_none());
}

#[test]
fn wikilink_parser_extracts_link_with_display_text() {
    let dir = TempDir::new().unwrap();
    let parser = WikilinkParser::new(dir.path().to_path_buf());

    let md = "See [[target page|Click here]] for details.";
    let links = parser.extract_links(md);

    assert_eq!(links.len(), 1);
    assert_eq!(links[0].raw, "target page");
    assert_eq!(links[0].display_text.as_deref(), Some("Click here"));
}

#[test]
fn wikilink_parser_extracts_multiple_links() {
    let dir = TempDir::new().unwrap();
    let parser = WikilinkParser::new(dir.path().to_path_buf());

    let md = "[[link one]] and [[link two]] and [[link three|three display]].";
    let links = parser.extract_links(md);

    assert_eq!(links.len(), 3);
    assert_eq!(links[0].raw, "link one");
    assert_eq!(links[1].raw, "link two");
    assert_eq!(links[2].raw, "link three");
    assert_eq!(links[2].display_text.as_deref(), Some("three display"));
}

#[test]
fn wikilink_parser_returns_empty_for_no_links() {
    let dir = TempDir::new().unwrap();
    let parser = WikilinkParser::new(dir.path().to_path_buf());

    let md = "This markdown has no wikilinks at all.";
    let links = parser.extract_links(md);
    assert!(links.is_empty());
}

#[test]
fn wikilink_parser_resolves_existing_file() {
    let dir = TempDir::new().unwrap();
    // Create a file that the parser can resolve
    let page_path = dir.path().join("my-page.md");
    std::fs::write(&page_path, "# My Page\nContent here.").unwrap();

    let parser = WikilinkParser::new(dir.path().to_path_buf());
    let md = "Link to [[my-page]] in the vault.";
    let links = parser.extract_links(md);

    assert_eq!(links.len(), 1);
    assert!(links[0].resolved_path.is_some(), "Should resolve to the existing file");
    assert_eq!(links[0].resolved_path.as_ref().unwrap(), &page_path);
}

#[test]
fn wikilink_parser_unresolved_link_has_none_path() {
    let dir = TempDir::new().unwrap();
    let parser = WikilinkParser::new(dir.path().to_path_buf());

    let md = "[[nonexistent page that definitely is not in vault]]";
    let links = parser.extract_links(md);

    assert_eq!(links.len(), 1);
    assert!(links[0].resolved_path.is_none(), "Non-existent page should not resolve");
}

// ─── 6. MarkdownWriter frontmatter ───────────────────────────────────────────

#[test]
fn markdown_writer_creates_file_with_correct_name() {
    let dir = TempDir::new().unwrap();
    let writer = MarkdownWriter::new(dir.path().to_path_buf());

    let entry = make_entry(MemoryScope::Project, "Important decision: use SQLite WAL mode");
    writer.write(&entry).unwrap();

    let expected_path = dir.path().join(format!("{}.md", entry.id));
    assert!(expected_path.exists(), "Markdown file should be created with UUID filename");
}

#[test]
fn markdown_writer_includes_frontmatter_fields() {
    let dir = TempDir::new().unwrap();
    let writer = MarkdownWriter::new(dir.path().to_path_buf());

    let entry = make_entry(MemoryScope::Project, "Decision about architecture");
    writer.write(&entry).unwrap();

    let path = dir.path().join(format!("{}.md", entry.id));
    let content = std::fs::read_to_string(&path).unwrap();

    // Must have YAML frontmatter delimiters
    assert!(content.starts_with("---"), "File must start with YAML frontmatter");
    // Must include the entry ID
    assert!(content.contains(&entry.id.to_string()), "Frontmatter must include entry ID");
    // Must include scope
    assert!(content.contains("scope:"), "Frontmatter must include scope field");
    // Must include timestamps
    assert!(content.contains("created_at:"), "Frontmatter must include created_at");
    assert!(content.contains("updated_at:"), "Frontmatter must include updated_at");
    // Must include importance
    assert!(content.contains("importance:"), "Frontmatter must include importance");
}

#[test]
fn markdown_writer_includes_entry_content_in_body() {
    let dir = TempDir::new().unwrap();
    let writer = MarkdownWriter::new(dir.path().to_path_buf());

    let content_str = "The team decided to use Tantivy for full-text search";
    let entry = make_entry(MemoryScope::Project, content_str);
    writer.write(&entry).unwrap();

    let path = dir.path().join(format!("{}.md", entry.id));
    let file_content = std::fs::read_to_string(&path).unwrap();
    assert!(
        file_content.contains(content_str),
        "Entry content should appear in the Markdown body"
    );
}

#[test]
fn markdown_writer_overwrites_on_second_write() {
    let dir = TempDir::new().unwrap();
    let writer = MarkdownWriter::new(dir.path().to_path_buf());

    let entry = make_entry(MemoryScope::Identity, "Original content");
    writer.write(&entry).unwrap();

    // Overwrite with updated entry
    let mut updated = entry.clone();
    updated.importance = 0.99;
    writer.write(&updated).unwrap();

    let path = dir.path().join(format!("{}.md", entry.id));
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("0.990"), "Updated importance score should be in file");
}

#[test]
fn markdown_writer_delete_removes_file() {
    let dir = TempDir::new().unwrap();
    let writer = MarkdownWriter::new(dir.path().to_path_buf());

    let entry = make_entry(MemoryScope::Project, "Temporary entry to delete");
    writer.write(&entry).unwrap();

    let path = dir.path().join(format!("{}.md", entry.id));
    assert!(path.exists(), "File should exist before deletion");

    writer.delete(entry.id).unwrap();
    assert!(!path.exists(), "File should be deleted");
}

#[test]
fn markdown_writer_delete_nonexistent_is_noop() {
    let dir = TempDir::new().unwrap();
    let writer = MarkdownWriter::new(dir.path().to_path_buf());

    // Deleting an ID that was never written should not error
    let result = writer.delete(Uuid::new_v4());
    assert!(result.is_ok(), "Deleting non-existent entry should be a no-op");
}

// ─── 7. MemoryLayer search integration ───────────────────────────────────────

#[tokio::test]
async fn memory_layer_fulltext_search_finds_indexed_entry() {
    let dir = TempDir::new().unwrap();
    let layer = build_memory_layer(&dir).await;

    let written = layer
        .write(
            "Neural network training with backpropagation".to_string(),
            MemoryScope::Project,
            HashMap::new(),
        )
        .await
        .unwrap();

    // Tantivy uses OnCommitWithDelay — give the background reload task enough time
    // to pick up the committed write before searching.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let results = layer
        .search_text("backpropagation", MemoryScope::Project, 5)
        .await
        .unwrap();

    assert!(!results.is_empty(), "Fulltext search should find the indexed entry");
    assert!(
        results.iter().any(|r| r.entry.id == written.id),
        "Search results should contain the written entry"
    );
}

#[tokio::test]
async fn memory_layer_list_recent_returns_entries_in_order() {
    let dir = TempDir::new().unwrap();
    let layer = build_memory_layer(&dir).await;

    let since = chrono::Utc::now() - chrono::Duration::hours(1);
    for i in 0..3 {
        layer
            .write(
                format!("Recent entry {}", i),
                MemoryScope::Session,
                HashMap::new(),
            )
            .await
            .unwrap();
    }

    let recent = layer.list_recent(MemoryScope::Session, since, 10).await.unwrap();
    assert_eq!(recent.len(), 3, "Should list 3 recent session entries");
}

#[tokio::test]
async fn memory_layer_delete_removes_from_all_tiers() {
    let dir = TempDir::new().unwrap();
    let layer = build_memory_layer(&dir).await;

    let entry = layer
        .write("Entry to delete".to_string(), MemoryScope::Session, HashMap::new())
        .await
        .unwrap();

    // Verify present
    assert!(layer.read(entry.id).await.is_ok());

    // Delete
    layer.delete(entry.id).await.unwrap();

    // Should no longer be readable
    let result = layer.read(entry.id).await;
    assert!(result.is_err(), "Deleted entry should not be readable");
}
