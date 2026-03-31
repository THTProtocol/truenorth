/// Mermaid diagram generation for the Visual Reasoning Layer.
///
/// This module converts TrueNorth's internal data structures into syntactically
/// valid Mermaid DSL strings. The strings are then passed to the Leptos frontend,
/// which renders them client-side via `mermaid.js`, or to the `DiagramRenderer`
/// for server-side SVG wrapping.
///
/// All generated Mermaid text uses `graph TD` (top-down flowchart) syntax
/// unless otherwise noted, and uses double-quoted node labels to handle special
/// characters safely.

use uuid::Uuid;

use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::types::plan::{Plan, PlanStep, PlanStepStatus};

/// Generates and updates Mermaid diagrams from TrueNorth data structures.
///
/// All methods are pure functions — they take data and return `String` without
/// any mutable state. Incrementally updating an existing diagram is handled
/// by `update_node_status`.
#[derive(Debug, Default)]
pub struct MermaidGenerator;

impl MermaidGenerator {
    /// Creates a new `MermaidGenerator`.
    pub fn new() -> Self {
        Self
    }

    /// Generates a `graph TD` Mermaid flowchart from a `Plan`.
    ///
    /// Each `PlanStep` becomes a rectangular node. Dependency edges (`depends_on`)
    /// are rendered as directed arrows. Node fill colour reflects the step's
    /// current `PlanStepStatus`:
    ///
    /// | Status      | Colour                    |
    /// |-------------|---------------------------|
    /// | Pending     | grey (`#9e9e9e`)          |
    /// | InProgress  | amber (`#f9a825`)         |
    /// | Completed   | green (`#2e7d32`)         |
    /// | Failed      | red (`#b71c1c`)           |
    /// | Skipped     | light grey (`#bdbdbd`)    |
    ///
    /// # Example output (abbreviated)
    /// ```text
    /// graph TD
    ///   N0["1. Write tests"]
    ///   N1["2. Implement feature"]
    ///   N0 --> N1
    ///   style N0 fill:#9e9e9e,color:#fff,stroke:#616161
    ///   style N1 fill:#f9a825,color:#000,stroke:#f57f17
    /// ```
    pub fn from_plan(plan: &Plan) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push("graph TD".to_string());

        if plan.steps.is_empty() {
            lines.push("  EMPTY[\"No steps defined\"]".to_string());
            return lines.join("\n");
        }

        // Build a stable node ID mapping: step UUID → Nx identifier.
        // Mermaid node IDs must start with a letter and contain no hyphens.
        let node_id = |step: &PlanStep| format!("N{}", step.step_number.saturating_sub(1));

        // Node declarations.
        for step in &plan.steps {
            let nid = node_id(step);
            let label = escape_mermaid_label(&format!("{}. {}", step.step_number, step.title));
            lines.push(format!("  {}[\"{}\"]", nid, label));
        }

        // Dependency edges.
        let step_map: std::collections::HashMap<Uuid, &PlanStep> =
            plan.steps.iter().map(|s| (s.id, s)).collect();

        for step in &plan.steps {
            for dep_id in &step.depends_on {
                if let Some(dep_step) = step_map.get(dep_id) {
                    lines.push(format!("  {} --> {}", node_id(dep_step), node_id(step)));
                }
            }
        }

        // Style declarations.
        for step in &plan.steps {
            let nid = node_id(step);
            let style = step_status_style(&step.status);
            lines.push(format!("  style {} {}", nid, style));
        }

        lines.join("\n")
    }

    /// Generates a Mermaid timeline/sequence diagram summarising a slice of
    /// `ReasoningEvent`s.
    ///
    /// Produces a `graph TD` where each event is a node labelled with its type
    /// and a brief content summary, linked in chronological order.  This gives
    /// a linear "event tape" view suitable for the `EventTimeline` frontend
    /// component.
    ///
    /// Events are truncated to 60 characters to keep node labels readable.
    pub fn from_events(events: &[ReasoningEvent]) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push("graph TD".to_string());

        if events.is_empty() {
            lines.push("  EMPTY[\"No events recorded\"]".to_string());
            return lines.join("\n");
        }

        let mut prev_nid: Option<String> = None;

        for (i, event) in events.iter().enumerate() {
            let nid = format!("E{}", i);
            let (type_tag, summary) = event_label(&event.payload);
            let ts = event.timestamp.format("%H:%M:%S").to_string();
            let label = escape_mermaid_label(&format!("[{}] {}: {}", ts, type_tag, summary));
            lines.push(format!("  {}[\"{}\"]", nid, label));

            if let Some(prev) = &prev_nid {
                lines.push(format!("  {} --> {}", prev, nid));
            }

            // Style each event node by category.
            let style = event_category_style(&event.payload);
            lines.push(format!("  style {} {}", nid, style));

            prev_nid = Some(nid);
        }

        lines.join("\n")
    }

    /// Updates the style of a single node in an existing Mermaid diagram string.
    ///
    /// Locates the `style Nx ...` line for the step identified by its 1-based
    /// `step_number` (which is used to build the stable `N{step_number-1}` ID)
    /// and replaces the style declaration to reflect `new_status`.
    ///
    /// If no matching `style` line is found (e.g. the step is not in this
    /// diagram), the original string is returned unchanged.
    ///
    /// This avoids regenerating the entire diagram on every step transition,
    /// keeping the incremental update path cheap.
    pub fn update_node_status(
        mermaid: &str,
        step_number: usize,
        new_status: &PlanStepStatus,
    ) -> String {
        let nid = format!("N{}", step_number.saturating_sub(1));
        let new_style_line = format!("  style {} {}", nid, step_status_style(new_status));
        let prefix = format!("  style {} ", nid);

        let mut updated = false;
        let result = mermaid
            .lines()
            .map(|line| {
                if line.starts_with(&prefix) {
                    updated = true;
                    new_style_line.clone()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if updated { result } else { mermaid.to_string() }
    }

    /// Generates a Mermaid diagram illustrating the R/C/S (Reason/Critic/Synthesis)
    /// loop for a single activation, with content summaries in each node.
    ///
    /// The diagram shows the three phases as a linear flow with a feedback loop
    /// from Critic back to Reason when the critic did not approve.
    ///
    /// # Arguments
    /// * `reason` — summary from the Reason phase.
    /// * `critique` — summary from the Critic phase.
    /// * `synthesis` — summary from the Synthesis phase.
    pub fn from_rcs_loop(reason: &str, critique: &str, synthesis: &str) -> String {
        let r_label = escape_mermaid_label(&truncate(reason, 80));
        let c_label = escape_mermaid_label(&truncate(critique, 80));
        let s_label = escape_mermaid_label(&truncate(synthesis, 80));

        format!(
            r#"graph TD
  REASON["Reason\n{r}"]
  CRITIC["Critic\n{c}"]
  SYNTH["Synthesis\n{s}"]
  REASON --> CRITIC
  CRITIC -->|"Approved"| SYNTH
  CRITIC -->|"Issues found"| REASON
  style REASON fill:#1565c0,color:#fff,stroke:#0d47a1
  style CRITIC fill:#6a1b9a,color:#fff,stroke:#4a148c
  style SYNTH fill:#2e7d32,color:#fff,stroke:#1b5e20"#,
            r = r_label,
            c = c_label,
            s = s_label,
        )
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Returns a Mermaid `style` attribute string for a `PlanStepStatus`.
fn step_status_style(status: &PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "fill:#9e9e9e,color:#fff,stroke:#616161",
        PlanStepStatus::InProgress => "fill:#f9a825,color:#000,stroke:#f57f17",
        PlanStepStatus::Completed => "fill:#2e7d32,color:#fff,stroke:#1b5e20",
        PlanStepStatus::Failed { .. } => "fill:#b71c1c,color:#fff,stroke:#7f0000",
        PlanStepStatus::Skipped { .. } => "fill:#bdbdbd,color:#424242,stroke:#9e9e9e",
    }
}

/// Returns a Mermaid `style` attribute string for a reasoning event category.
fn event_category_style(payload: &ReasoningEventPayload) -> &'static str {
    match payload {
        ReasoningEventPayload::TaskReceived { .. } | ReasoningEventPayload::TaskCompleted { .. } | ReasoningEventPayload::TaskFailed { .. } => {
            "fill:#1565c0,color:#fff,stroke:#0d47a1"
        }
        ReasoningEventPayload::StepStarted { .. } | ReasoningEventPayload::StepCompleted { .. } => {
            "fill:#2e7d32,color:#fff,stroke:#1b5e20"
        }
        ReasoningEventPayload::StepFailed { .. } => "fill:#b71c1c,color:#fff,stroke:#7f0000",
        ReasoningEventPayload::ToolCalled { .. } | ReasoningEventPayload::ToolResult { .. } => {
            "fill:#6a1b9a,color:#fff,stroke:#4a148c"
        }
        ReasoningEventPayload::LlmRouted { .. }
        | ReasoningEventPayload::LlmFallback { .. }
        | ReasoningEventPayload::LlmExhausted { .. } => {
            "fill:#e65100,color:#fff,stroke:#bf360c"
        }
        ReasoningEventPayload::RcsActivated { .. }
        | ReasoningEventPayload::ReasonCompleted { .. }
        | ReasoningEventPayload::CriticCompleted { .. }
        | ReasoningEventPayload::SynthesisCompleted { .. } => {
            "fill:#880e4f,color:#fff,stroke:#560027"
        }
        ReasoningEventPayload::MemoryWritten { .. }
        | ReasoningEventPayload::MemoryQueried { .. }
        | ReasoningEventPayload::MemoryConsolidated { .. } => {
            "fill:#004d40,color:#fff,stroke:#00251a"
        }
        ReasoningEventPayload::ContextCompacted { .. } => {
            "fill:#f57f17,color:#000,stroke:#bc5100"
        }
        ReasoningEventPayload::FatalError { .. } => {
            "fill:#212121,color:#ff5252,stroke:#b71c1c"
        }
        _ => "fill:#37474f,color:#fff,stroke:#263238",
    }
}

/// Extracts a human-readable (type_tag, summary) pair from an event payload.
fn event_label(payload: &ReasoningEventPayload) -> (&'static str, String) {
    match payload {
        ReasoningEventPayload::TaskReceived { title, .. } => {
            ("TaskReceived", truncate(title, 60))
        }
        ReasoningEventPayload::PlanCreated { step_count, .. } => {
            ("PlanCreated", format!("{} steps", step_count))
        }
        ReasoningEventPayload::PlanApproved { .. } => ("PlanApproved", "User approved".into()),
        ReasoningEventPayload::StepStarted { title, step_number, .. } => {
            ("StepStarted", format!("#{} {}", step_number, truncate(title, 50)))
        }
        ReasoningEventPayload::StepCompleted { output_summary, .. } => {
            ("StepCompleted", truncate(output_summary, 60))
        }
        ReasoningEventPayload::StepFailed { error, .. } => {
            ("StepFailed", truncate(error, 60))
        }
        ReasoningEventPayload::ToolCalled { tool_name, .. } => {
            ("ToolCalled", truncate(tool_name, 60))
        }
        ReasoningEventPayload::ToolResult { tool_name, success, .. } => {
            let outcome = if *success { "ok" } else { "fail" };
            ("ToolResult", format!("{} [{}]", tool_name, outcome))
        }
        ReasoningEventPayload::LlmRouted { provider, model, .. } => {
            ("LlmRouted", format!("{}/{}", provider, model))
        }
        ReasoningEventPayload::LlmFallback { failed_provider, next_provider, .. } => {
            ("LlmFallback", format!("{} → {}", failed_provider, next_provider))
        }
        ReasoningEventPayload::LlmExhausted { .. } => ("LlmExhausted", "All providers failed".into()),
        ReasoningEventPayload::RcsActivated { reason, .. } => {
            ("RcsActivated", truncate(reason, 60))
        }
        ReasoningEventPayload::ReasonCompleted { summary, .. } => {
            ("ReasonCompleted", truncate(summary, 60))
        }
        ReasoningEventPayload::CriticCompleted { approved, .. } => {
            ("CriticCompleted", if *approved { "Approved" } else { "Issues found" }.into())
        }
        ReasoningEventPayload::SynthesisCompleted { final_decision, .. } => {
            ("SynthesisCompleted", truncate(final_decision, 60))
        }
        ReasoningEventPayload::ContextCompacted { before_tokens, after_tokens, .. } => {
            ("ContextCompacted", format!("{} → {} tokens", before_tokens, after_tokens))
        }
        ReasoningEventPayload::MemoryWritten { content_preview, .. } => {
            ("MemoryWritten", truncate(content_preview, 60))
        }
        ReasoningEventPayload::MemoryQueried { query_preview, .. } => {
            ("MemoryQueried", truncate(query_preview, 60))
        }
        ReasoningEventPayload::MemoryConsolidated { entries_merged, .. } => {
            ("MemoryConsolidated", format!("{} entries merged", entries_merged))
        }
        ReasoningEventPayload::DeviationDetected { severity, .. } => {
            ("DeviationDetected", format!("{:?}", severity))
        }
        ReasoningEventPayload::ChecklistVerified { passed, failed, .. } => {
            ("ChecklistVerified", format!("{} pass / {} fail", passed, failed))
        }
        ReasoningEventPayload::SessionSaved { reason, .. } => {
            ("SessionSaved", truncate(reason, 60))
        }
        ReasoningEventPayload::SessionResumed { resumed_at_step, .. } => {
            ("SessionResumed", format!("at step {}", resumed_at_step))
        }
        ReasoningEventPayload::HeartbeatFired { tick_count, .. } => {
            ("HeartbeatFired", format!("tick #{}", tick_count))
        }
        ReasoningEventPayload::SkillActivated { skill_name, .. } => {
            ("SkillActivated", truncate(skill_name, 60))
        }
        ReasoningEventPayload::TaskCompleted { output_summary, .. } => {
            ("TaskCompleted", truncate(output_summary, 60))
        }
        ReasoningEventPayload::TaskFailed { error, .. } => {
            ("TaskFailed", truncate(error, 60))
        }
        ReasoningEventPayload::FatalError { error, .. } => {
            ("FatalError", truncate(error, 60))
        }
    }
}

/// Escapes characters that have special meaning in Mermaid node label strings.
///
/// Double quotes are replaced with single quotes (Mermaid uses `"..."` for labels).
/// Newlines embedded via `\n` are preserved (Mermaid renders them as line breaks
/// inside nodes when using the `<br/>` entity — we keep `\n` as literal text here
/// since the frontend's mermaid.js handles them).
fn escape_mermaid_label(s: &str) -> String {
    s.replace('"', "'")
        .replace('#', "\\#")
}

/// Truncates a string to at most `max_chars` Unicode characters, appending `…`
/// if truncation occurred.
fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars.saturating_sub(1)].iter().collect();
        format!("{}…", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::plan::{Plan, PlanStep, PlanStatus, PlanStepStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_plan(steps: Vec<PlanStep>) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            created_at: Utc::now(),
            approved_at: None,
            steps,
            estimated_tokens: 0,
            estimated_duration_seconds: 0,
            mermaid_diagram: String::new(),
            status: PlanStatus::Executing,
            metadata: serde_json::Value::Null,
        }
    }

    fn make_step(n: usize, status: PlanStepStatus) -> PlanStep {
        PlanStep {
            id: Uuid::new_v4(),
            step_number: n,
            title: format!("Step {}", n),
            description: format!("Do thing {}", n),
            tools_expected: vec![],
            skills_expected: vec![],
            depends_on: vec![],
            estimated_tokens: 0,
            status,
            started_at: None,
            completed_at: None,
            actual_output: None,
        }
    }

    #[test]
    fn from_plan_produces_graph_td() {
        let plan = make_plan(vec![
            make_step(1, PlanStepStatus::Completed),
            make_step(2, PlanStepStatus::InProgress),
        ]);
        let diagram = MermaidGenerator::from_plan(&plan);
        assert!(diagram.starts_with("graph TD"));
        assert!(diagram.contains("N0["));
        assert!(diagram.contains("N1["));
        assert!(diagram.contains("fill:#2e7d32")); // completed = green
        assert!(diagram.contains("fill:#f9a825")); // in_progress = amber
    }

    #[test]
    fn from_plan_empty() {
        let plan = make_plan(vec![]);
        let diagram = MermaidGenerator::from_plan(&plan);
        assert!(diagram.contains("No steps defined"));
    }

    #[test]
    fn update_node_status_changes_style() {
        let plan = make_plan(vec![make_step(1, PlanStepStatus::Pending)]);
        let diagram = MermaidGenerator::from_plan(&plan);
        assert!(diagram.contains("fill:#9e9e9e")); // pending = grey

        let updated = MermaidGenerator::update_node_status(&diagram, 1, &PlanStepStatus::Completed);
        assert!(updated.contains("fill:#2e7d32")); // completed = green
        assert!(!updated.contains("fill:#9e9e9e")); // grey gone
    }

    #[test]
    fn from_rcs_loop_contains_all_phases() {
        let diagram = MermaidGenerator::from_rcs_loop("Analyse X", "Looks good", "Use X");
        assert!(diagram.contains("REASON"));
        assert!(diagram.contains("CRITIC"));
        assert!(diagram.contains("SYNTH"));
        assert!(diagram.contains("Reason"));
        assert!(diagram.contains("Critic"));
        assert!(diagram.contains("Synthesis"));
    }

    #[test]
    fn escape_double_quotes() {
        let label = escape_mermaid_label("say \"hello\" world");
        assert!(!label.contains('"'));
        assert!(label.contains('\''));
    }

    #[test]
    fn truncate_long_string() {
        let s = "a".repeat(100);
        let result = truncate(&s, 10);
        assert_eq!(result.chars().count(), 10); // 9 chars + ellipsis
    }
}
