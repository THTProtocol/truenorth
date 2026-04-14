//! Task planner — complexity assessment and plan creation.
//!
//! The `TaskPlanner` assesses task complexity and constructs execution plans.
//! For simple tasks, it creates a direct single-step plan. For complex tasks,
//! it decomposes the task into multiple ordered steps.

use chrono::Utc;
use tracing::{debug, instrument};
use uuid::Uuid;

use truenorth_core::types::plan::{Plan, PlanStatus, PlanStep, PlanStepStatus};
use truenorth_core::types::task::{ComplexityScore, ExecutionMode, Task};
use truenorth_core::traits::execution::ExecutionError;

/// Complexity thresholds for strategy selection.
#[allow(dead_code)]
const SIMPLE_TASK_THRESHOLD: f32 = 0.35;
const COMPLEX_TASK_THRESHOLD: f32 = 0.75;

/// The task planner: assesses complexity and creates execution plans.
///
/// Uses a heuristic classifier that evaluates:
/// - Number of distinct verbs (implied sub-tasks)
/// - Presence of multi-step keywords ("then", "after", "finally")
/// - Estimated output length from description
/// - Tool requirement signals in the task description
#[derive(Debug, Default)]
pub struct TaskPlanner;

impl TaskPlanner {
    /// Creates a new `TaskPlanner`.
    pub fn new() -> Self {
        Self
    }

    /// Assesses the complexity of a task using heuristic analysis.
    ///
    /// Returns a `ComplexityScore` with an overall score from 0.0 to 1.0,
    /// where 0.0 is trivial and 1.0 is extremely complex.
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    pub fn assess_complexity(&self, task: &Task) -> ComplexityScore {
        let description = &task.description;
        let mut score: f32 = 0.0;
        #[allow(unused_assignments)]
        let mut estimated_tool_calls = 0usize;

        // Check for multi-step indicators
        let multi_step_keywords = ["then", "after", "next", "finally", "also", "additionally",
            "furthermore", "step", "steps", "first", "second", "third"];
        let multi_step_count = multi_step_keywords.iter()
            .filter(|kw| description.to_lowercase().contains(*kw))
            .count();
        score += (multi_step_count as f32 * 0.08).min(0.4);

        // Check for tool-requiring keywords
        let tool_keywords = ["search", "fetch", "read", "write", "execute", "run",
            "create", "delete", "update", "query", "call", "file", "web", "api"];
        let tool_count = tool_keywords.iter()
            .filter(|kw| description.to_lowercase().contains(*kw))
            .count();
        estimated_tool_calls = tool_count.min(10);
        score += (tool_count as f32 * 0.05).min(0.3);

        // Check description length (proxy for complexity)
        let word_count = description.split_whitespace().count();
        score += (word_count as f32 / 200.0).min(0.2);

        // Check execution mode override
        score += match &task.execution_mode {
            ExecutionMode::Direct => 0.0,
            ExecutionMode::Sequential => 0.3,
            ExecutionMode::Parallel => 0.5,
            ExecutionMode::Graph => 0.6,
            ExecutionMode::ReasonCriticSynthesis => 0.8,
            ExecutionMode::Persistent { .. } => 0.2,
        };

        let score = score.min(1.0);
        let recommend_rcs = score >= COMPLEX_TASK_THRESHOLD;

        ComplexityScore {
            score,
            estimated_tool_calls,
            estimated_tokens: word_count * 5 + estimated_tool_calls * 200,
            recommend_rcs,
            rationale: format!(
                "Score={:.2}: multi_step={}, tools={}, words={}",
                score, multi_step_count, tool_count, word_count
            ),
        }
    }

    /// Creates a single-step direct execution plan.
    ///
    /// Used when task complexity is below the planning threshold.
    pub fn create_direct_plan(&self, task: &Task) -> Plan {
        let plan_id = Uuid::new_v4();
        let step = PlanStep {
            id: Uuid::new_v4(),
            step_number: 1,
            title: task.title.clone(),
            description: task.description.clone(),
            tools_expected: vec![],
            skills_expected: vec![],
            depends_on: vec![],
            estimated_tokens: (task.description.split_whitespace().count() * 5 + 200) as u32,
            status: PlanStepStatus::Pending,
            started_at: None,
            completed_at: None,
            actual_output: None,
        };

        Plan {
            id: plan_id,
            task_id: task.id,
            created_at: Utc::now(),
            approved_at: None,
            steps: vec![step],
            estimated_tokens: 200,
            estimated_duration_seconds: 30,
            mermaid_diagram: format!(
                "graph LR\n    A[Start] --> B[{}]\n    B --> C[Complete]",
                task.title
            ),
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        }
    }

    /// Creates a multi-step plan by decomposing the task description.
    ///
    /// For complex tasks, this analyzes the task and splits it into
    /// logically ordered steps based on the execution mode.
    pub fn create_multi_step_plan(&self, task: &Task) -> Result<Plan, ExecutionError> {
        let plan_id = Uuid::new_v4();
        let mut steps = Vec::new();

        // Decompose based on execution mode
        let step_descriptions = match &task.execution_mode {
            ExecutionMode::Sequential | ExecutionMode::Graph => {
                self.decompose_sequential(task)
            }
            ExecutionMode::Parallel => self.decompose_parallel(task),
            ExecutionMode::ReasonCriticSynthesis => {
                vec![
                    ("Reason: Analyze problem and produce initial reasoning".to_string(),
                     "Apply systematic reasoning to understand the problem and produce a comprehensive analysis.".to_string()),
                    ("Critic: Identify flaws and missing considerations".to_string(),
                     "From a critical perspective, identify the weaknesses, gaps, and potential failure modes in the reasoning.".to_string()),
                    ("Synthesis: Produce final resolved response".to_string(),
                     "Synthesize the original reasoning and critic's objections into a final, comprehensive response.".to_string()),
                ]
            }
            _ => {
                vec![(task.title.clone(), task.description.clone())]
            }
        };

        if step_descriptions.is_empty() {
            return Err(ExecutionError::PlanningFailed {
                task_id: task.id,
                message: "Plan decomposition produced 0 steps".to_string(),
            });
        }

        let mut prev_step_id: Option<Uuid> = None;
        for (idx, (title, desc)) in step_descriptions.iter().enumerate() {
            let step_id = Uuid::new_v4();
            let depends_on = prev_step_id.map(|id| vec![id]).unwrap_or_default();

            let step = PlanStep {
                id: step_id,
                step_number: idx + 1,
                title: title.clone(),
                description: desc.clone(),
                tools_expected: vec![],
                skills_expected: vec![],
                depends_on,
                estimated_tokens: 500,
                status: PlanStepStatus::Pending,
                started_at: None,
                completed_at: None,
                actual_output: None,
            };
            steps.push(step);
            prev_step_id = Some(step_id);
        }

        let step_count = steps.len();
        let mermaid = self.generate_mermaid(&steps, &task.title);

        debug!("Created plan with {} steps for task {}", step_count, task.id);

        Ok(Plan {
            id: plan_id,
            task_id: task.id,
            created_at: Utc::now(),
            approved_at: None,
            steps,
            estimated_tokens: (step_count * 500) as u32,
            estimated_duration_seconds: (step_count * 30) as u64,
            mermaid_diagram: mermaid,
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        })
    }

    /// Decomposes a task description into sequential steps.
    fn decompose_sequential(&self, task: &Task) -> Vec<(String, String)> {
        // Simple heuristic: split on sentence-boundary step markers
        let desc = &task.description;
        let sentences: Vec<&str> = desc.split(". ").filter(|s| !s.is_empty()).collect();

        if sentences.len() <= 1 {
            return vec![(task.title.clone(), task.description.clone())];
        }

        sentences.iter().enumerate().map(|(i, s)| {
            (format!("Step {}: {}", i + 1, s.chars().take(60).collect::<String>()),
             s.to_string())
        }).collect()
    }

    /// Decomposes a task into independent parallel sub-tasks.
    fn decompose_parallel(&self, task: &Task) -> Vec<(String, String)> {
        // For parallel: treat each sentence as an independent sub-task
        self.decompose_sequential(task)
    }

    /// Generates a Mermaid flowchart for the given steps.
    fn generate_mermaid(&self, steps: &[PlanStep], title: &str) -> String {
        let mut lines = vec![
            "graph TD".to_string(),
            format!("    Start([{}]) --> S1", title.chars().take(30).collect::<String>()),
        ];

        for (i, step) in steps.iter().enumerate() {
            let node_id = format!("S{}", i + 1);
            let label = step.title.chars().take(40).collect::<String>();
            if i < steps.len() - 1 {
                let next_id = format!("S{}", i + 2);
                lines.push(format!("    {}[{}] --> {}", node_id, label, next_id));
            } else {
                lines.push(format!("    {}[{}] --> End", node_id, label));
            }
        }
        lines.push("    End([Complete])".to_string());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::task::{TaskPriority, ExecutionMode};

    fn make_task(desc: &str, mode: ExecutionMode) -> Task {
        Task {
            id: Uuid::new_v4(),
            parent_id: None,
            title: "Test".to_string(),
            description: desc.to_string(),
            constraints: vec![],
            context_requirements: vec![],
            execution_mode: mode,
            created_at: Utc::now(),
            deadline: None,
            priority: TaskPriority::Normal,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn direct_plan_has_one_step() {
        let planner = TaskPlanner::new();
        let task = make_task("Do something simple.", ExecutionMode::Direct);
        let plan = planner.create_direct_plan(&task);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.status, PlanStatus::Approved);
    }

    #[test]
    fn rcs_plan_has_three_steps() {
        let planner = TaskPlanner::new();
        let task = make_task("Complex multi-factor analysis.", ExecutionMode::ReasonCriticSynthesis);
        let plan = planner.create_multi_step_plan(&task).unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.steps[0].title.starts_with("Reason"));
        assert!(plan.steps[1].title.starts_with("Critic"));
        assert!(plan.steps[2].title.starts_with("Synthesis"));
    }

    #[test]
    fn complexity_direct_is_low() {
        let planner = TaskPlanner::new();
        let task = make_task("Say hello.", ExecutionMode::Direct);
        let score = planner.assess_complexity(&task);
        assert!(score.score < COMPLEX_TASK_THRESHOLD);
    }

    #[test]
    fn complexity_rcs_mode_is_high() {
        let planner = TaskPlanner::new();
        let task = make_task("Analyze then plan then execute.", ExecutionMode::ReasonCriticSynthesis);
        let score = planner.assess_complexity(&task);
        assert!(score.recommend_rcs);
    }
}
