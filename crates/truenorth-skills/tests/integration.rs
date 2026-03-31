//! Integration tests for truenorth-skills.
//!
//! Tests cover: SkillMarkdownParser (frontmatter + sections), SkillValidator
//! (well-formed vs malformed skills), TriggerMatcher (pattern matching),
//! and SkillRegistry (register, lookup, list, find_by_trigger).

use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;

use truenorth_skills::{
    ParsedSkill, SkillBody, SkillMarkdownParser, SkillRegistry, SkillValidator, TriggerMatcher,
};
use truenorth_core::types::skill::{SkillFrontmatter, SkillLoadLevel, SkillMetadata};

// ─── Helpers ──────────────────────────────────────────────────────────────────

const VALID_SKILL_MD: &str = r#"---
name: Research Assistant
version: 1.2.0
description: Guides the agent through deep multi-source research workflows.
triggers:
  - research
  - investigate
  - deep dive into
tools_required:
  - search_web
  - fetch_url
permission_level: low
author: TrueNorth Team
sandboxed: false
tags:
  - research
  - information
---

## When to Use

Use this skill when the user asks for comprehensive research on a topic,
wants to investigate a question in depth, or needs a structured summary
of information from multiple sources.

## Workflow

1. Clarify the research question.
2. Search for primary sources using `search_web`.
3. Fetch and extract relevant content from top results.
4. Synthesize findings into a structured report.
5. Cite all sources at the end.

## Best Practices

- Prefer primary sources over aggregation sites.
- Always verify facts across at least two independent sources.
- Include publication dates for time-sensitive content.

## References

- https://example.com/research-methodology
- https://example.com/citation-guide
"#;

const VALID_SANDBOXED_SKILL_MD: &str = r#"---
name: Safe Community Skill
version: 2.0.0
description: A community skill with only safe code blocks.
triggers:
  - community task
tools_required: []
permission_level: medium
author: Community Member
sandboxed: true
tags:
  - community
---

## When to Use

Use this for community-contributed tasks.

## Workflow

Here is a YAML example:

```yaml
key: value
```

And a JSON example:

```json
{"foo": "bar"}
```
"#;

fn write_temp_skill(content: &str) -> NamedTempFile {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(content.as_bytes()).unwrap();
    tmp
}

fn parse_skill(content: &str) -> ParsedSkill {
    let tmp = write_temp_skill(content);
    SkillMarkdownParser::new().parse_skill_file(tmp.path()).unwrap()
}

fn make_skill_metadata(name: &str, triggers: &[&str]) -> SkillMetadata {
    SkillMetadata {
        name: name.to_string(),
        version: "1.0.0".to_string(),
        description: format!("Description of {}", name),
        triggers: triggers.iter().map(|s| s.to_string()).collect(),
        tags: vec![],
        is_active: true,
        loaded_at: SkillLoadLevel::Minimal,
    }
}

// ─── 1. SkillMarkdownParser ───────────────────────────────────────────────────

#[test]
fn parser_extracts_frontmatter_name_and_version() {
    let parser = SkillMarkdownParser::new();
    let fm = parser.parse_frontmatter(VALID_SKILL_MD, Path::new("test.md")).unwrap();
    assert_eq!(fm.name, "Research Assistant");
    assert_eq!(fm.version, "1.2.0");
}

#[test]
fn parser_extracts_frontmatter_description() {
    let parser = SkillMarkdownParser::new();
    let fm = parser.parse_frontmatter(VALID_SKILL_MD, Path::new("test.md")).unwrap();
    assert!(fm.description.contains("research"));
}

#[test]
fn parser_extracts_triggers_list() {
    let parser = SkillMarkdownParser::new();
    let fm = parser.parse_frontmatter(VALID_SKILL_MD, Path::new("test.md")).unwrap();
    assert_eq!(fm.triggers.len(), 3);
    assert!(fm.triggers.contains(&"research".to_string()));
    assert!(fm.triggers.contains(&"investigate".to_string()));
    assert!(fm.triggers.contains(&"deep dive into".to_string()));
}

#[test]
fn parser_extracts_tools_required() {
    let parser = SkillMarkdownParser::new();
    let fm = parser.parse_frontmatter(VALID_SKILL_MD, Path::new("test.md")).unwrap();
    assert_eq!(fm.tools_required.len(), 2);
    assert!(fm.tools_required.contains(&"search_web".to_string()));
    assert!(fm.tools_required.contains(&"fetch_url".to_string()));
}

#[test]
fn parser_extracts_permission_level_and_author() {
    let parser = SkillMarkdownParser::new();
    let fm = parser.parse_frontmatter(VALID_SKILL_MD, Path::new("test.md")).unwrap();
    assert_eq!(fm.permission_level, "low");
    assert_eq!(fm.author, "TrueNorth Team");
    assert!(!fm.sandboxed);
}

#[test]
fn parser_extracts_tags() {
    let parser = SkillMarkdownParser::new();
    let fm = parser.parse_frontmatter(VALID_SKILL_MD, Path::new("test.md")).unwrap();
    assert!(fm.tags.contains(&"research".to_string()));
    assert!(fm.tags.contains(&"information".to_string()));
}

#[test]
fn parser_extracts_body_sections() {
    let parser = SkillMarkdownParser::new();
    let body = parser.parse_body(VALID_SKILL_MD);

    assert!(!body.raw.is_empty(), "Body raw should not be empty");
    assert!(body.when_to_use.contains("research on a topic"), "when_to_use section should be extracted");
    assert!(body.workflow.contains("Clarify the research question"), "workflow section should be extracted");
    assert!(body.best_practices.contains("primary sources"), "best_practices section should be extracted");
    assert!(body.references.contains("example.com"), "references section should be extracted");
}

#[test]
fn parser_parse_skill_file_returns_parsed_skill() {
    let tmp = write_temp_skill(VALID_SKILL_MD);
    let parser = SkillMarkdownParser::new();
    let parsed = parser.parse_skill_file(tmp.path()).unwrap();

    assert_eq!(parsed.frontmatter.name, "Research Assistant");
    assert_eq!(parsed.frontmatter.version, "1.2.0");
    assert!(parsed.body.workflow.contains("Clarify"));
    assert_eq!(parsed.file_path, tmp.path());
}

#[test]
fn parser_errors_on_missing_frontmatter() {
    use truenorth_skills::parser::ParseError;
    let parser = SkillMarkdownParser::new();
    let content = "# No frontmatter here\n\nJust a heading and some text.";
    let result = parser.parse_frontmatter(content, Path::new("bad.md"));
    assert!(
        matches!(result, Err(ParseError::NoFrontmatter { .. })),
        "Should return NoFrontmatter error"
    );
}

#[test]
fn parser_errors_on_io_failure() {
    use truenorth_skills::parser::ParseError;
    let parser = SkillMarkdownParser::new();
    let result = parser.parse_skill_file(Path::new("/nonexistent/path/skill.md"));
    assert!(matches!(result, Err(ParseError::Io { .. })), "Should return Io error");
}

#[test]
fn parser_body_missing_sections_are_empty_strings() {
    let minimal_md = r#"---
name: Minimal Skill
version: 1.0.0
description: Minimal.
triggers:
  - minimal
tools_required: []
permission_level: low
author: Test
sandboxed: false
tags: []
---

No sections here, just free-form text.
"#;
    let parser = SkillMarkdownParser::new();
    let body = parser.parse_body(minimal_md);

    assert!(body.when_to_use.is_empty(), "Missing when_to_use should be empty string");
    assert!(body.workflow.is_empty(), "Missing workflow should be empty string");
    assert!(body.best_practices.is_empty(), "Missing best_practices should be empty string");
    assert!(body.references.is_empty(), "Missing references should be empty string");
    // But raw should contain the free-form text
    assert!(body.raw.contains("No sections here"), "Raw body should contain all text");
}

// ─── 2. SkillValidator ────────────────────────────────────────────────────────

#[test]
fn validator_accepts_valid_skill() {
    let skill = parse_skill(VALID_SKILL_MD);
    let validator = SkillValidator::new();
    let result = validator.validate(&skill, &[]);
    assert!(result.is_ok(), "Valid skill should pass validation: {:?}", result);
}

#[test]
fn validator_rejects_empty_name() {
    let content = VALID_SKILL_MD.replace("name: Research Assistant", "name: \"\"");
    let skill = parse_skill(&content);
    let errors = SkillValidator::new().validate(&skill, &[]).unwrap_err();
    assert!(
        errors.iter().any(|e| e.field == "name"),
        "Should report error for empty name"
    );
}

#[test]
fn validator_rejects_invalid_version() {
    let content = VALID_SKILL_MD.replace("version: 1.2.0", "version: not-semver");
    let skill = parse_skill(&content);
    let errors = SkillValidator::new().validate(&skill, &[]).unwrap_err();
    assert!(
        errors.iter().any(|e| e.field == "version"),
        "Should report error for invalid version"
    );
}

#[test]
fn validator_rejects_invalid_permission_level() {
    let content = VALID_SKILL_MD.replace("permission_level: low", "permission_level: admin");
    let skill = parse_skill(&content);
    let errors = SkillValidator::new().validate(&skill, &[]).unwrap_err();
    assert!(
        errors.iter().any(|e| e.field == "permission_level"),
        "Should report error for invalid permission level"
    );
}

#[test]
fn validator_rejects_empty_triggers_list() {
    let content = VALID_SKILL_MD
        .replace("triggers:\n  - research\n  - investigate\n  - deep dive into", "triggers: []");
    let skill = parse_skill(&content);
    let errors = SkillValidator::new().validate(&skill, &[]).unwrap_err();
    assert!(
        errors.iter().any(|e| e.field == "triggers"),
        "Should report error for empty triggers"
    );
}

#[test]
fn validator_rejects_missing_required_tool() {
    let content = VALID_SKILL_MD.replace(
        "tools_required:\n  - search_web\n  - fetch_url",
        "tools_required:\n  - some_tool_not_in_registry",
    );
    let skill = parse_skill(&content);
    let available = vec!["search_web".to_string(), "fetch_url".to_string()];
    let errors = SkillValidator::new().validate(&skill, &available).unwrap_err();
    assert!(
        errors.iter().any(|e| e.field == "tools_required"),
        "Should report error for unregistered tool"
    );
}

#[test]
fn validator_accepts_all_permission_levels() {
    for level in &["low", "medium", "high"] {
        let content = VALID_SKILL_MD.replace("permission_level: low", &format!("permission_level: {}", level));
        let skill = parse_skill(&content);
        let result = SkillValidator::new().validate(&skill, &[]);
        assert!(result.is_ok(), "Permission level '{}' should be accepted", level);
    }
}

#[test]
fn validator_rejects_executable_code_blocks_in_sandboxed_skill() {
    let with_python = VALID_SANDBOXED_SKILL_MD.to_string() + "\n```python\nprint('hello')\n```\n";
    let skill = parse_skill(&with_python);
    let errors = SkillValidator::new().validate(&skill, &[]).unwrap_err();
    assert!(
        errors.iter().any(|e| e.field == "body"),
        "Sandboxed skill with Python code block should fail validation"
    );
}

#[test]
fn validator_allows_safe_code_blocks_in_sandboxed_skill() {
    let skill = parse_skill(VALID_SANDBOXED_SKILL_MD);
    let result = SkillValidator::new().validate(&skill, &[]);
    assert!(
        result.is_ok(),
        "Sandboxed skill with only yaml/json code blocks should pass: {:?}", result
    );
}

#[test]
fn validator_collects_all_errors_not_just_first() {
    let content = r#"---
name: ""
version: bad-version
description: ""
triggers: []
tools_required: []
permission_level: invalid_level
author: ""
sandboxed: false
tags: []
---
"#;
    let skill = parse_skill(content);
    let errors = SkillValidator::new().validate(&skill, &[]).unwrap_err();
    // Should get errors for: name, version, description, triggers, permission_level, author
    assert!(errors.len() >= 4, "Should collect all errors, got only {}: {:?}", errors.len(), errors);
}

// ─── 3. TriggerMatcher ───────────────────────────────────────────────────────

#[test]
fn trigger_matcher_basic_substring_match() {
    let matcher = TriggerMatcher::new();
    let skills = vec![make_skill_metadata("Research", &["research", "investigate"])];

    let matches = matcher.match_triggers("I want to research AI", &skills);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].skill_name, "Research");
    assert!(matches[0].matched_triggers.contains(&"research".to_string()));
}

#[test]
fn trigger_matcher_case_insensitive() {
    let matcher = TriggerMatcher::new();
    let skills = vec![make_skill_metadata("Coding", &["write code", "implement"])];

    let matches = matcher.match_triggers("WRITE CODE for me please", &skills);
    assert!(!matches.is_empty(), "Matching should be case-insensitive");
    assert_eq!(matches[0].skill_name, "Coding");
}

#[test]
fn trigger_matcher_no_match_returns_empty() {
    let matcher = TriggerMatcher::new();
    let skills = vec![make_skill_metadata("Research", &["research"])];

    let matches = matcher.match_triggers("write me a haiku about autumn leaves", &skills);
    assert!(matches.is_empty(), "Should return empty when no triggers match");
}

#[test]
fn trigger_matcher_multiple_skills_all_that_match() {
    let matcher = TriggerMatcher::new();
    let skills = vec![
        make_skill_metadata("Research", &["research"]),
        make_skill_metadata("Coding", &["code", "implement"]),
        make_skill_metadata("Writing", &["write", "draft"]),
    ];

    let matches = matcher.match_triggers("I need to research and code a solution", &skills);
    // Should match "Research" (research) and "Coding" (code)
    let matched_names: Vec<&str> = matches.iter().map(|m| m.skill_name.as_str()).collect();
    assert!(matched_names.contains(&"Research"), "Research should match");
    assert!(matched_names.contains(&"Coding"), "Coding should match");
}

#[test]
fn trigger_matcher_glob_wildcard_expansion() {
    let matcher = TriggerMatcher::new();
    let skills = vec![make_skill_metadata("Research", &["research*"])];

    let matches = matcher.match_triggers("I am researching machine learning", &skills);
    assert!(!matches.is_empty(), "Glob pattern 'research*' should match 'researching'");
}

#[test]
fn trigger_matcher_longer_trigger_scores_higher() {
    let matcher = TriggerMatcher::new();
    let skills = vec![
        make_skill_metadata("Generic", &["code"]),
        make_skill_metadata("Specific", &["code review process"]),
    ];

    let matches = matcher.match_triggers("let us do a code review process today", &skills);
    assert_eq!(matches.len(), 2, "Both skills should match");
    // Longer trigger → higher score → should appear first
    assert_eq!(matches[0].skill_name, "Specific", "More specific (longer) trigger should rank higher");
}

#[test]
fn trigger_matcher_results_are_sorted_descending_by_score() {
    let matcher = TriggerMatcher::new();
    let skills = vec![
        make_skill_metadata("Short", &["x"]),
        make_skill_metadata("Long", &["extra long trigger phrase"]),
    ];

    let matches = matcher.match_triggers("extra long trigger phrase and x", &skills);
    assert_eq!(matches.len(), 2);
    // Scores must be non-increasing
    for window in matches.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Results must be sorted descending: {} ({}) vs {} ({})",
            window[0].skill_name, window[0].score,
            window[1].skill_name, window[1].score
        );
    }
}

#[test]
fn trigger_matcher_matched_triggers_populated() {
    let matcher = TriggerMatcher::new();
    let skills = vec![make_skill_metadata("MultiTrigger", &["alpha", "beta", "gamma"])];

    let matches = matcher.match_triggers("alpha and gamma in the same sentence", &skills);
    assert_eq!(matches.len(), 1);
    let m = &matches[0];
    assert!(m.matched_triggers.contains(&"alpha".to_string()));
    assert!(m.matched_triggers.contains(&"gamma".to_string()));
    // "beta" should NOT be in matched_triggers since it wasn't in the prompt
    assert!(!m.matched_triggers.contains(&"beta".to_string()));
}

// ─── 4. SkillRegistry ────────────────────────────────────────────────────────

#[test]
fn registry_starts_empty() {
    let registry = SkillRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn registry_register_and_get() {
    let registry = SkillRegistry::new();
    let meta = make_skill_metadata("Alpha", &["alpha"]);
    registry.register(meta);

    assert_eq!(registry.len(), 1);
    let found = registry.get("Alpha");
    assert!(found.is_some(), "Registered skill should be retrievable");
    assert_eq!(found.unwrap().name, "Alpha");
}

#[test]
fn registry_get_nonexistent_returns_none() {
    let registry = SkillRegistry::new();
    assert!(registry.get("NonExistentSkill").is_none());
}

#[test]
fn registry_list_returns_all_skills_sorted_by_name() {
    let registry = SkillRegistry::new();
    registry.register(make_skill_metadata("Zebra", &["z"]));
    registry.register(make_skill_metadata("Apple", &["a"]));
    registry.register(make_skill_metadata("Mango", &["m"]));

    let list = registry.list();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "Apple");
    assert_eq!(list[1].name, "Mango");
    assert_eq!(list[2].name, "Zebra");
}

#[test]
fn registry_unregister_removes_skill() {
    let registry = SkillRegistry::new();
    registry.register(make_skill_metadata("Temp", &["temp"]));
    assert!(registry.get("Temp").is_some());

    let removed = registry.unregister("Temp");
    assert!(removed, "Unregister should return true for existing skill");
    assert!(registry.get("Temp").is_none());
    assert!(registry.is_empty());
}

#[test]
fn registry_unregister_nonexistent_returns_false() {
    let registry = SkillRegistry::new();
    let removed = registry.unregister("DoesNotExist");
    assert!(!removed, "Unregister of nonexistent skill should return false");
}

#[test]
fn registry_reregister_replaces_existing() {
    let registry = SkillRegistry::new();
    registry.register(make_skill_metadata("Skill", &["old trigger"]));

    let mut updated = make_skill_metadata("Skill", &["new trigger"]);
    updated.version = "2.0.0".to_string();
    registry.register(updated);

    assert_eq!(registry.len(), 1, "Should not create duplicates on re-register");
    let meta = registry.get("Skill").unwrap();
    assert_eq!(meta.version, "2.0.0", "Should have updated version");
    assert!(meta.triggers.contains(&"new trigger".to_string()), "Should have new triggers");
}

#[test]
fn registry_find_by_trigger_matches_registered_skills() {
    let registry = SkillRegistry::new();
    registry.register(make_skill_metadata("Research", &["research", "investigate"]));
    registry.register(make_skill_metadata("Coding", &["write code", "implement function"]));

    let matches = registry.find_by_trigger("I need to research and investigate this");
    assert!(!matches.is_empty(), "Should find matching skills via trigger");
    assert!(
        matches.iter().any(|m| m.skill_name == "Research"),
        "Research skill should be found"
    );
}

#[test]
fn registry_find_by_trigger_returns_empty_when_no_match() {
    let registry = SkillRegistry::new();
    registry.register(make_skill_metadata("Research", &["research"]));

    let matches = registry.find_by_trigger("write a poem about autumn");
    assert!(matches.is_empty(), "Non-matching prompt should return empty");
}

#[test]
fn registry_multiple_concurrent_registers() {
    use std::sync::Arc;
    use std::thread;

    let registry = Arc::new(SkillRegistry::new());

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let r = registry.clone();
            thread::spawn(move || {
                r.register(make_skill_metadata(&format!("Skill{}", i), &["trigger"]));
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(registry.len(), 10, "All 10 concurrent registrations should succeed");
}
