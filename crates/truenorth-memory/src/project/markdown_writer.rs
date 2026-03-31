//! `MarkdownWriter` — Converts `MemoryEntry` to Obsidian-compatible Markdown files.
//!
//! Each memory entry is written as a `.md` file with YAML frontmatter containing
//! the structured metadata. The filename is derived from the entry's UUID.
//! Cross-references between entries are expressed as Obsidian `[[wikilinks]]`.
//!
//! ## File layout
//!
//! ```text
//! vault/project/
//! ├── <uuid>.md        ← one file per MemoryEntry
//! └── _index.md        ← regenerated index of all entries
//! ```
//!
//! ## YAML frontmatter structure
//!
//! ```yaml
//! ---
//! id: "550e8400-e29b-41d4-a716-446655440000"
//! scope: Project
//! created_at: "2026-03-31T22:00:00Z"
//! updated_at: "2026-03-31T22:00:00Z"
//! importance: 0.75
//! retrieval_count: 3
//! tags: [memory, project]
//! # ... arbitrary metadata keys
//! ---
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tracing::{debug, warn};
use uuid::Uuid;

use truenorth_core::types::memory::{MemoryEntry, MemoryScope};

/// Writes memory entries as Obsidian-compatible Markdown files.
#[derive(Debug, Clone)]
pub struct MarkdownWriter {
    /// Root directory where Markdown files are written.
    dir: PathBuf,
}

impl MarkdownWriter {
    /// Create a new `MarkdownWriter` targeting `dir`.
    ///
    /// The directory is created if it doesn't exist.
    pub fn new(dir: PathBuf) -> Self {
        if let Err(e) = fs::create_dir_all(&dir) {
            warn!("MarkdownWriter: cannot create dir {}: {e}", dir.display());
        }
        Self { dir }
    }

    /// Write a `MemoryEntry` to a Markdown file.
    ///
    /// The file is named `<uuid>.md` inside the writer's directory. If the file
    /// already exists, it is overwritten (equivalent to an update).
    ///
    /// # Errors
    ///
    /// Returns an error string if the file cannot be written.
    pub fn write(&self, entry: &MemoryEntry) -> Result<(), String> {
        let path = self.dir.join(format!("{}.md", entry.id));
        let content = self.render(entry);
        fs::write(&path, content)
            .map_err(|e| format!("Cannot write Markdown file {}: {e}", path.display()))?;
        debug!("Wrote Markdown for entry {} → {}", entry.id, path.display());
        Ok(())
    }

    /// Delete the Markdown file for an entry.
    ///
    /// Silently succeeds if the file doesn't exist.
    pub fn delete(&self, id: Uuid) -> Result<(), String> {
        let path = self.dir.join(format!("{}.md", id));
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("Cannot delete Markdown file {}: {e}", path.display()))?;
        }
        Ok(())
    }

    /// Render a `MemoryEntry` as a Markdown string with YAML frontmatter.
    ///
    /// The frontmatter includes all structured metadata fields. The body contains
    /// the entry content, with `[[wikilinks]]` inserted for any cross-references
    /// found in the metadata.
    pub fn render(&self, entry: &MemoryEntry) -> String {
        let mut fm = String::with_capacity(512);
        fm.push_str("---\n");
        fm.push_str(&format!("id: \"{}\"\n", entry.id));
        fm.push_str(&format!("scope: {:?}\n", entry.scope));
        fm.push_str(&format!("created_at: \"{}\"\n", entry.created_at.to_rfc3339()));
        fm.push_str(&format!("updated_at: \"{}\"\n", entry.updated_at.to_rfc3339()));
        fm.push_str(&format!("importance: {:.3}\n", entry.importance));
        fm.push_str(&format!("retrieval_count: {}\n", entry.retrieval_count));

        // Emit metadata as additional YAML keys.
        for (key, val) in &entry.metadata {
            // Skip reserved keys already emitted above.
            if matches!(key.as_str(), "id" | "scope" | "created_at" | "updated_at" | "importance" | "retrieval_count") {
                continue;
            }
            let yaml_val = self.value_to_yaml(val);
            fm.push_str(&format!("{key}: {yaml_val}\n"));
        }
        fm.push_str("---\n\n");

        // Body: the main content.
        let body = self.insert_wikilinks(&entry.content, &entry.metadata);
        fm.push_str(&body);
        fm.push('\n');
        fm
    }

    /// Convert a `serde_json::Value` to a YAML-compatible inline string.
    fn value_to_yaml(&self, val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::String(s) => {
                // Escape double quotes and wrap in double quotes.
                format!("\"{}\"", s.replace('"', "\\\""))
            }
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| self.value_to_yaml(v)).collect();
                format!("[{}]", items.join(", "))
            }
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Object(_) => {
                // Inline JSON for nested objects.
                serde_json::to_string(val).unwrap_or_else(|_| "{}".to_string())
            }
        }
    }

    /// Insert Obsidian `[[wikilinks]]` for cross-references stored in metadata.
    ///
    /// Looks for a `"references"` array in the metadata. Each element is treated
    /// as a UUID reference to another memory entry and linked with `[[uuid]]`.
    fn insert_wikilinks(
        &self,
        content: &str,
        metadata: &HashMap<String, serde_json::Value>,
    ) -> String {
        let mut body = content.to_string();

        if let Some(serde_json::Value::Array(refs)) = metadata.get("references") {
            let mut links = String::new();
            for r in refs {
                if let Some(ref_id) = r.as_str() {
                    links.push_str(&format!("\n- [[{}]]", ref_id));
                }
            }
            if !links.is_empty() {
                body.push_str("\n\n## References\n");
                body.push_str(&links);
            }
        }
        body
    }

    /// Parse a Markdown file back into a partial `MemoryEntry`.
    ///
    /// Reads the YAML frontmatter to reconstruct the entry's fields.
    /// Returns `None` if the file cannot be parsed.
    pub fn parse_file(&self, id: Uuid) -> Option<MemoryEntry> {
        let path = self.dir.join(format!("{}.md", id));
        let raw = fs::read_to_string(&path).ok()?;
        self.parse_markdown(&raw)
    }

    /// Parse a raw Markdown string into a `MemoryEntry`.
    ///
    /// Extracts YAML frontmatter and the body text. Returns `None` if the
    /// frontmatter cannot be parsed.
    pub fn parse_markdown(&self, raw: &str) -> Option<MemoryEntry> {
        // Split on the YAML frontmatter delimiters.
        let raw = raw.trim();
        if !raw.starts_with("---") {
            return None;
        }
        let rest = &raw[3..];
        let end = rest.find("\n---")?;
        let frontmatter_str = &rest[..end];
        let body_start = end + 4; // skip "\n---"
        let body = rest.get(body_start..)?.trim().to_string();

        // Parse frontmatter as YAML.
        let fm: serde_yaml::Value = serde_yaml::from_str(frontmatter_str).ok()?;
        let fm_map = fm.as_mapping()?;

        let id_str = fm_map.get("id")?.as_str()?;
        let id = Uuid::parse_str(id_str.trim_matches('"')).ok()?;

        let scope_str = fm_map.get("scope")?.as_str().unwrap_or("Project");
        let scope = match scope_str {
            "Session" => MemoryScope::Session,
            "Identity" => MemoryScope::Identity,
            _ => MemoryScope::Project,
        };

        let created_at = fm_map
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s.trim_matches('"'))
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .ok()
            })
            .unwrap_or_else(chrono::Utc::now);

        let updated_at = fm_map
            .get("updated_at")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s.trim_matches('"'))
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .ok()
            })
            .unwrap_or_else(chrono::Utc::now);

        let importance = fm_map
            .get("importance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5) as f32;

        let retrieval_count = fm_map
            .get("retrieval_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Collect remaining metadata keys.
        let mut metadata: HashMap<String, serde_json::Value> = HashMap::new();
        let skip = ["id", "scope", "created_at", "updated_at", "importance", "retrieval_count"];
        for (k, v) in fm_map {
            let key = k.as_str()?;
            if skip.contains(&key) {
                continue;
            }
            if let Ok(json_val) = serde_json::to_value(yaml_to_json(v)) {
                metadata.insert(key.to_string(), json_val);
            }
        }

        Some(MemoryEntry {
            id,
            scope,
            content: body,
            metadata,
            embedding: None,
            created_at,
            updated_at,
            importance,
            retrieval_count,
        })
    }

    /// Return the directory this writer targets.
    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    /// List all `.md` files in the vault directory.
    pub fn list_files(&self) -> Vec<PathBuf> {
        match fs::read_dir(&self.dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "md").unwrap_or(false))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Convert a `serde_yaml::Value` to a `serde_json::Value` (best-effort).
fn yaml_to_json(v: &serde_yaml::Value) -> serde_json::Value {
    match v {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::json!(f)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(m) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in m {
                if let Some(ks) = k.as_str() {
                    obj.insert(ks.to_string(), yaml_to_json(v));
                }
            }
            serde_json::Value::Object(obj)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(&t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_entry() -> MemoryEntry {
        let now = Utc::now();
        MemoryEntry {
            id: Uuid::new_v4(),
            scope: MemoryScope::Project,
            content: "Test content. More here.".to_string(),
            metadata: {
                let mut m = HashMap::new();
                m.insert("task_id".to_string(), serde_json::json!("abc-123"));
                m
            },
            embedding: None,
            created_at: now,
            updated_at: now,
            importance: 0.7,
            retrieval_count: 2,
        }
    }

    #[test]
    fn test_render_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let writer = MarkdownWriter::new(tmp.path().to_path_buf());
        let entry = make_entry();
        writer.write(&entry).unwrap();
        let parsed = writer.parse_file(entry.id).unwrap();
        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.scope, entry.scope);
        assert_eq!(parsed.content.trim(), entry.content.trim());
    }
}
