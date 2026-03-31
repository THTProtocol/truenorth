//! Sequential execution strategy — ordered multi-step execution.
//!
//! Steps are executed one-at-a-time in dependency order. Each step receives
//! the outputs of all previous steps as context. Appropriate for tasks that
//! involve a clearly ordered sequence of actions (research → analyze → write).

use std::sync::Arc;

use async_trait::async_trait;
use tracing::instrument;

use truenorth_core::traits::execution::{
    ExecutionContext, ExecutionControl, ExecutionError, ExecutionStrategy, StepResult,
};
use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::plan::{Plan, PlanStep};
use truenorth_core::types::task::{ExecutionMode, Task};

use crate::agent_loop::planner::TaskPlanner;
use crate::agent_loop::step_runner::StepRunner;

/// Sequential execution strategy.
///
/// Steps execute in dependency order, each receiving the accumulated
/// context from all previous steps. Suitable for workflows where
/// each step builds on the result of the last.
#[derive(Debug)]
pub struct SequentialExecutionStrategy {
    step_runner: StepRunner,
    planner: TaskPlanner,
}

impl SequentialExecutionStrategy {
    /// Creates a new `SequentialExecutionStrategy`.
    pub fn new(
        llm_router: Option<Arc<dyn LlmRouter>>,
        event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    ) -> Self {
        Self {
            step_runner: StepRunner::new(llm_router, event_emitter),
            planner: TaskPlanner::new(),
        }
    }
}

#[async_trait]
impl ExecutionStrategy for SequentialExecutionStrategy {
    fn name(&self) -> &str {
        "sequential"
    }

    fn is_applicable(&self, task: &Task) -> bool {
        matches!(task.execution_mode, ExecutionMode::Sequential)
    }

    /// Creates an ordered multi-step plan by decomposing the task.
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    async fn plan(&self, task: &Task) -> Result<Plan, ExecutionError> {
        self.planner.create_multi_step_plan(task)
    }

    /// Executes a single step, with all previous results in context.
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        self.step_runner.run_step(step, context).await
    }

    /// All steps must be completed for the plan to be considered done.
    fn is_complete(&self, plan: &Plan, results: &[StepResult]) -> bool {
        results.len() >= plan.steps.len()
            && results.iter().all(|r| r.success)
    }

    fn control_signal(&self, _plan: &Plan, results: &[StepResult]) -> ExecutionControl {
        // If any step failed critically, halt
        if results.iter().any(|r| !r.success && r.deviation_detected) {
            return ExecutionControl::Halt {
                reason: "Critical deviation detected in sequential step".to_string(),
                save_state: true,
            };
        }
        ExecutionControl::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;
    use truenorth_core::types::task::TaskPriority;
    use truenorth_core::PlanStatus;
    use truenorth_core::types::plan::PlanStepStatus;

    fn make_sequential_task() -> Task {
        Task {
            id: Uuid::new_v4(),
            parent_id: None,
            title: "Multi-step task".to_string(),
            description: "First do this. Then do that. Finally finish.".to_string(),
            constraints: vec![],
            context_requirements: vec![],
            execution_mode: ExecutionMode::Sequential,
            created_at: Utc::now(),
            deadline: None,
            priority: TaskPriority::Normal,
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn plan_creates_multiple_steps() {
        let strategy = SequentialExecutionStrategy::new(None, None);
        let task = make_sequential_task();
        let plan = strategy.plan(&task).await.unwrap();
        assert!(plan.steps.len() > 0);
    }

    #[test]
    fn is_complete_when_all_steps_done() {
        let strategy = SequentialExecutionStrategy::new(None, None);
        let plan = Plan {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            created_at: Utc::now(),
            approved_at: None,
            steps: vec![],
            estimated_tokens: 0,
            estimated_duration_seconds: 0,
            mermaid_diagram: String::new(),
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        };
        let results = vec![];
        assert!(strategy.is_complete(&plan, &results));
    }
}
