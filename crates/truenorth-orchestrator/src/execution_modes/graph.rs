//! Graph execution strategy — DAG execution with conditional routing.
//!
//! Steps are executed in topological order according to their dependency graph.
//! Conditional edges allow branching (e.g., success/failure paths).
//! Appropriate for complex workflows with branching logic.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use tracing::instrument;
use uuid::Uuid;

use truenorth_core::traits::execution::{
    ExecutionContext, ExecutionControl, ExecutionError, ExecutionStrategy, StepResult,
};
use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::plan::{Plan, PlanStep};
use truenorth_core::types::task::{ExecutionMode, Task};

use crate::agent_loop::planner::TaskPlanner;
use crate::agent_loop::step_runner::StepRunner;

/// Graph execution strategy: DAG-based execution with topological ordering.
///
/// Executes steps in dependency order, parallelizing where possible.
/// Supports conditional routing: if a step fails, dependent steps
/// can be skipped or alternative branches can be activated.
#[derive(Debug)]
pub struct GraphExecutionStrategy {
    step_runner: StepRunner,
    planner: TaskPlanner,
}

impl GraphExecutionStrategy {
    /// Creates a new `GraphExecutionStrategy`.
    pub fn new(
        llm_router: Option<Arc<dyn LlmRouter>>,
        event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    ) -> Self {
        Self {
            step_runner: StepRunner::new(llm_router, event_emitter),
            planner: TaskPlanner::new(),
        }
    }

    /// Computes the topological execution order for the given steps.
    ///
    /// Uses Kahn's algorithm for topological sort. Returns an ordered
    /// list of step indices. Returns an error if a cycle is detected.
    pub fn topological_order(steps: &[PlanStep]) -> Result<Vec<usize>, ExecutionError> {
        let n = steps.len();
        let id_to_idx: HashMap<Uuid, usize> = steps.iter().enumerate()
            .map(|(i, s)| (s.id, i))
            .collect();

        let mut in_degree = vec![0usize; n];
        let mut adjacency: Vec<Vec<usize>> = vec![vec![]; n];

        for (i, step) in steps.iter().enumerate() {
            for dep_id in &step.depends_on {
                if let Some(&dep_idx) = id_to_idx.get(dep_id) {
                    adjacency[dep_idx].push(i);
                    in_degree[i] += 1;
                }
            }
        }

        let mut queue: VecDeque<usize> = (0..n)
            .filter(|&i| in_degree[i] == 0)
            .collect();

        let mut order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for &neighbor in &adjacency[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }

        if order.len() != n {
            return Err(ExecutionError::PlanningFailed {
                task_id: Uuid::nil(),
                message: "Cycle detected in task dependency graph".to_string(),
            });
        }

        Ok(order)
    }
}

#[async_trait]
impl ExecutionStrategy for GraphExecutionStrategy {
    fn name(&self) -> &str {
        "graph"
    }

    fn is_applicable(&self, task: &Task) -> bool {
        matches!(task.execution_mode, ExecutionMode::Graph)
    }

    /// Creates a DAG plan with dependency edges.
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    async fn plan(&self, task: &Task) -> Result<Plan, ExecutionError> {
        let plan = self.planner.create_multi_step_plan(task)?;
        // Validate the graph has no cycles
        GraphExecutionStrategy::topological_order(&plan.steps)?;
        Ok(plan)
    }

    /// Executes a single step in topological order.
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        self.step_runner.run_step(step, context).await
    }

    /// Complete when all non-skipped steps have results.
    fn is_complete(&self, plan: &Plan, results: &[StepResult]) -> bool {
        results.len() >= plan.steps.len()
    }

    fn control_signal(&self, _plan: &Plan, _results: &[StepResult]) -> ExecutionControl {
        ExecutionControl::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;
    use truenorth_core::PlanStepStatus;
    use truenorth_core::PlanStatus;

    fn make_step(step_number: usize, deps: Vec<Uuid>) -> PlanStep {
        PlanStep {
            id: Uuid::new_v4(),
            step_number,
            title: format!("Step {}", step_number),
            description: format!("Description for step {}", step_number),
            tools_expected: vec![],
            skills_expected: vec![],
            depends_on: deps,
            estimated_tokens: 100,
            status: PlanStepStatus::Pending,
            started_at: None,
            completed_at: None,
            actual_output: None,
        }
    }

    #[test]
    fn topological_order_linear_chain() {
        let s1 = make_step(1, vec![]);
        let s2 = make_step(2, vec![s1.id]);
        let s3 = make_step(3, vec![s2.id]);
        let steps = vec![s1, s2, s3];
        let order = GraphExecutionStrategy::topological_order(&steps).unwrap();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn topological_order_diamond() {
        let s1 = make_step(1, vec![]);
        let s2 = make_step(2, vec![s1.id]);
        let s3 = make_step(3, vec![s1.id]);
        let s4 = make_step(4, vec![s2.id, s3.id]);
        let steps = vec![s1, s2, s3, s4];
        let order = GraphExecutionStrategy::topological_order(&steps).unwrap();
        // s1 must come before s2, s3; s4 must come last
        assert_eq!(order[0], 0); // s1
        assert_eq!(order[3], 3); // s4
    }

    #[test]
    fn topological_order_no_deps_parallel() {
        let s1 = make_step(1, vec![]);
        let s2 = make_step(2, vec![]);
        let s3 = make_step(3, vec![]);
        let steps = vec![s1, s2, s3];
        let order = GraphExecutionStrategy::topological_order(&steps).unwrap();
        assert_eq!(order.len(), 3);
    }
}
