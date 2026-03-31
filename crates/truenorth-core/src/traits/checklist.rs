/// NegativeChecklist trait — anti-pattern verification at runtime.
///
/// The negative checklist inverts positive verification: rather than checking
/// "did we do X correctly?", it checks "did we avoid doing Y incorrectly?"
/// Negative checks catch errors that positive verification misses — especially
/// the "verification laziness" failure mode where the agent confirms expected
/// behavior rather than verifying actual behavior.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

/// A single item in the negative checklist.
///
/// Negative checklist items describe things TrueNorth must NEVER do.
/// They are verified throughout task execution, not just at the end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    /// Unique identifier for this checklist item.
    pub id: String,
    /// The anti-pattern being checked (e.g., "Never use unwrap() in library code").
    pub description: String,
    /// The category this item belongs to (e.g., "code_quality", "security", "execution").
    pub category: String,
    /// When in the execution lifecycle to check this item.
    pub check_point: CheckPoint,
    /// Severity if this item is violated.
    pub severity: ChecklistSeverity,
    /// Optional remediation advice displayed when the check fails.
    pub remediation: Option<String>,
}

/// When during execution a checklist item should be verified.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckPoint {
    /// Verify before task planning begins.
    PrePlanning,
    /// Verify before each tool call.
    PreToolCall,
    /// Verify after each step completes.
    PostStep,
    /// Verify before the final response is emitted.
    PreResponse,
    /// Verify on session end (cleanup check).
    SessionEnd,
}

/// Severity of a checklist violation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChecklistSeverity {
    /// Log the violation but continue.
    Warning,
    /// Pause and alert the user.
    Error,
    /// Halt execution immediately.
    Critical,
}

/// The result of verifying a single checklist item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistVerification {
    /// The checklist item that was verified.
    pub item: ChecklistItem,
    /// Whether the check passed (true = no violation found).
    pub passed: bool,
    /// Optional message explaining why the check failed.
    pub message: Option<String>,
    /// When the check was performed.
    pub checked_at: DateTime<Utc>,
}

/// The complete verification report for a checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistReport {
    /// The checkpoint that was verified.
    pub check_point: CheckPoint,
    /// Total number of items checked.
    pub total_items: usize,
    /// Number of items that passed.
    pub passed: usize,
    /// Number of items that failed.
    pub failed: usize,
    /// Individual verification results.
    pub verifications: Vec<ChecklistVerification>,
    /// Whether all items passed.
    pub all_passing: bool,
    /// The highest severity among failed items (if any).
    pub max_severity_failed: Option<ChecklistSeverity>,
    /// The session this report belongs to.
    pub session_id: Uuid,
    /// When this report was generated.
    pub generated_at: DateTime<Utc>,
}

/// Errors from the negative checklist verifier.
#[derive(Debug, Error)]
pub enum ChecklistError {
    /// The checklist item was not found.
    #[error("Checklist item '{id}' not found")]
    ItemNotFound { id: String },

    /// The verification context is missing required data.
    #[error("Checklist verification context is missing required data: {message}")]
    MissingContext { message: String },

    /// Failed to load the checklist file.
    #[error("Failed to load checklist from {path}: {message}")]
    LoadFailed { path: std::path::PathBuf, message: String },

    /// I/O error while reading the checklist file.
    #[error("Checklist I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The negative checklist verifier: enforces the NEGATIVE_CHECKLIST.md at runtime.
///
/// Design rationale: verification laziness (Article 3 failure mode) is the
/// failure mode where the agent confirms expected behavior rather than verifying
/// actual behavior. The negative checklist inverts this: rather than checking
/// "did we do X correctly?", it checks "did we avoid doing Y incorrectly?"
/// Negative checks catch errors that positive verification misses.
///
/// The checklist items are loaded from NEGATIVE_CHECKLIST.md at startup.
/// The verifier runs items at the appropriate checkpoints throughout execution.
#[async_trait]
pub trait NegativeChecklist: Send + Sync + std::fmt::Debug {
    /// Loads checklist items from the NEGATIVE_CHECKLIST.md file.
    ///
    /// Called at startup. Returns the number of items loaded.
    async fn load_from_file(&self, path: &Path) -> Result<usize, ChecklistError>;

    /// Verifies all checklist items for a given checkpoint.
    ///
    /// The `context` parameter contains the current execution state as a JSON value.
    /// The verifier extracts what it needs for each item's check from this context.
    async fn verify(
        &self,
        check_point: CheckPoint,
        context: &serde_json::Value,
        session_id: Uuid,
    ) -> Result<ChecklistReport, ChecklistError>;

    /// Returns whether all items at a checkpoint are passing.
    ///
    /// Lightweight check for use in hot paths. Does not generate a full report.
    async fn all_passing(
        &self,
        check_point: CheckPoint,
        context: &serde_json::Value,
    ) -> bool;

    /// Returns a formatted human-readable summary report for display.
    ///
    /// Used in CLI output and the Visual Reasoning Layer's checklist panel.
    fn format_report(&self, report: &ChecklistReport) -> String;

    /// Adds a new checklist item at runtime.
    ///
    /// Used by the `truenorth config` CLI to add custom checks without
    /// modifying the NEGATIVE_CHECKLIST.md file.
    fn add_item(&self, item: ChecklistItem) -> Result<(), ChecklistError>;

    /// Returns all registered checklist items.
    fn list_items(&self) -> Vec<ChecklistItem>;

    /// Returns all items for a specific checkpoint.
    fn items_for_checkpoint(&self, check_point: &CheckPoint) -> Vec<ChecklistItem>;
}
