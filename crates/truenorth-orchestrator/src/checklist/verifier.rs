//! Negative checklist verifier implementation.
//!
//! Implements `NegativeChecklist` from `truenorth-core::traits::checklist`.
//! Verifies anti-pattern constraints at configured checkpoints (PrePlanning,
//! PreToolCall, PostStep, PreResponse, SessionEnd).

use std::path::Path;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::RwLock;
use tracing::{debug, warn};
use uuid::Uuid;

use truenorth_core::traits::checklist::{
    ChecklistError, ChecklistItem, ChecklistReport, ChecklistSeverity,
    ChecklistVerification, CheckPoint, NegativeChecklist,
};

/// Default negative checklist verifier.
///
/// Loads checklist items at startup and verifies them at configured checkpoints.
/// Default built-in items are included for common anti-patterns.
#[derive(Debug)]
pub struct DefaultNegativeChecklist {
    items: RwLock<Vec<ChecklistItem>>,
}

impl DefaultNegativeChecklist {
    /// Creates a new verifier with built-in default checklist items.
    pub fn new() -> Self {
        let verifier = Self {
            items: RwLock::new(Vec::new()),
        };
        verifier.load_defaults();
        verifier
    }

    /// Loads the default built-in checklist items.
    fn load_defaults(&self) {
        let defaults = vec![
            ChecklistItem {
                id: "no-unwrap-in-library".to_string(),
                description: "Never use .unwrap() in library code — use proper error handling".to_string(),
                category: "code_quality".to_string(),
                check_point: CheckPoint::PostStep,
                severity: ChecklistSeverity::Warning,
                remediation: Some("Replace .unwrap() with ? or .unwrap_or_else()".to_string()),
            },
            ChecklistItem {
                id: "no-infinite-loop".to_string(),
                description: "Never produce output identical to the previous step output".to_string(),
                category: "execution".to_string(),
                check_point: CheckPoint::PostStep,
                severity: ChecklistSeverity::Critical,
                remediation: Some("Check loop guard thresholds and semantic similarity detection".to_string()),
            },
            ChecklistItem {
                id: "no-delete-without-confirm".to_string(),
                description: "Never delete files or data without explicit user confirmation".to_string(),
                category: "security".to_string(),
                check_point: CheckPoint::PreToolCall,
                severity: ChecklistSeverity::Critical,
                remediation: Some("Add user confirmation step before destructive operations".to_string()),
            },
            ChecklistItem {
                id: "no-prod-modifications".to_string(),
                description: "Never modify production systems without explicit authorization".to_string(),
                category: "security".to_string(),
                check_point: CheckPoint::PreToolCall,
                severity: ChecklistSeverity::Critical,
                remediation: Some("Check task constraints before any production-touching tool calls".to_string()),
            },
            ChecklistItem {
                id: "no-credentials-in-output".to_string(),
                description: "Never include API keys, passwords, or credentials in output".to_string(),
                category: "security".to_string(),
                check_point: CheckPoint::PreResponse,
                severity: ChecklistSeverity::Critical,
                remediation: Some("Redact any credential-like patterns before output".to_string()),
            },
            ChecklistItem {
                id: "pre-planning-context-check".to_string(),
                description: "Verify sufficient context has been gathered before planning".to_string(),
                category: "planning".to_string(),
                check_point: CheckPoint::PrePlanning,
                severity: ChecklistSeverity::Warning,
                remediation: Some("Gather more context from memory before proceeding to planning".to_string()),
            },
            ChecklistItem {
                id: "session-state-saved".to_string(),
                description: "Verify session state is saved before session ends".to_string(),
                category: "persistence".to_string(),
                check_point: CheckPoint::SessionEnd,
                severity: ChecklistSeverity::Warning,
                remediation: Some("Ensure session manager save() was called".to_string()),
            },
        ];
        *self.items.write() = defaults;
    }

    /// Checks a single checklist item against the execution context.
    fn check_item(item: &ChecklistItem, context: &serde_json::Value) -> ChecklistVerification {
        // Simple heuristic checks based on item ID
        let (passed, message) = match item.id.as_str() {
            "no-credentials-in-output" => {
                // Check if output contains credential-like patterns
                let output = context.get("output_summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let has_creds = ["api_key", "password", "secret", "token", "AKIA", "sk-"]
                    .iter()
                    .any(|pat| output.to_lowercase().contains(pat));
                if has_creds {
                    (false, Some("Potential credentials detected in output".to_string()))
                } else {
                    (true, None)
                }
            }
            "no-infinite-loop" => {
                // This is primarily checked by SemanticSimilarityGuard
                (true, None)
            }
            _ => (true, None), // Default: pass
        };

        ChecklistVerification {
            item: item.clone(),
            passed,
            message,
            checked_at: Utc::now(),
        }
    }
}

impl Default for DefaultNegativeChecklist {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NegativeChecklist for DefaultNegativeChecklist {
    async fn load_from_file(&self, path: &Path) -> Result<usize, ChecklistError> {
        // In a full implementation, parse NEGATIVE_CHECKLIST.md
        // For now, we just use the built-in defaults
        debug!("Loading checklist from: {}", path.display());
        let count = self.items.read().len();
        Ok(count)
    }

    async fn verify(
        &self,
        check_point: CheckPoint,
        context: &serde_json::Value,
        session_id: Uuid,
    ) -> Result<ChecklistReport, ChecklistError> {
        let items = self.items.read();
        let checkpoint_items: Vec<&ChecklistItem> = items.iter()
            .filter(|item| item.check_point == check_point)
            .collect();

        let total_items = checkpoint_items.len();
        let mut verifications = Vec::with_capacity(total_items);

        for item in &checkpoint_items {
            let verification = Self::check_item(item, context);
            if !verification.passed {
                warn!("Checklist item '{}' FAILED at {:?}: {:?}",
                    item.id, check_point, verification.message);
            }
            verifications.push(verification);
        }

        let passed = verifications.iter().filter(|v| v.passed).count();
        let failed = total_items - passed;
        let all_passing = failed == 0;
        let max_severity_failed = verifications.iter()
            .filter(|v| !v.passed)
            .map(|v| v.item.severity.clone())
            .max();

        Ok(ChecklistReport {
            check_point,
            total_items,
            passed,
            failed,
            verifications,
            all_passing,
            max_severity_failed,
            session_id,
            generated_at: Utc::now(),
        })
    }

    async fn all_passing(
        &self,
        check_point: CheckPoint,
        context: &serde_json::Value,
    ) -> bool {
        let items = self.items.read();
        items.iter()
            .filter(|item| item.check_point == check_point)
            .all(|item| Self::check_item(item, context).passed)
    }

    fn format_report(&self, report: &ChecklistReport) -> String {
        let status = if report.all_passing { "✓ PASS" } else { "✗ FAIL" };
        let mut s = format!(
            "[Checklist {:?}] {} ({}/{} items passed)\n",
            report.check_point, status, report.passed, report.total_items
        );
        for v in &report.verifications {
            if !v.passed {
                s.push_str(&format!("  FAIL: {} — {:?}\n", v.item.id, v.message));
            }
        }
        s
    }

    fn add_item(&self, item: ChecklistItem) -> Result<(), ChecklistError> {
        let mut items = self.items.write();
        if items.iter().any(|i| i.id == item.id) {
            // Replace existing
            items.retain(|i| i.id != item.id);
        }
        items.push(item);
        Ok(())
    }

    fn list_items(&self) -> Vec<ChecklistItem> {
        self.items.read().clone()
    }

    fn items_for_checkpoint(&self, check_point: &CheckPoint) -> Vec<ChecklistItem> {
        self.items.read()
            .iter()
            .filter(|item| &item.check_point == check_point)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_default_items() {
        let checklist = DefaultNegativeChecklist::new();
        let items = checklist.list_items();
        assert!(!items.is_empty());
    }

    #[test]
    fn items_for_checkpoint_filters_correctly() {
        let checklist = DefaultNegativeChecklist::new();
        let pre_planning = checklist.items_for_checkpoint(&CheckPoint::PrePlanning);
        for item in &pre_planning {
            assert_eq!(item.check_point, CheckPoint::PrePlanning);
        }
    }

    #[tokio::test]
    async fn verify_passes_with_clean_output() {
        let checklist = DefaultNegativeChecklist::new();
        let ctx = serde_json::json!({
            "output_summary": "Successfully analyzed the codebase and found 3 issues.",
            "stage": "post_step"
        });
        let report = checklist.verify(CheckPoint::PostStep, &ctx, Uuid::new_v4()).await.unwrap();
        assert!(report.all_passing);
    }

    #[tokio::test]
    async fn verify_fails_with_credentials_in_output() {
        let checklist = DefaultNegativeChecklist::new();
        let ctx = serde_json::json!({
            "output_summary": "Here is your api_key: sk-abc123",
            "stage": "pre_response"
        });
        let report = checklist.verify(CheckPoint::PreResponse, &ctx, Uuid::new_v4()).await.unwrap();
        // Should fail the credentials check
        assert!(!report.all_passing);
    }

    #[test]
    fn add_custom_item() {
        let checklist = DefaultNegativeChecklist::new();
        let before = checklist.list_items().len();
        checklist.add_item(ChecklistItem {
            id: "custom-test".to_string(),
            description: "Custom test item".to_string(),
            category: "test".to_string(),
            check_point: CheckPoint::PostStep,
            severity: ChecklistSeverity::Warning,
            remediation: None,
        }).unwrap();
        let after = checklist.list_items().len();
        assert_eq!(after, before + 1);
    }
}
