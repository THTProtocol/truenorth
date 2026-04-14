//! R/C/S execution strategy — Reason → Critic → Synthesis.
//!
//! The differentiating execution mode of TrueNorth. Three separate LLM calls,
//! each with FRESH CONTEXT (not appended), designed to overcome the verification
//! laziness failure mode where the agent confirms expected behavior rather than
//! verifying actual behavior.
//!
//! ## Protocol
//! 1. **Reason**: "Given this task, produce your best reasoning and plan."
//!    Fresh context: only the task description + gathered memory context.
//! 2. **Critic**: "Given this reasoning, find flaws, missing considerations, and failure modes."
//!    Fresh context: only the task + the Reason output (no history).
//! 3. **Synthesis**: "Given the original reasoning and the critic's objections, produce a final
//!    synthesized response that addresses all valid criticisms."
//!    Fresh context: task + Reason output + Critic output only.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use truenorth_core::traits::execution::{
    ExecutionContext, ExecutionControl, ExecutionError, ExecutionStrategy, StepResult,
};
use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::types::llm::{CompletionParameters, CompletionRequest, NormalizedMessage};
use truenorth_core::types::message::{ContentBlock, MessageRole};
use truenorth_core::types::plan::{Plan, PlanStatus, PlanStep, PlanStepStatus};
use truenorth_core::types::task::{ExecutionMode, Task};

/// The phase of the R/C/S loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcsPhase {
    /// The Reason phase: produce initial analysis.
    Reason,
    /// The Critic phase: challenge the reasoning.
    Critic,
    /// The Synthesis phase: merge reason and criticism.
    Synthesis,
}

/// Output from the Reason phase.
#[derive(Debug, Clone)]
pub struct ReasonOutput {
    /// The full text output from the Reason LLM call.
    pub content: String,
    /// Tokens consumed during the Reason phase.
    pub tokens_used: usize,
}

/// Output from the Critic phase.
#[derive(Debug, Clone)]
pub struct CriticOutput {
    /// The full text output from the Critic LLM call.
    pub content: String,
    /// Whether the critic approved the reasoning without major issues.
    pub approved: bool,
    /// Specific issues identified by the critic.
    pub issues: Vec<String>,
    /// Tokens consumed during the Critic phase.
    pub tokens_used: usize,
}

/// Output from the Synthesis phase.
#[derive(Debug, Clone)]
pub struct SynthesisOutput {
    /// The final synthesized response combining Reason and Critic insights.
    pub content: String,
    /// List of critic objections that were resolved in the synthesis.
    pub resolved_conflicts: Vec<String>,
    /// Tokens consumed during the Synthesis phase.
    pub tokens_used: usize,
}

/// R/C/S (Reason / Critic / Synthesis) execution strategy.
///
/// The highest-quality execution mode. Each phase uses a FRESH context window —
/// no conversation history is carried between phases. This prevents the
/// verification laziness failure mode and produces more robust outputs for
/// high-stakes decisions.
#[derive(Debug)]
pub struct RCSExecutionStrategy {
    llm_router: Option<Arc<dyn LlmRouter>>,
    event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
}

impl RCSExecutionStrategy {
    /// Creates a new `RCSExecutionStrategy`.
    pub fn new(
        llm_router: Option<Arc<dyn LlmRouter>>,
        event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    ) -> Self {
        Self { llm_router, event_emitter }
    }

    /// Emits a reasoning event if an emitter is configured.
    async fn emit(&self, session_id: Uuid, payload: ReasoningEventPayload) {
        if let Some(emitter) = &self.event_emitter {
            let event = ReasoningEvent::new(session_id, payload);
            if let Err(e) = emitter.emit(event).await {
                warn!("Failed to emit R/C/S reasoning event: {}", e);
            }
        }
    }

    /// Calls the LLM with a fresh context (no conversation history).
    ///
    /// This is the core differentiation: each phase gets a clean slate,
    /// preventing anchoring bias from earlier phases.
    async fn fresh_context_call(
        &self,
        system_prompt: &str,
        user_message: &str,
        max_tokens: u32,
    ) -> Result<(String, usize), ExecutionError> {
        if let Some(router) = &self.llm_router {
            let request = CompletionRequest {
                request_id: Uuid::new_v4(),
                messages: vec![
                    NormalizedMessage {
                        role: MessageRole::System,
                        content: vec![ContentBlock::Text { text: system_prompt.to_string() }],
                    },
                    NormalizedMessage {
                        role: MessageRole::User,
                        content: vec![ContentBlock::Text { text: user_message.to_string() }],
                    },
                ],
                parameters: CompletionParameters {
                    max_tokens,
                    temperature: Some(0.7),
                    ..Default::default()
                },
                tools: None,
                session_id: Uuid::nil(),
                stream: false,
                required_capabilities: vec![],
            };

            match router.route(&request).await {
                Ok(response) => {
                    let tokens = response.usage.total() as usize;
                    let content = response.content.iter()
                        .filter_map(|b| if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok((content, tokens))
                }
                Err(_e) => Err(ExecutionError::LlmExhausted {
                    task_id: Uuid::nil(),
                }),
            }
        } else {
            // Mock implementation for testing
            Ok((
                format!("Mock RCS response for: {}", &user_message[..user_message.len().min(50)]),
                50,
            ))
        }
    }

    /// Executes the **Reason** phase with fresh context.
    ///
    /// System: You are a reasoning engine. Analyze the given task and produce
    /// a comprehensive, structured response with clear reasoning.
    pub async fn run_reason(
        &self,
        task: &Task,
        context: &ExecutionContext,
    ) -> Result<ReasonOutput, ExecutionError> {
        let start = Instant::now();
        info!("R/C/S: Starting Reason phase for task {}", task.id);

        let system = "\
You are a high-precision reasoning engine. Your task is to analyze the given problem \
and produce a comprehensive, structured response. Focus on:
- Understanding the problem deeply
- Identifying the key factors and considerations
- Producing a clear, actionable plan or answer
- Being explicit about assumptions
Be thorough and precise. This response will be reviewed by a critic.";

        let user = format!(
            "## Task\n{}\n\n## Description\n{}\n\nProduce your best reasoning and response for this task.",
            task.title, task.description
        );

        let (content, tokens_used) = self.fresh_context_call(system, &user, 4096).await?;

        // Emit ReasonCompleted event
        self.emit(context.session_id, ReasoningEventPayload::ReasonCompleted {
            task_id: task.id,
            summary: content.chars().take(200).collect::<String>(),
            token_count: tokens_used as u32,
        }).await;

        info!("R/C/S: Reason phase complete ({} tokens, {}ms)",
            tokens_used, start.elapsed().as_millis());

        Ok(ReasonOutput { content, tokens_used })
    }

    /// Executes the **Critic** phase with fresh context.
    ///
    /// System: You are an adversarial critic. Your role is to find every flaw,
    /// gap, and failure mode in the provided reasoning.
    pub async fn run_critic(
        &self,
        task: &Task,
        reason_output: &ReasonOutput,
        context: &ExecutionContext,
    ) -> Result<CriticOutput, ExecutionError> {
        let start = Instant::now();
        info!("R/C/S: Starting Critic phase for task {}", task.id);

        let system = "\
You are an adversarial critic. Your sole purpose is to find every flaw, gap, and failure mode \
in the reasoning you are presented with. Be rigorous and uncharitable. Identify:
- Logical fallacies or unsound reasoning
- Missing considerations or edge cases
- Assumptions that may not hold
- Potential failure modes or risks
- Alternative interpretations that were ignored
Do NOT be constructive — that is the Synthesis phase's job. Just identify problems clearly.
End your response with a line: VERDICT: APPROVED or VERDICT: ISSUES FOUND";

        let user = format!(
            "## Original Task\n{}\n\n## Reasoning to Critique\n{}\n\nIdentify all flaws, gaps, and failure modes in this reasoning.",
            task.description, reason_output.content
        );

        let (content, tokens_used) = self.fresh_context_call(system, &user, 2048).await?;

        // Parse verdict
        let approved = content.contains("VERDICT: APPROVED");
        let issues: Vec<String> = if !approved {
            content.lines()
                .filter(|l| l.starts_with("- ") || l.starts_with("* "))
                .map(|l| l.trim_start_matches("- ").trim_start_matches("* ").to_string())
                .collect()
        } else {
            vec![]
        };

        // Emit CriticCompleted event
        self.emit(context.session_id, ReasoningEventPayload::CriticCompleted {
            task_id: task.id,
            approved,
            issues: issues.clone(),
            token_count: tokens_used as u32,
        }).await;

        info!("R/C/S: Critic phase complete (approved={}, issues={}, {}ms)",
            approved, issues.len(), start.elapsed().as_millis());

        Ok(CriticOutput { content, approved, issues, tokens_used })
    }

    /// Executes the **Synthesis** phase with fresh context.
    ///
    /// System: You are a synthesis engine. Combine the original reasoning and
    /// the critic's objections into a final, superior response.
    pub async fn run_synthesis(
        &self,
        task: &Task,
        reason_output: &ReasonOutput,
        critic_output: &CriticOutput,
        context: &ExecutionContext,
    ) -> Result<SynthesisOutput, ExecutionError> {
        let start = Instant::now();
        info!("R/C/S: Starting Synthesis phase for task {}", task.id);

        let system = "\
You are a synthesis engine. You have been given:
1. An original reasoning/response to a task
2. A critic's analysis of that reasoning

Your job is to produce a FINAL, SUPERIOR response that:
- Preserves the valid strengths of the original reasoning
- Addresses every valid criticism raised by the critic
- Discards criticisms that are irrelevant or misguided (with explanation)
- Produces a definitive, high-confidence answer or plan
Be explicit about which criticisms you addressed and how.";

        let user = format!(
            "## Original Task\n{}\n\n## Original Reasoning\n{}\n\n## Critic's Objections\n{}\n\nProduce the final synthesized response.",
            task.description, reason_output.content, critic_output.content
        );

        let (content, tokens_used) = self.fresh_context_call(system, &user, 4096).await?;

        // Extract resolved conflicts
        let resolved: Vec<String> = critic_output.issues.iter()
            .take(5) // Simplified: assume we resolved the top issues
            .cloned()
            .collect();

        // Emit SynthesisCompleted event
        self.emit(context.session_id, ReasoningEventPayload::SynthesisCompleted {
            task_id: task.id,
            final_decision: content.chars().take(200).collect::<String>(),
            resolved_conflicts: resolved.clone(),
            token_count: tokens_used as u32,
        }).await;

        info!("R/C/S: Synthesis phase complete ({} tokens, {}ms)",
            tokens_used, start.elapsed().as_millis());

        Ok(SynthesisOutput {
            content,
            resolved_conflicts: resolved,
            tokens_used,
        })
    }
}

#[async_trait]
impl ExecutionStrategy for RCSExecutionStrategy {
    fn name(&self) -> &str {
        "rcs"
    }

    fn is_applicable(&self, task: &Task) -> bool {
        matches!(task.execution_mode, ExecutionMode::ReasonCriticSynthesis)
    }

    /// Creates a three-step R/C/S plan.
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    async fn plan(&self, task: &Task) -> Result<Plan, ExecutionError> {
        let reason_id = Uuid::new_v4();
        let critic_id = Uuid::new_v4();
        let synthesis_id = Uuid::new_v4();

        let steps = vec![
            PlanStep {
                id: reason_id,
                step_number: 1,
                title: "Reason: Analyze and produce initial response".to_string(),
                description: format!(
                    "Fresh-context analysis of: {}. Produce comprehensive reasoning.",
                    task.title
                ),
                tools_expected: vec![],
                skills_expected: vec![],
                depends_on: vec![],
                estimated_tokens: 1500,
                status: PlanStepStatus::Pending,
                started_at: None,
                completed_at: None,
                actual_output: None,
            },
            PlanStep {
                id: critic_id,
                step_number: 2,
                title: "Critic: Identify flaws and failure modes".to_string(),
                description: "Adversarial review of the Reason phase output. Fresh context only.".to_string(),
                tools_expected: vec![],
                skills_expected: vec![],
                depends_on: vec![reason_id],
                estimated_tokens: 1000,
                status: PlanStepStatus::Pending,
                started_at: None,
                completed_at: None,
                actual_output: None,
            },
            PlanStep {
                id: synthesis_id,
                step_number: 3,
                title: "Synthesis: Produce final resolved response".to_string(),
                description: "Merge Reason output with Critic objections into final answer.".to_string(),
                tools_expected: vec![],
                skills_expected: vec![],
                depends_on: vec![reason_id, critic_id],
                estimated_tokens: 2000,
                status: PlanStepStatus::Pending,
                started_at: None,
                completed_at: None,
                actual_output: None,
            },
        ];

        Ok(Plan {
            id: Uuid::new_v4(),
            task_id: task.id,
            created_at: Utc::now(),
            approved_at: Some(Utc::now()),
            steps,
            estimated_tokens: 4500,
            estimated_duration_seconds: 120,
            mermaid_diagram: format!(
                "graph TD\n    A([Task: {}]) --> B[Reason]\n    B --> C[Critic]\n    C --> D[Synthesis]\n    D --> E([Complete])",
                task.title.chars().take(30).collect::<String>()
            ),
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        })
    }

    /// Executes an R/C/S step based on the step number.
    ///
    /// Step 1 = Reason, Step 2 = Critic, Step 3 = Synthesis.
    /// Each phase uses fresh context — no conversation history is passed.
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        let start = Instant::now();
        let task_desc = context.approved_plan.steps.first()
            .map(|s| s.description.clone())
            .unwrap_or_default();

        // Construct a minimal task from context
        let task = Task {
            id: context.task_id,
            parent_id: None,
            title: step.title.clone(),
            description: task_desc,
            constraints: vec![],
            context_requirements: vec![],
            execution_mode: ExecutionMode::ReasonCriticSynthesis,
            created_at: Utc::now(),
            deadline: None,
            priority: truenorth_core::types::task::TaskPriority::Normal,
            metadata: serde_json::Value::Null,
        };

        let (output_text, tokens_used) = match step.step_number {
            1 => {
                // Emit RCS activated
                self.emit(context.session_id, ReasoningEventPayload::RcsActivated {
                    task_id: context.task_id,
                    reason: "Task requires R/C/S mode".to_string(),
                    complexity_score: 0.8,
                }).await;

                let reason = self.run_reason(&task, context).await?;
                (reason.content, reason.tokens_used)
            }
            2 => {
                // Get Reason output from previous results
                let reason_content = context.previous_results.first()
                    .and_then(|r| r.output.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("No reason output available")
                    .to_string();

                let reason_output = ReasonOutput {
                    content: reason_content,
                    tokens_used: 0,
                };
                let critic = self.run_critic(&task, &reason_output, context).await?;
                (critic.content, critic.tokens_used)
            }
            3 => {
                // Get Reason and Critic outputs from previous results
                let reason_content = context.previous_results.first()
                    .and_then(|r| r.output.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("No reason output")
                    .to_string();
                let critic_content = context.previous_results.get(1)
                    .and_then(|r| r.output.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("No critic output")
                    .to_string();

                let reason_output = ReasonOutput { content: reason_content, tokens_used: 0 };
                let critic_output = CriticOutput {
                    content: critic_content,
                    approved: false,
                    issues: vec![],
                    tokens_used: 0,
                };
                let synthesis = self.run_synthesis(&task, &reason_output, &critic_output, context).await?;
                (synthesis.content, synthesis.tokens_used)
            }
            _ => {
                return Err(ExecutionError::StrategyMismatch {
                    strategy: "rcs".to_string(),
                    reason: format!("Unexpected step number {} in R/C/S plan", step.step_number),
                });
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let output_summary = if output_text.len() > 200 {
            format!("{}...", &output_text[..200])
        } else {
            output_text.clone()
        };

        Ok(StepResult {
            step_id: step.id,
            step_number: step.step_number,
            success: true,
            output: serde_json::json!({
                "text": output_text,
                "phase": match step.step_number {
                    1 => "reason",
                    2 => "critic",
                    3 => "synthesis",
                    _ => "unknown"
                }
            }),
            output_summary,
            tool_calls_made: vec![],
            tokens_used,
            execution_ms: duration_ms,
            deviation_detected: false,
        })
    }

    /// Complete when all three R/C/S phases have results.
    fn is_complete(&self, _plan: &Plan, results: &[StepResult]) -> bool {
        results.len() >= 3 && results.iter().all(|r| r.success)
    }

    fn control_signal(&self, _plan: &Plan, _results: &[StepResult]) -> ExecutionControl {
        ExecutionControl::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::task::TaskPriority;

    fn make_rcs_task() -> Task {
        Task {
            id: Uuid::new_v4(),
            parent_id: None,
            title: "Complex strategic analysis".to_string(),
            description: "Analyze the trade-offs of approach A vs B and recommend a path forward.".to_string(),
            constraints: vec![],
            context_requirements: vec![],
            execution_mode: ExecutionMode::ReasonCriticSynthesis,
            created_at: Utc::now(),
            deadline: None,
            priority: TaskPriority::High,
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn rcs_plan_has_three_phases() {
        let strategy = RCSExecutionStrategy::new(None, None);
        let task = make_rcs_task();
        let plan = strategy.plan(&task).await.unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].step_number, 1);
        assert_eq!(plan.steps[1].step_number, 2);
        assert_eq!(plan.steps[2].step_number, 3);
    }

    #[tokio::test]
    async fn rcs_phases_have_correct_dependencies() {
        let strategy = RCSExecutionStrategy::new(None, None);
        let task = make_rcs_task();
        let plan = strategy.plan(&task).await.unwrap();
        // Reason has no deps
        assert!(plan.steps[0].depends_on.is_empty());
        // Critic depends on Reason
        assert_eq!(plan.steps[1].depends_on.len(), 1);
        // Synthesis depends on both
        assert_eq!(plan.steps[2].depends_on.len(), 2);
    }

    #[tokio::test]
    async fn run_reason_returns_mock_output_without_llm() {
        let strategy = RCSExecutionStrategy::new(None, None);
        let task = make_rcs_task();
        let plan = strategy.plan(&task).await.unwrap();
        let context = ExecutionContext {
            session_id: Uuid::new_v4(),
            task_id: task.id,
            step_number: 1,
            approved_plan: plan,
            previous_results: vec![],
        };
        let result = strategy.run_reason(&task, &context).await.unwrap();
        assert!(!result.content.is_empty());
        assert!(result.tokens_used > 0);
    }
}
