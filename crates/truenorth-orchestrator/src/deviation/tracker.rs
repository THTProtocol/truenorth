//! Deviation tracker implementation.
//!
//! Implements `DeviationTracker` from `truenorth-core::traits::deviation`.
//! Compares step outputs against the approved plan using text similarity.
//! Flags deviations when similarity drops below a configurable threshold.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::RwLock;
use tracing::{debug, warn};
use uuid::Uuid;

use truenorth_core::traits::deviation::{
    Deviation, DeviationAction, DeviationAlert, DeviationError, DeviationTracker,
};
use truenorth_core::traits::execution::StepResult;
use truenorth_core::types::event::DeviationSeverity;
use truenorth_core::types::plan::Plan;

/// Default deviation threshold: flag deviation if similarity < 0.6.
const DEFAULT_DEVIATION_THRESHOLD: f32 = 0.6;

/// Per-task tracking state.
#[derive(Debug)]
struct TaskTrackingState {
    plan: Plan,
    deviations: Vec<Deviation>,
}

/// Default deviation tracker.
///
/// Uses a simple bag-of-words Jaccard similarity as a fallback when
/// embedding providers are not available. In production, this would
/// use cosine similarity over embedding vectors.
#[derive(Debug)]
pub struct DefaultDeviationTracker {
    tasks: RwLock<HashMap<Uuid, TaskTrackingState>>,
    threshold: f32,
}

impl DefaultDeviationTracker {
    /// Creates a new tracker with the default deviation threshold.
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            threshold: DEFAULT_DEVIATION_THRESHOLD,
        }
    }

    /// Creates a tracker with a custom deviation threshold.
    pub fn with_threshold(threshold: f32) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            threshold,
        }
    }

    /// Computes simple Jaccard similarity between two text strings.
    ///
    /// Used as a fallback when embedding providers are not available.
    /// Jaccard similarity = |A ∩ B| / |A ∪ B| where A and B are word sets.
    fn text_similarity(a: &str, b: &str) -> f32 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let words_a: std::collections::HashSet<&str> = a
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2)
            .collect();
        let words_b: std::collections::HashSet<&str> = b
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2)
            .collect();

        if words_a.is_empty() || words_b.is_empty() {
            return 0.5; // Benefit of the doubt for very short strings
        }

        let intersection = words_a.intersection(&words_b).count() as f32;
        let union = words_a.union(&words_b).count() as f32;

        if union == 0.0 {
            0.0
        } else {
            intersection / union
        }
    }

    /// Determines severity based on similarity score.
    fn severity_for_score(score: f32, threshold: f32) -> DeviationSeverity {
        if score < threshold * 0.5 {
            DeviationSeverity::Critical
        } else if score < threshold * 0.75 {
            DeviationSeverity::Significant
        } else {
            DeviationSeverity::Minor
        }
    }

    /// Determines the recommended action for a deviation.
    fn action_for_severity(severity: &DeviationSeverity) -> DeviationAction {
        match severity {
            DeviationSeverity::Minor => DeviationAction::ContinueWithLog,
            DeviationSeverity::Significant => DeviationAction::UpdatePlan {
                new_step_description: "Step deviated — plan updated to reflect actual action".to_string(),
            },
            DeviationSeverity::Critical => DeviationAction::HaltAndFlag {
                reason: "Critical deviation from approved plan".to_string(),
            },
        }
    }
}

impl Default for DefaultDeviationTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DeviationTracker for DefaultDeviationTracker {
    async fn register_plan(
        &self,
        task_id: Uuid,
        plan: Plan,
    ) -> Result<(), DeviationError> {
        debug!("Registering plan for task {}", task_id);
        self.tasks.write().insert(task_id, TaskTrackingState {
            plan,
            deviations: vec![],
        });
        Ok(())
    }

    async fn check_step(
        &self,
        task_id: Uuid,
        step_number: usize,
        result: &StepResult,
    ) -> Result<Option<DeviationAlert>, DeviationError> {
        let tasks = self.tasks.read();
        let state = tasks.get(&task_id)
            .ok_or(DeviationError::NoPlanRegistered { task_id })?;

        // Find the matching plan step
        let plan_step = state.plan.steps.iter()
            .find(|s| s.step_number == step_number);

        let plan_step = match plan_step {
            Some(s) => s,
            None => return Ok(None), // Extra step, no baseline to compare against
        };

        // Compute similarity between planned description and actual output summary
        let similarity = Self::text_similarity(
            &plan_step.description,
            &result.output_summary,
        );

        debug!(
            "Deviation check step {}: similarity={:.2} (threshold={})",
            step_number, similarity, self.threshold
        );

        if similarity >= self.threshold {
            return Ok(None); // No deviation
        }

        // Deviation detected
        let severity = Self::severity_for_score(similarity, self.threshold);
        let deviation = Deviation {
            id: Uuid::new_v4(),
            task_id,
            plan_step: plan_step.clone(),
            actual_action: result.output_summary.clone(),
            similarity_score: similarity,
            severity: severity.clone(),
            detected_at: Utc::now(),
            auto_resolved: matches!(severity, DeviationSeverity::Minor),
            resolution: if matches!(severity, DeviationSeverity::Minor) {
                Some("Auto-resolved: minor deviation logged".to_string())
            } else {
                None
            },
            resolved_at: if matches!(severity, DeviationSeverity::Minor) {
                Some(Utc::now())
            } else {
                None
            },
        };

        let recommended_action = Self::action_for_severity(&deviation.severity);

        warn!(
            "Deviation detected on task {} step {}: similarity={:.2}, severity={:?}",
            task_id, step_number, similarity, deviation.severity
        );

        let alert = DeviationAlert {
            deviation,
            recommended_action,
        };

        drop(tasks);

        // Store the deviation
        if let Some(state) = self.tasks.write().get_mut(&task_id) {
            state.deviations.push(alert.deviation.clone());
        }

        Ok(Some(alert))
    }

    async fn task_deviations(
        &self,
        task_id: Uuid,
    ) -> Result<Vec<Deviation>, DeviationError> {
        self.tasks.read()
            .get(&task_id)
            .map(|s| s.deviations.clone())
            .ok_or(DeviationError::NoPlanRegistered { task_id })
    }

    async fn resolve_deviation(
        &self,
        deviation_id: Uuid,
        resolution: String,
    ) -> Result<(), DeviationError> {
        let mut tasks = self.tasks.write();
        for state in tasks.values_mut() {
            if let Some(dev) = state.deviations.iter_mut().find(|d| d.id == deviation_id) {
                dev.resolution = Some(resolution.clone());
                dev.auto_resolved = true;
                dev.resolved_at = Some(Utc::now());
                return Ok(());
            }
        }
        Err(DeviationError::DeviationNotFound { deviation_id })
    }

    async fn update_plan(
        &self,
        task_id: Uuid,
        updated_plan: Plan,
    ) -> Result<(), DeviationError> {
        let mut tasks = self.tasks.write();
        let state = tasks.get_mut(&task_id)
            .ok_or(DeviationError::NoPlanRegistered { task_id })?;
        state.plan = updated_plan;
        Ok(())
    }

    fn deviation_threshold(&self) -> f32 {
        self.threshold
    }

    async fn has_unresolved_critical(
        &self,
        task_id: Uuid,
    ) -> Result<bool, DeviationError> {
        let tasks = self.tasks.read();
        let state = tasks.get(&task_id)
            .ok_or(DeviationError::NoPlanRegistered { task_id })?;

        let has_critical = state.deviations.iter().any(|d| {
            d.severity == DeviationSeverity::Critical && !d.auto_resolved
        });
        Ok(has_critical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::plan::{Plan, PlanStatus, PlanStep, PlanStepStatus};

    fn make_plan(task_id: Uuid, description: &str) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            task_id,
            created_at: Utc::now(),
            approved_at: None,
            steps: vec![PlanStep {
                id: Uuid::new_v4(),
                step_number: 1,
                title: "Test step".to_string(),
                description: description.to_string(),
                tools_expected: vec![],
                skills_expected: vec![],
                depends_on: vec![],
                estimated_tokens: 100,
                status: PlanStepStatus::Pending,
                started_at: None,
                completed_at: None,
                actual_output: None,
            }],
            estimated_tokens: 100,
            estimated_duration_seconds: 30,
            mermaid_diagram: String::new(),
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        }
    }

    fn make_result(step_number: usize, summary: &str) -> StepResult {
        StepResult {
            step_id: Uuid::new_v4(),
            step_number,
            success: true,
            output: serde_json::Value::Null,
            output_summary: summary.to_string(),
            tool_calls_made: vec![],
            tokens_used: 100,
            execution_ms: 100,
            deviation_detected: false,
        }
    }

    #[tokio::test]
    async fn no_deviation_for_matching_output() {
        let tracker = DefaultDeviationTracker::new();
        let task_id = Uuid::new_v4();
        let plan = make_plan(task_id, "Search for information about Rust programming");
        tracker.register_plan(task_id, plan).await.unwrap();

        let result = make_result(1, "Search for information about Rust programming");
        let alert = tracker.check_step(task_id, 1, &result).await.unwrap();
        assert!(alert.is_none());
    }

    #[tokio::test]
    async fn deviation_detected_for_different_output() {
        let tracker = DefaultDeviationTracker::new();
        let task_id = Uuid::new_v4();
        let plan = make_plan(task_id, "Search for information about Rust programming language");
        tracker.register_plan(task_id, plan).await.unwrap();

        // Completely unrelated output
        let result = make_result(1, "Downloaded a music playlist and organized by genre");
        let alert = tracker.check_step(task_id, 1, &result).await.unwrap();
        assert!(alert.is_some());
    }

    #[test]
    fn text_similarity_identical_strings() {
        let sim = DefaultDeviationTracker::text_similarity(
            "search for information about topic",
            "search for information about topic",
        );
        assert!((sim - 1.0).abs() < 0.01);
    }

    #[test]
    fn text_similarity_unrelated_strings() {
        let sim = DefaultDeviationTracker::text_similarity(
            "search for information",
            "delete all files now",
        );
        assert!(sim < 0.2);
    }

    #[tokio::test]
    async fn resolve_deviation() {
        let tracker = DefaultDeviationTracker::new();
        let task_id = Uuid::new_v4();
        let plan = make_plan(task_id, "Write a function to process data");
        tracker.register_plan(task_id, plan).await.unwrap();

        let result = make_result(1, "User was redirected to help page unexpectedly");
        let alert = tracker.check_step(task_id, 1, &result).await.unwrap();
        if let Some(alert) = alert {
            let dev_id = alert.deviation.id;
            tracker.resolve_deviation(dev_id, "Acknowledged and corrected".to_string())
                .await
                .unwrap();
            let deviations = tracker.task_deviations(task_id).await.unwrap();
            assert!(deviations.iter().any(|d| d.id == dev_id && d.auto_resolved));
        }
    }
}
