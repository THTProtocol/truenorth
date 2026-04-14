//! Parallel execution strategy — concurrent independent sub-task execution.
//!
//! Independent steps are spawned as Tokio tasks and executed concurrently.
//! Results are collected via `JoinSet`. Steps with dependencies are executed
//! after their prerequisites complete. Appropriate for tasks where multiple
//! independent pieces of work can proceed simultaneously.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::task::JoinSet;
use tracing::instrument;
use uuid::Uuid;

use truenorth_core::traits::execution::{
    ExecutionContext, ExecutionControl, ExecutionError, ExecutionStrategy, StepResult,
};
use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::plan::{Plan, PlanStep, PlanStepStatus};
use truenorth_core::types::task::{ExecutionMode, Task};

use crate::agent_loop::planner::TaskPlanner;
use crate::agent_loop::step_runner::StepRunner;

/// Parallel execution strategy.
///
/// Independent steps are launched concurrently using `tokio::task::JoinSet`.
/// Steps with `depends_on` entries are executed after their dependencies
/// complete. This strategy is appropriate for fan-out research or
/// parallel data processing workflows.
#[derive(Debug)]
pub struct ParallelExecutionStrategy {
    step_runner: StepRunner,
    planner: TaskPlanner,
}

impl ParallelExecutionStrategy {
    /// Creates a new `ParallelExecutionStrategy`.
    pub fn new(
        llm_router: Option<Arc<dyn LlmRouter>>,
        event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    ) -> Self {
        Self {
            step_runner: StepRunner::new(llm_router, event_emitter),
            planner: TaskPlanner::new(),
        }
    }

    /// Returns which steps can run immediately given the set of completed step IDs.
#[allow(dead_code)]
    fn eligible_steps<'a>(
        steps: &'a [PlanStep],
        completed_ids: &std::collections::HashSet<Uuid>,
    ) -> Vec<&'a PlanStep> {
        steps.iter().filter(|s| {
            matches!(s.status, PlanStepStatus::Pending) &&
            s.depends_on.iter().all(|dep| completed_ids.contains(dep))
        }).collect()
    }
}

#[async_trait]
impl ExecutionStrategy for ParallelExecutionStrategy {
    fn name(&self) -> &str {
        "parallel"
    }

    fn is_applicable(&self, task: &Task) -> bool {
        matches!(task.execution_mode, ExecutionMode::Parallel)
    }

    /// Creates a plan where all steps are independent (no dependencies).
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    async fn plan(&self, task: &Task) -> Result<Plan, ExecutionError> {
        let mut plan = self.planner.create_multi_step_plan(task)?;
        // Clear all dependencies to make steps truly parallel
        for step in plan.steps.iter_mut() {
            step.depends_on.clear();
        }
        Ok(plan)
    }

    /// Executes a step (called by the executor for each step individually).
    ///
    /// Note: The executor drives parallel execution by calling this for each
    /// step; the `run_parallel_steps` method is used for batch execution.
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        self.step_runner.run_step(step, context).await
    }

    /// Complete when all steps have results.
    fn is_complete(&self, plan: &Plan, results: &[StepResult]) -> bool {
        results.len() >= plan.steps.len()
    }

    fn control_signal(&self, _plan: &Plan, _results: &[StepResult]) -> ExecutionControl {
        ExecutionControl::Continue
    }
}

impl ParallelExecutionStrategy {
    /// Executes all independent steps concurrently.
    ///
    /// This is the core parallel execution logic. Steps are grouped by their
    /// dependency level and executed in waves. Within each wave, steps run
    /// concurrently via `JoinSet`.
    pub async fn run_parallel_steps(
        &self,
        plan: &Plan,
        base_context: &ExecutionContext,
    ) -> Vec<StepResult> {
        let mut results = Vec::new();
        let mut completed_ids = std::collections::HashSet::new();
        let mut remaining_steps: Vec<PlanStep> = plan.steps.clone();

        while !remaining_steps.is_empty() {
            // Find all steps that can run now
            let eligible: Vec<usize> = remaining_steps.iter().enumerate()
                .filter(|(_, s)| s.depends_on.iter().all(|d| completed_ids.contains(d)))
                .map(|(i, _)| i)
                .collect();

            if eligible.is_empty() {
                // Circular dependency or all remaining blocked
                break;
            }

            // Launch all eligible steps concurrently
            let mut join_set: JoinSet<(Uuid, Result<StepResult, ExecutionError>)> = JoinSet::new();
            let eligible_steps: Vec<PlanStep> = eligible.iter()
                .map(|&i| remaining_steps[i].clone())
                .collect();

            for step in eligible_steps.iter() {
                let step_clone = step.clone();
                let ctx_clone = ExecutionContext {
                    session_id: base_context.session_id,
                    task_id: base_context.task_id,
                    step_number: step.step_number,
                    approved_plan: base_context.approved_plan.clone(),
                    previous_results: results.clone(),
                };
                let runner = StepRunner::new(None, None); // Simplified for now
                let step_id = step.id;
                join_set.spawn(async move {
                    let result = runner.run_step(&step_clone, &ctx_clone).await;
                    (step_id, result)
                });
            }

            // Collect results
            while let Some(join_result) = join_set.join_next().await {
                if let Ok((step_id, Ok(result))) = join_result {
                    completed_ids.insert(step_id);
                    results.push(result);
                }
            }

            // Remove completed steps
            remaining_steps.retain(|s| !completed_ids.contains(&s.id));
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::task::TaskPriority;

    fn make_parallel_task() -> Task {
        Task {
            id: Uuid::new_v4(),
            parent_id: None,
            title: "Parallel task".to_string(),
            description: "Do A and also do B and also do C.".to_string(),
            constraints: vec![],
            context_requirements: vec![],
            execution_mode: ExecutionMode::Parallel,
            created_at: Utc::now(),
            deadline: None,
            priority: TaskPriority::Normal,
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn plan_has_no_dependencies() {
        let strategy = ParallelExecutionStrategy::new(None, None);
        let task = make_parallel_task();
        let plan = strategy.plan(&task).await.unwrap();
        for step in &plan.steps {
            assert!(step.depends_on.is_empty(), "Parallel steps should have no dependencies");
        }
    }
}
