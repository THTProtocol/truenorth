//! Direct execution strategy — single-shot Reason → Act → Respond.
//!
//! The simplest execution mode: no multi-step planning. The task is
//! executed in a single LLM call with the full task description as context.
//! Appropriate for simple, single-output tasks (Q&A, summarization, etc.).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tracing::instrument;
use uuid::Uuid;

use truenorth_core::traits::execution::{
    ExecutionContext, ExecutionControl, ExecutionError, ExecutionStrategy, StepResult,
};
use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::plan::{Plan, PlanStatus, PlanStep, PlanStepStatus};
use truenorth_core::types::task::{ExecutionMode, Task};

use crate::agent_loop::step_runner::StepRunner;

/// Direct execution strategy: single-shot, no planning.
///
/// Applicable to tasks with `ExecutionMode::Direct` or any task
/// with a complexity score below the planning threshold.
#[derive(Debug)]
pub struct DirectExecutionStrategy {
    step_runner: StepRunner,
}

impl DirectExecutionStrategy {
    /// Creates a new `DirectExecutionStrategy`.
    pub fn new(
        llm_router: Option<Arc<dyn LlmRouter>>,
        event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    ) -> Self {
        Self {
            step_runner: StepRunner::new(llm_router, event_emitter),
        }
    }
}

#[async_trait]
impl ExecutionStrategy for DirectExecutionStrategy {
    fn name(&self) -> &str {
        "direct"
    }

    fn is_applicable(&self, task: &Task) -> bool {
        matches!(task.execution_mode, ExecutionMode::Direct)
    }

    /// Creates a trivial single-step plan for direct execution.
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    async fn plan(&self, task: &Task) -> Result<Plan, ExecutionError> {
        let step = PlanStep {
            id: Uuid::new_v4(),
            step_number: 1,
            title: task.title.clone(),
            description: task.description.clone(),
            tools_expected: vec![],
            skills_expected: vec![],
            depends_on: vec![],
            estimated_tokens: 500,
            status: PlanStepStatus::Pending,
            started_at: None,
            completed_at: None,
            actual_output: None,
        };

        Ok(Plan {
            id: Uuid::new_v4(),
            task_id: task.id,
            created_at: Utc::now(),
            approved_at: Some(Utc::now()),
            steps: vec![step],
            estimated_tokens: 500,
            estimated_duration_seconds: 30,
            mermaid_diagram: format!(
                "graph LR\n    A([Start]) --> B[{}]\n    B --> C([Complete])",
                task.title.chars().take(40).collect::<String>()
            ),
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        })
    }

    /// Executes the single step using the step runner.
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        self.step_runner.run_step(step, context).await
    }

    fn is_complete(&self, _plan: &Plan, results: &[StepResult]) -> bool {
        !results.is_empty() && results.iter().all(|r| r.success)
    }

    fn control_signal(&self, _plan: &Plan, _results: &[StepResult]) -> ExecutionControl {
        ExecutionControl::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::task::{TaskPriority};

    fn make_direct_task() -> Task {
        Task {
            id: Uuid::new_v4(),
            parent_id: None,
            title: "Direct task".to_string(),
            description: "Do something simple.".to_string(),
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
    async fn plan_creates_single_step() {
        let strategy = DirectExecutionStrategy::new(None, None);
        let task = make_direct_task();
        let plan = strategy.plan(&task).await.unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].step_number, 1);
    }

    #[test]
    fn is_applicable_for_direct_mode() {
        let strategy = DirectExecutionStrategy::new(None, None);
        let task = make_direct_task();
        assert!(strategy.is_applicable(&task));
    }

    #[test]
    fn not_applicable_for_sequential_mode() {
        let strategy = DirectExecutionStrategy::new(None, None);
        let mut task = make_direct_task();
        task.execution_mode = ExecutionMode::Sequential;
        assert!(!strategy.is_applicable(&task));
    }
}
