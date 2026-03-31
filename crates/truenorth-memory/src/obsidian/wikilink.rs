//! `WikilinkParser` — parse and resolve Obsidian `[[wikilinks]]`.
//!
//! Obsidian uses `[[page name]]` or `[[page name|display text]]` syntax for
//! internal links. This parser extracts all wikilinks from Markdown text,
//! attempts to resolve them to actual file paths in the vault, and builds a
//! directed link graph for cross-reference navigation.
//!
//! ## Link formats supported
//!
//! | Syntax | Target | Display |
//! |--------|--------|---------|
//! | `[[target]]` | `target` | `target` |
//! | `[[target\|display]]` | `target` | `display` |
//! | `[[uuid]]` | UUID string | UUID string |
//!
//! ## Link graph
//!
//! The link graph is a `HashMap<PathBuf, Vec<WikilinkTarget>>`. Each key is a
//! vault file; the values are the wikilinks it contains, resolved to file paths
//! where possible.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

/// A single resolved wikilink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikilinkTarget {
    /// The raw link text inside `[[ ]]`.
    pub raw: String,
    /// Optional display text (after `|` in `[[target|display]]`).
    pub display_text: Option<String>,
    /// Resolved file path in the vault, if the target could be found.
    pub resolved_path: Option<PathBuf>,
}

/// Entry in the vault link graph.
#[derive(Debug, Clone)]
pub struct LinkGraphEntry {
    /// The file that contains these wikilinks.
    pub source_file: PathBuf,
    /// All wikilinks found in `source_file`.
    pub outgoing_links: Vec<WikilinkTarget>,
}

/// Obsidian wikilink parser and link graph builder.
///
/// Parses `[[wikilink]]` syntax from Markdown and resolves targets to file
/// paths within the vault.
#[derive(Debug)]
pub struct WikilinkParser {
    /// Root directory of the vault.
    vault_root: PathBuf,
    /// Compiled regex for matching `[[...]]` patterns.
    pattern: Regex,
}

impl WikilinkParser {
    /// Create a new `WikilinkParser` for the given vault root.
    pub fn new(vault_root: PathBuf) -> Self {
        // Matches [[target]] or [[target|display text]].
        // Uses a non-greedy match to handle multiple wikilinks on one line.
        let pattern = Regex::new(r"\[\[([^\[\]]+)\]\]").expect("Invalid wikilink regex");
        Self { vault_root, pattern }
    }

    /// Extract all wikilinks from a Markdown string.
    ///
    /// Returns a list of `WikilinkTarget` structs for each `[[...]]` found.
    /// Targets that resolve to files in the vault have `resolved_path` set.
    pub fn extract_links(&self, markdown: &str) -> Vec<WikilinkTarget> {
        let mut links = Vec::new();
        for cap in self.pattern.captures_iter(markdown) {
            let inner = cap[1].trim().to_string();
            let (raw_target, display_text) = if let Some(pipe_pos) = inner.find('|') {
                let target = inner[..pipe_pos].trim().to_string();
                let display = inner[pipe_pos + 1..].trim().to_string();
                (target, Some(display))
            } else {
                (inner.clone(), None)
            };

            let resolved_path = self.resolve_target(&raw_target);
            links.push(WikilinkTarget {
                raw: raw_target,
                display_text,
                resolved_path,
            });
        }
        links
    }

    /// Resolve a wikilink target string to a file path within the vault.
    ///
    /// Resolution order:
    /// 1. Exact filename match (`target.md`).
    /// 2. Case-insensitive filename match.
    /// 3. UUID match — look for `<uuid>.md`.
    /// 4. Recursive search through vault subdirectories.
    ///
    /// Returns `None` if no matching file is found.
    pub fn resolve_target(&self, target: &str) -> Option<PathBuf> {
        // Try exact match first.
        let exact = self.vault_root.join(format!("{}.md", target));
        if exact.exists() {
            return Some(exact);
        }

        // Try as UUID.
        if let Ok(uuid) = uuid::Uuid::parse_str(target) {
            let uuid_path = self.vault_root.join(format!("{}.md", uuid));
            if uuid_path.exists() {
                return Some(uuid_path);
            }
        }

        // Recursive search for filename (case-insensitive on all platforms).
        self.find_in_vault(target)
    }

    /// Recursively search the vault for a file matching `target_name` (without extension).
    fn find_in_vault(&self, target_name: &str) -> Option<PathBuf> {
        let target_lower = target_name.to_lowercase();
        let target_with_ext = format!("{}.md", target_lower);

        let Ok(walker) = std::fs::read_dir(&self.vault_root) else {
            return None;
        };

        for entry in walker.flatten() {
            let path = entry.path();
            if path.is_file() {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if filename == target_with_ext {
                    return Some(path);
                }
            } else if path.is_dir() {
                // Recurse into subdirectory.
                if let Some(found) = self.find_in_subdir(&path, &target_with_ext) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Recursively search a subdirectory.
    fn find_in_subdir(&self, dir: &Path, target_filename: &str) -> Option<PathBuf> {
        let Ok(walker) = std::fs::read_dir(dir) else {
            return None;
        };
        for entry in walker.flatten() {
            let path = entry.path();
            if path.is_file() {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if filename == target_filename {
                    return Some(path);
                }
            } else if path.is_dir() {
                if let Some(found) = self.find_in_subdir(&path, target_filename) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Build a link graph from all Markdown files in the vault.
    ///
    /// Reads every `.md` file in the vault (recursively), extracts wikilinks,
    /// and returns a map from `source_file → [WikilinkTarget]`.
    pub fn build_link_graph(&self) -> HashMap<PathBuf, LinkGraphEntry> {
        let mut graph: HashMap<PathBuf, LinkGraphEntry> = HashMap::new();
        self.scan_dir_into_graph(&self.vault_root, &mut graph);
        graph
    }

    /// Recursively scan a directory and populate the link graph.
    fn scan_dir_into_graph(&self, dir: &Path, graph: &mut HashMap<PathBuf, LinkGraphEntry>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let links = self.extract_links(&content);
                    if !links.is_empty() {
                        graph.insert(
                            path.clone(),
                            LinkGraphEntry {
                                source_file: path,
                                outgoing_links: links,
                            },
                        );
                    }
                }
            } else if path.is_dir() {
                self.scan_dir_into_graph(&path, graph);
            }
        }
    }

    /// Find all files that link to `target_path` (incoming links).
    ///
    /// Scans the provided link graph and returns all source files whose outgoing
    /// links resolve to `target_path`.
    pub fn incoming_links<'a>(
        graph: &'a HashMap<PathBuf, LinkGraphEntry>,
        target_path: &PathBuf,
    ) -> Vec<&'a PathBuf> {
        graph
            .iter()
            .filter(|(_, entry)| {
                entry
                    .outgoing_links
                    .iter()
                    .any(|link| link.resolved_path.as_ref() == Some(target_path))
            })
            .map(|(source, _)| source)
            .collect()
    }

    /// Return all wikilinks in `markdown` as plain strings (targets only, no display text).
    pub fn extract_link_targets(&self, markdown: &str) -> Vec<String> {
        self.extract_links(markdown)
            .into_iter()
            .map(|l| l.raw)
            .collect()
    }

    /// Generate Obsidian wikilink syntax for a given target name.
    pub fn format_wikilink(target: &str) -> String {
        format!("[[{}]]", target)
    }

    /// Generate an aliased wikilink: `[[target|display]]`.
    pub fn format_aliased_wikilink(target: &str, display: &str) -> String {
        format!("[[{}|{}]]", target, display)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_parser(tmp: &TempDir) -> WikilinkParser {
        WikilinkParser::new(tmp.path().to_path_buf())
    }

    #[test]
    fn test_extract_simple_links() {
        let tmp = TempDir::new().unwrap();
        let parser = make_parser(&tmp);
        let md = "See [[Rust Basics]] and [[Async Programming|async]] for details.";
        let links = parser.extract_links(md);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].raw, "Rust Basics");
        assert!(links[0].display_text.is_none());
        assert_eq!(links[1].raw, "Async Programming");
        assert_eq!(links[1].display_text.as_deref(), Some("async"));
    }

    #[test]
    fn test_resolve_existing_file() {
        let tmp = TempDir::new().unwrap();
        let parser = make_parser(&tmp);
        std::fs::write(tmp.path().join("Rust Basics.md"), "# Rust Basics").unwrap();
        let resolved = parser.resolve_target("Rust Basics");
        assert!(resolved.is_some());
    }

    #[test]
    fn test_resolve_uuid_link() {
        let tmp = TempDir::new().unwrap();
        let id = uuid::Uuid::new_v4();
        std::fs::write(tmp.path().join(format!("{}.md", id)), "content").unwrap();
        let parser = make_parser(&tmp);
        let resolved = parser.resolve_target(&id.to_string());
        assert!(resolved.is_some());
    }

    #[test]
    fn test_no_links() {
        let tmp = TempDir::new().unwrap();
        let parser = make_parser(&tmp);
        let links = parser.extract_links("Just plain text with no links.");
        assert!(links.is_empty());
    }

    #[test]
    fn test_format_wikilink() {
        assert_eq!(WikilinkParser::format_wikilink("My Note"), "[[My Note]]");
        assert_eq!(
            WikilinkParser::format_aliased_wikilink("target", "display"),
            "[[target|display]]"
        );
    }
}
