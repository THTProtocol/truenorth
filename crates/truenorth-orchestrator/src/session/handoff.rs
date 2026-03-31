//! Handoff document generation for session context transfer.
//!
//! When the context window approaches exhaustion, a handoff document preserves
//! task continuity across session boundaries. The new session loads this
//! document as its initial system context.

use chrono::Utc;
use uuid::Uuid;

use truenorth_core::types::session::{HandoffDocument, SessionState};

/// Generates handoff documents from session state.
pub struct HandoffGenerator;

impl HandoffGenerator {
    /// Generates a handoff document from the current session state.
    ///
    /// The handoff document is designed to be loaded as the initial system
    /// context for the continuation session. It provides a compact summary
    /// of what was accomplished and what still needs to be done.
    pub fn generate(state: &SessionState) -> HandoffDocument {
        let completed_steps = Self::extract_completed_steps(state);
        let remaining_steps = Self::extract_remaining_steps(state);
        let critical_context = Self::extract_critical_context(state);
        let objective = Self::extract_objective(state);
        let modified_files = Self::extract_modified_files(state);

        HandoffDocument {
            from_session_id: state.session_id,
            to_session_id: Uuid::new_v4(),
            created_at: Utc::now(),
            objective,
            completed_steps,
            remaining_steps,
            critical_context,
            original_plan: state.active_plan.clone(),
            memory_references: vec![],
            modified_files,
            resume_from_state: state.agent_state.clone(),
        }
    }

    /// Extracts the session objective from the task description or title.
    fn extract_objective(state: &SessionState) -> String {
        state.current_task
            .as_ref()
            .and_then(|t| t.get("description"))
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(500).collect())
            .unwrap_or_else(|| format!("Continue session: {}", state.title))
    }

    /// Extracts completed step summaries from the plan or reasoning events.
    fn extract_completed_steps(state: &SessionState) -> Vec<String> {
        if let Some(plan) = &state.active_plan {
            if let Some(steps) = plan.get("steps").and_then(|v| v.as_array()) {
                return steps.iter()
                    .filter(|s| {
                        s.get("status")
                            .and_then(|v| v.as_str())
                            .map(|st| st == "Completed")
                            .unwrap_or(false)
                    })
                    .filter_map(|s| s.get("title").and_then(|v| v.as_str()).map(|t| t.to_string()))
                    .collect();
            }
        }

        // Fallback: extract from reasoning events
        state.reasoning_events.iter()
            .filter(|e| {
                e.get("payload")
                    .and_then(|p| p.get("type"))
                    .and_then(|v| v.as_str())
                    .map(|t| t == "step_completed")
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                e.get("payload")
                    .and_then(|p| p.get("output_summary"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect()
    }

    /// Extracts remaining (pending) steps from the active plan.
    fn extract_remaining_steps(state: &SessionState) -> Vec<String> {
        if let Some(plan) = &state.active_plan {
            if let Some(steps) = plan.get("steps").and_then(|v| v.as_array()) {
                return steps.iter()
                    .filter(|s| {
                        s.get("status")
                            .and_then(|v| v.as_str())
                            .map(|st| st == "Pending" || st == "InProgress")
                            .unwrap_or(true)
                    })
                    .filter_map(|s| s.get("description").and_then(|v| v.as_str()).map(|t| t.to_string()))
                    .collect();
            }
        }
        vec!["Continue from where the previous session left off".to_string()]
    }

    /// Extracts critical context points from the conversation history.
    fn extract_critical_context(state: &SessionState) -> Vec<String> {
        let mut context = Vec::new();

        // Add agent state
        context.push(format!("Agent was in state: {}", state.agent_state));

        // Add token usage info
        context.push(format!(
            "Context used: {} / {} tokens ({:.1}%)",
            state.context_tokens,
            state.context_budget,
            if state.context_budget > 0 {
                (state.context_tokens as f64 / state.context_budget as f64) * 100.0
            } else {
                0.0
            }
        ));

        // Add last few conversation turns
        let recent_turns: Vec<&serde_json::Value> = state.conversation_history
            .iter()
            .rev()
            .take(3)
            .collect();
        for turn in recent_turns.iter().rev() {
            if let Some(content) = turn.get("content").and_then(|v| v.as_str()) {
                let preview = content.chars().take(100).collect::<String>();
                context.push(format!("Recent: {}", preview));
            }
        }

        context
    }

    /// Extracts modified file paths from the conversation history.
    fn extract_modified_files(_state: &SessionState) -> Vec<String> {
        // In a full implementation, this would track tool calls that modified files.
        // For now, return empty list.
        vec![]
    }
}

/// Formats a `HandoffDocument` as a Markdown string for display or injection.
pub fn format_handoff_as_markdown(doc: &HandoffDocument) -> String {
    let mut md = String::new();
    md.push_str("# TrueNorth Session Handoff\n\n");
    md.push_str(&format!("**From session**: `{}`\n", doc.from_session_id));
    md.push_str(&format!("**Created**: {}\n\n", doc.created_at.to_rfc3339()));

    md.push_str("## Objective\n\n");
    md.push_str(&doc.objective);
    md.push_str("\n\n");

    if !doc.completed_steps.is_empty() {
        md.push_str("## Completed\n\n");
        for step in &doc.completed_steps {
            md.push_str(&format!("- {}\n", step));
        }
        md.push('\n');
    }

    if !doc.remaining_steps.is_empty() {
        md.push_str("## Still To Do\n\n");
        for step in &doc.remaining_steps {
            md.push_str(&format!("- {}\n", step));
        }
        md.push('\n');
    }

    if !doc.critical_context.is_empty() {
        md.push_str("## Critical Context\n\n");
        for ctx in &doc.critical_context {
            md.push_str(&format!("- {}\n", ctx));
        }
        md.push('\n');
    }

    md.push_str(&format!("## Resume From\n\n`{}`\n", doc.resume_from_state));

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use truenorth_core::types::session::{LlmRoutingState, SessionState};

    fn make_session() -> SessionState {
        SessionState {
            session_id: Uuid::new_v4(),
            title: "Test session".to_string(),
            created_at: Utc::now(),
            snapshot_at: Utc::now(),
            agent_state: "Executing".to_string(),
            current_task: Some(serde_json::json!({
                "description": "Build a new feature for the API"
            })),
            conversation_history: vec![],
            active_plan: None,
            context_tokens: 8500,
            context_budget: 10000,
            routing_state: LlmRoutingState {
                primary_provider: "anthropic".to_string(),
                exhausted_providers: vec![],
                rate_limited_providers: vec![],
            },
            reasoning_events: vec![],
            save_reason: Some("context_limit".to_string()),
            schema_version: "1.0".to_string(),
        }
    }

    #[test]
    fn generate_handoff_has_required_fields() {
        let session = make_session();
        let doc = HandoffGenerator::generate(&session);
        assert_eq!(doc.from_session_id, session.session_id);
        assert_ne!(doc.to_session_id, Uuid::nil());
        assert!(!doc.objective.is_empty());
        assert!(!doc.critical_context.is_empty());
        assert_eq!(doc.resume_from_state, "Executing");
    }

    #[test]
    fn format_handoff_contains_sections() {
        let session = make_session();
        let doc = HandoffGenerator::generate(&session);
        let md = format_handoff_as_markdown(&doc);
        assert!(md.contains("# TrueNorth Session Handoff"));
        assert!(md.contains("## Objective"));
        assert!(md.contains("## Critical Context"));
        assert!(md.contains("## Resume From"));
    }
}
