//! `truenorth skill` — skill management sub-commands.

use anyhow::Result;
use clap::Subcommand;
use std::path::Path;

use crate::output::{json, terminal};
use crate::OutputFormat;

/// Sub-commands for `truenorth skill`.
#[derive(Debug, Subcommand)]
pub enum SkillAction {
    /// Install a skill from the marketplace or a local path.
    Install {
        /// Skill name or `path:/path/to/SKILL.md`.
        source: String,
    },
    /// List all installed skills.
    List,
    /// Remove an installed skill.
    Remove {
        /// Name of the skill to remove.
        name: String,
    },
}

/// Execute a `skill` sub-command.
pub async fn execute(action: SkillAction, format: OutputFormat) -> Result<()> {
    match action {
        SkillAction::Install { source } => install(&source, format).await,
        SkillAction::List => list(format).await,
        SkillAction::Remove { name } => remove(&name, format).await,
    }
}

/// Scan a skills directory for SKILL.md or .md files and return (name, path) pairs.
fn scan_skills_dir(dir: &Path) -> Vec<(String, String)> {
    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                let name = path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                skills.push((name, path.display().to_string()));
            } else if path.is_dir() {
                // Check for SKILL.md inside subdirectory
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    let name = path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    skills.push((name, skill_md.display().to_string()));
                }
            }
        }
    }
    skills
}

/// Install a skill.
async fn install(source: &str, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth skill install");
            if source.starts_with("path:") || Path::new(source).exists() {
                let path = source.strip_prefix("path:").unwrap_or(source);
                if Path::new(path).exists() {
                    terminal::print_success(&format!("Found skill file: {path}"));
                    terminal::print_info("Skill installation from local files is not yet fully wired.");
                    terminal::print_info("The file was found and validated. Full registration coming soon.");
                } else {
                    terminal::print_error(&format!("File not found: {path}"));
                }
            } else {
                terminal::print_info(&format!("Marketplace skill: {source}"));
                terminal::print_info("Marketplace installation not yet available. Use a local path instead:");
                terminal::print_info(&format!("  truenorth skill install path:./skills/{source}.md"));
            }
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "skill install",
                "source": source,
                "status": "not_yet_available",
            }));
        }
    }
    Ok(())
}

/// List all installed skills.
async fn list(format: OutputFormat) -> Result<()> {
    // Scan default skill directories
    let dirs = vec![
        Path::new("skills/builtin"),
        Path::new("skills/community"),
        Path::new("skills/custom"),
        Path::new("skills"),
    ];

    let mut all_skills: Vec<(String, String)> = Vec::new();
    for dir in &dirs {
        if dir.exists() {
            all_skills.extend(scan_skills_dir(dir));
        }
    }

    // Deduplicate by name
    all_skills.sort_by(|a, b| a.0.cmp(&b.0));
    all_skills.dedup_by(|a, b| a.0 == b.0);

    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth skill list");
            if all_skills.is_empty() {
                terminal::print_info("No skills found.");
                terminal::print_info("");
                terminal::print_info("Skills are loaded from:");
                terminal::print_info("  skills/builtin/    — shipped with TrueNorth");
                terminal::print_info("  skills/community/  — community marketplace");
                terminal::print_info("  skills/custom/     — your custom skills");
                terminal::print_info("");
                terminal::print_info("Create a skill: place a .md file in skills/custom/");
            } else {
                terminal::print_info(&format!("{} skill(s) found:", all_skills.len()));
                terminal::print_info("");
                let rows: Vec<Vec<String>> = all_skills.iter().map(|(name, path)| {
                    vec![name.clone(), "installed".into(), path.clone()]
                }).collect();
                terminal::print_table(&["Name", "Status", "Path"], &rows);
            }
        }
        OutputFormat::Json => {
            let skills_json: Vec<serde_json::Value> = all_skills.iter().map(|(name, path)| {
                serde_json::json!({ "name": name, "path": path, "status": "installed" })
            }).collect();
            json::print_json(&serde_json::json!({
                "skills": skills_json,
                "count": all_skills.len(),
            }));
        }
    }
    Ok(())
}

/// Remove a skill by name.
async fn remove(name: &str, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            terminal::print_header("truenorth skill remove");
            terminal::print_info(&format!("Skill: {name}"));
            terminal::print_info("Skill removal is not yet implemented.");
            terminal::print_info("You can manually delete the skill file from the skills/ directory.");
        }
        OutputFormat::Json => {
            json::print_json(&serde_json::json!({
                "command": "skill remove",
                "name": name,
                "status": "not_yet_available",
            }));
        }
    }
    Ok(())
}
