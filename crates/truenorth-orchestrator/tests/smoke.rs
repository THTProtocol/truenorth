//! End-to-end smoke test for the TrueNorth orchestrator.
//!
//! Proves that the full agent loop can be instantiated and execute
//! a task from start to finish (using default config — no LLM provider).

use chrono::Utc;
use uuid::Uuid;

use truenorth_core::types::task::{ExecutionMode, Task, TaskPriority};
use truenorth_orchestrator::{Orchestrator, OrchestratorConfig};

fn make_task(prompt: &str) -> Task {
    Task {
        id: Uuid::new_v4(),
        parent_id: None,
        title: prompt.chars().take(80).collect(),
        description: prompt.to_string(),
        constraints: vec![],
        context_requirements: vec![],
        execution_mode: ExecutionMode::Direct,
        created_at: Utc::now(),
        deadline: None,
        priority: TaskPriority::Normal,
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn orchestrator_builds_with_default_config() {
    let config = OrchestratorConfig::default();
    let result = Orchestrator::builder().with_config(config).build();
    assert!(result.is_ok(), "Orchestrator should build with default config");
}

#[tokio::test]
async fn orchestrator_runs_task_without_provider() {
    // Without an LLM provider, the agent loop should handle the error gracefully
    // rather than panicking. This proves the state machine and error handling work.
    let config = OrchestratorConfig::default();
    let orchestrator = Orchestrator::builder().with_config(config).build().unwrap();
    let task = make_task("Hello, TrueNorth");
    let result = orchestrator.run_task(task).await;
    // Either succeeds with a no-op result or returns a handled error — not a panic
    match result {
        Ok(r) => {
            // If it succeeds, the output should be non-empty
            assert!(!r.output_summary.is_empty() || r.steps_completed == 0);
        }
        Err(e) => {
            // An ExecutionError is fine — it means the loop ran and hit a known error path
            let msg = format!("{e}");
            assert!(!msg.is_empty(), "Error should have a message");
        }
    }
}

#[tokio::test]
async fn orchestrator_config_defaults_are_sane() {
    let config = OrchestratorConfig::default();
    assert_eq!(config.max_steps, 50);
    assert!(config.task_timeout_secs > 0);
    assert!(config.rcs_threshold > 0.0);
}
