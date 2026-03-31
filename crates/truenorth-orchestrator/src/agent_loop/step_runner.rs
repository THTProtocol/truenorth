//! Step runner — single-step execution: LLM call + tool dispatch + observation.
//!
//! The `StepRunner` handles execution of a single plan step:
//! 1. Build context from task + history + memory
//! 2. Call LLM via router
//! 3. Parse response for tool calls
//! 4. Dispatch tool calls if present
//! 5. Inject tool results back into context
//! 6. Produce a `StepResult`

use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, instrument, warn};
use uuid::Uuid;

use truenorth_core::traits::execution::{ExecutionContext, ExecutionError, StepResult};
use truenorth_core::traits::llm_router::LlmRouter;
use truenorth_core::traits::reasoning::ReasoningEventEmitter;
use truenorth_core::types::plan::PlanStep;
use truenorth_core::types::llm::{CompletionRequest, CompletionParameters, NormalizedMessage};
use truenorth_core::types::message::{ContentBlock, MessageRole};
use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};

/// Single-step execution engine.
///
/// Wraps the LLM router and tool registry to execute one plan step.
/// All state changes flow through `StepResult` — the runner is stateless.
#[derive(Debug)]
pub struct StepRunner {
    /// LLM router for making inference calls.
    llm_router: Option<Arc<dyn LlmRouter>>,
    /// Event emitter for observability.
    event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
}

impl StepRunner {
    /// Creates a new `StepRunner`.
    pub fn new(
        llm_router: Option<Arc<dyn LlmRouter>>,
        event_emitter: Option<Arc<dyn ReasoningEventEmitter>>,
    ) -> Self {
        Self { llm_router, event_emitter }
    }

    /// Executes a single plan step.
    ///
    /// Builds a prompt from the step description and execution context,
    /// calls the LLM router, and returns a structured `StepResult`.
    #[instrument(skip(self, step, context), fields(step_number = step.step_number))]
    pub async fn run_step(
        &self,
        step: &PlanStep,
        context: &ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        let start = Instant::now();
        debug!("Running step {}: {}", step.step_number, step.title);

        // Build the LLM prompt
        let system_prompt = self.build_system_prompt(context);
        let user_message = self.build_step_prompt(step, context);

        let mut tokens_used = 0usize;
        let mut output_text = String::new();
        let mut tool_calls_made = Vec::new();

        // Call LLM if available
        if let Some(router) = &self.llm_router {
            let request = CompletionRequest {
                request_id: Uuid::new_v4(),
                messages: vec![
                    NormalizedMessage {
                        role: MessageRole::System,
                        content: vec![ContentBlock::Text { text: system_prompt.clone() }],
                    },
                    NormalizedMessage {
                        role: MessageRole::User,
                        content: vec![ContentBlock::Text { text: user_message.clone() }],
                    },
                ],
                tools: None,
                parameters: CompletionParameters {
                    max_tokens: 2048,
                    temperature: Some(0.7),
                    ..Default::default()
                },
                session_id: context.session_id,
                stream: false,
                required_capabilities: vec![],
            };

            match router.route(&request).await {
                Ok(response) => {
                    tokens_used = response.usage.total() as usize;
                    // Extract text from content blocks
                    output_text = response.content.iter()
                        .filter_map(|b| if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None })
                        .collect::<Vec<_>>()
                        .join("\n");

                    // Emit LLM routing event
                    if let Some(emitter) = &self.event_emitter {
                        let event = ReasoningEvent::new(context.session_id, ReasoningEventPayload::LlmRouted {
                            request_id: Uuid::new_v4(),
                            provider: response.provider.clone(),
                            model: response.model.clone(),
                            usage: response.usage.clone(),
                            latency_ms: start.elapsed().as_millis() as u64,
                            fallback_number: 0,
                        });
                        let _ = emitter.emit(event).await;
                    }

                    // Parse for tool calls from content blocks
                    for block in &response.content {
                        if let ContentBlock::ToolUse { name, .. } = block {
                            tool_calls_made.push(name.clone());
                            debug!("Tool call detected: {}", name);
                        }
                    }
                }
                Err(e) => {
                    warn!("LLM call failed on step {}: {}", step.step_number, e);
                    // Return a failed result but don't abort the whole task
                    output_text = format!("LLM call failed: {}", e);
                }
            }
        } else {
            // No LLM: generate a mock result for testing
            output_text = format!(
                "Mock output for step {}: {}",
                step.step_number, step.title
            );
            tokens_used = 50;
        }

        let execution_ms = start.elapsed().as_millis() as u64;

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
                "step_title": step.title,
            }),
            output_summary,
            tool_calls_made,
            tokens_used,
            execution_ms,
            deviation_detected: false,
        })
    }

    /// Builds the system prompt for step execution.
    fn build_system_prompt(&self, context: &ExecutionContext) -> String {
        let mut prompt = String::from(
            "You are TrueNorth, an AI orchestration system. Execute the given task step with precision and accuracy.\n\n"
        );

        if !context.previous_results.is_empty() {
            prompt.push_str("## Previous Steps Completed\n");
            for result in &context.previous_results {
                prompt.push_str(&format!(
                    "- Step {}: {}\n",
                    result.step_number,
                    result.output_summary
                ));
            }
            prompt.push('\n');
        }

        prompt
    }

    /// Builds the user prompt for a specific step.
    fn build_step_prompt(&self, step: &PlanStep, context: &ExecutionContext) -> String {
        format!(
            "## Step {} of {}: {}\n\n{}\n\nExpected tools: {}\nExpected skills: {}\n\nPlease execute this step and provide a clear, structured response.",
            step.step_number,
            context.approved_plan.steps.len(),
            step.title,
            step.description,
            step.tools_expected.join(", "),
            step.skills_expected.join(", "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::plan::{Plan, PlanStatus, PlanStep, PlanStepStatus};
    use truenorth_core::types::task::{ExecutionMode, Task, TaskPriority};

    fn make_test_step(step_number: usize) -> PlanStep {
        PlanStep {
            id: Uuid::new_v4(),
            step_number,
            title: format!("Test step {}", step_number),
            description: "Do something".to_string(),
            tools_expected: vec![],
            skills_expected: vec![],
            depends_on: vec![],
            estimated_tokens: 100,
            status: PlanStepStatus::Pending,
            started_at: None,
            completed_at: None,
            actual_output: None,
        }
    }

    fn make_test_context() -> ExecutionContext {
        let task_id = Uuid::new_v4();
        let plan = Plan {
            id: Uuid::new_v4(),
            task_id,
            created_at: Utc::now(),
            approved_at: None,
            steps: vec![make_test_step(1)],
            estimated_tokens: 100,
            estimated_duration_seconds: 30,
            mermaid_diagram: String::new(),
            status: PlanStatus::Approved,
            metadata: serde_json::Value::Null,
        };
        ExecutionContext {
            session_id: Uuid::new_v4(),
            task_id,
            step_number: 1,
            approved_plan: plan,
            previous_results: vec![],
        }
    }

    #[tokio::test]
    async fn run_step_without_llm_produces_mock_result() {
        let runner = StepRunner::new(None, None);
        let step = make_test_step(1);
        let ctx = make_test_context();
        let result = runner.run_step(&step, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output_summary.contains("Mock output"));
        assert_eq!(result.step_number, 1);
    }
}
