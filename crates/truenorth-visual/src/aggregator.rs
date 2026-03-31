/// Event aggregator for the Visual Reasoning Layer.
///
/// The `EventAggregator` subscribes to the `EventBus` in a background tokio task
/// and maintains a running snapshot of the system's observable state. Leptos
/// server functions query the aggregator via snapshot methods rather than
/// replaying the full event history on every request.
///
/// ## Design
///
/// The aggregator holds all mutable state behind an `Arc<RwLock<AggregatorState>>`.
/// The background task has exclusive write access during event processing; all
/// snapshot methods acquire a read lock. The write lock is held only long enough
/// to apply a single event — it is never held across an `.await` point — so
/// snapshot queries are always responsive.
///
/// ## Lifecycle
///
/// 1. Create an `EventAggregator` via `EventAggregator::new(bus)`.
/// 2. Call `EventAggregator::spawn()` to start the background task. This returns
///    a `JoinHandle` that can be used to cancel the task on shutdown.
/// 3. Query snapshots via `current_task_graph()`, `active_steps()`, etc.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;
use uuid::Uuid;

use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::types::plan::{PlanStepStatus};

use crate::event_bus::{recv_handling_lag, EventBus};
use crate::mermaid::MermaidGenerator;
use crate::types::{
    ActiveStep, ContextUtilization, MemoryOperation, RoutingDecision, TaskEdge,
    TaskGraphSnapshot, TaskNode,
};

// ---------------------------------------------------------------------------
// Internal aggregated state
// ---------------------------------------------------------------------------

/// All mutable state maintained by the aggregator.
#[derive(Debug, Default)]
struct AggregatorState {
    /// All known task nodes, keyed by step ID.
    task_nodes: HashMap<Uuid, TaskNode>,
    /// All dependency edges between steps.
    task_edges: Vec<TaskEdge>,
    /// The most recently built Mermaid string for the task graph.
    mermaid_diagram: String,
    /// Steps that are currently executing, keyed by step ID.
    active_steps: HashMap<Uuid, ActiveStep>,
    /// Current context window utilization.
    context_utilization: ContextUtilization,
    /// Log of LLM routing decisions (most recent 200).
    routing_log: Vec<RoutingDecision>,
    /// Log of memory operations (most recent 200).
    memory_operations: Vec<MemoryOperation>,
}

impl AggregatorState {
    /// Applies a single `ReasoningEvent` to the aggregated state.
    fn apply(&mut self, event: &ReasoningEvent) {
        match &event.payload {
            // ---------------------------------------------------------------
            // Plan / step events
            // ---------------------------------------------------------------
            ReasoningEventPayload::PlanCreated {
                mermaid_diagram,
                task_id,
                step_count,
                ..
            } => {
                // Store the initial Mermaid diagram from the PlanCreated event.
                self.mermaid_diagram = mermaid_diagram.clone();
                debug!(%task_id, step_count, "PlanCreated: stored initial Mermaid diagram");
            }

            ReasoningEventPayload::StepStarted {
                step_id,
                title,
                description,
                step_number,
                ..
            } => {
                // Mark node as in-progress.
                if let Some(node) = self.task_nodes.get_mut(step_id) {
                    node.status = PlanStepStatus::InProgress;
                    self.mermaid_diagram = MermaidGenerator::update_node_status(
                        &self.mermaid_diagram,
                        *step_number,
                        &PlanStepStatus::InProgress,
                    );
                } else {
                    // Node not yet in state (e.g. plan arrived via replay) — insert it.
                    self.task_nodes.insert(
                        *step_id,
                        TaskNode {
                            id: *step_id,
                            title: title.clone(),
                            description: description.clone(),
                            status: PlanStepStatus::InProgress,
                        },
                    );
                }

                // Track as active step.
                let now = Utc::now();
                self.active_steps.insert(
                    *step_id,
                    ActiveStep {
                        step_id: *step_id,
                        title: title.clone(),
                        description: description.clone(),
                        started_at: now,
                        duration_ms: 0,
                    },
                );
            }

            ReasoningEventPayload::StepCompleted {
                step_id,
                step_number,
                ..
            } => {
                if let Some(node) = self.task_nodes.get_mut(step_id) {
                    node.status = PlanStepStatus::Completed;
                }
                self.mermaid_diagram = MermaidGenerator::update_node_status(
                    &self.mermaid_diagram,
                    *step_number,
                    &PlanStepStatus::Completed,
                );
                self.active_steps.remove(step_id);
            }

            ReasoningEventPayload::StepFailed {
                step_id,
                step_number,
                error,
                ..
            } => {
                if let Some(node) = self.task_nodes.get_mut(step_id) {
                    node.status = PlanStepStatus::Failed {
                        error: error.clone(),
                    };
                }
                self.mermaid_diagram = MermaidGenerator::update_node_status(
                    &self.mermaid_diagram,
                    *step_number,
                    &PlanStepStatus::Failed {
                        error: error.clone(),
                    },
                );
                self.active_steps.remove(step_id);
            }

            // ---------------------------------------------------------------
            // LLM routing events
            // ---------------------------------------------------------------
            ReasoningEventPayload::LlmRouted {
                request_id,
                provider,
                model,
                usage,
                latency_ms,
                fallback_number,
            } => {
                let decision = RoutingDecision {
                    request_id: *request_id,
                    provider: provider.clone(),
                    model: model.clone(),
                    fallback_number: *fallback_number,
                    tokens_used: usage.total(),
                    latency_ms: *latency_ms,
                    timestamp: event.timestamp,
                };
                push_bounded(&mut self.routing_log, decision, 200);

                // Update context utilization from token usage.
                let total = self.context_utilization.tokens_used + usage.total();
                let max = self.context_utilization.tokens_max;
                let pct = if max > 0 {
                    (total as f32 / max as f32) * 100.0
                } else {
                    0.0
                };
                self.context_utilization = ContextUtilization {
                    tokens_used: total,
                    tokens_max: max,
                    percentage: pct,
                    state_label: budget_label(pct),
                    updated_at: event.timestamp,
                };
            }

            ReasoningEventPayload::ContextCompacted {
                after_tokens,
                ..
            } => {
                let max = self.context_utilization.tokens_max;
                let pct = if max > 0 {
                    (*after_tokens as f32 / max as f32) * 100.0
                } else {
                    0.0
                };
                self.context_utilization = ContextUtilization {
                    tokens_used: *after_tokens,
                    tokens_max: max,
                    percentage: pct,
                    state_label: budget_label(pct),
                    updated_at: event.timestamp,
                };
            }

            // ---------------------------------------------------------------
            // Memory events
            // ---------------------------------------------------------------
            ReasoningEventPayload::MemoryWritten {
                scope,
                content_preview,
                ..
            } => {
                let op = MemoryOperation {
                    scope: *scope,
                    operation: "write".to_string(),
                    content_preview: content_preview.clone(),
                    timestamp: event.timestamp,
                };
                push_bounded(&mut self.memory_operations, op, 200);
            }

            ReasoningEventPayload::MemoryQueried {
                scope,
                query_preview,
                ..
            } => {
                let op = MemoryOperation {
                    scope: *scope,
                    operation: "query".to_string(),
                    content_preview: query_preview.clone(),
                    timestamp: event.timestamp,
                };
                push_bounded(&mut self.memory_operations, op, 200);
            }

            ReasoningEventPayload::MemoryConsolidated {
                scope,
                entries_merged,
                ..
            } => {
                let op = MemoryOperation {
                    scope: *scope,
                    operation: "consolidate".to_string(),
                    content_preview: format!("{} entries merged", entries_merged),
                    timestamp: event.timestamp,
                };
                push_bounded(&mut self.memory_operations, op, 200);
            }

            // ---------------------------------------------------------------
            // Task completion — clear active steps
            // ---------------------------------------------------------------
            ReasoningEventPayload::TaskCompleted { .. }
            | ReasoningEventPayload::TaskFailed { .. }
            | ReasoningEventPayload::FatalError { .. } => {
                self.active_steps.clear();
            }

            // All other event variants are not currently aggregated.
            _ => {}
        }
    }

    /// Recomputes `duration_ms` for all active steps based on wall-clock elapsed time.
    fn refresh_active_step_durations(&mut self) {
        let now = Utc::now();
        for step in self.active_steps.values_mut() {
            let elapsed = now.signed_duration_since(step.started_at);
            step.duration_ms = elapsed.num_milliseconds().max(0) as u64;
        }
    }
}

// ---------------------------------------------------------------------------
// EventAggregator
// ---------------------------------------------------------------------------

/// Aggregates reasoning events into queryable snapshots for the Leptos frontend.
///
/// Holds shared state behind `Arc<RwLock<>>` so snapshots can be queried from
/// multiple Axum handler threads concurrently while the background task applies
/// incoming events.
#[derive(Debug, Clone)]
pub struct EventAggregator {
    state: Arc<RwLock<AggregatorState>>,
    bus: EventBus,
}

impl EventAggregator {
    /// Creates a new `EventAggregator` connected to the given `EventBus`.
    ///
    /// Call `spawn()` to start the background event-consumption task.
    pub fn new(bus: EventBus) -> Self {
        Self {
            state: Arc::new(RwLock::new(AggregatorState::default())),
            bus,
        }
    }

    /// Spawns the background tokio task that consumes events from the bus.
    ///
    /// Returns a `JoinHandle` that can be awaited or aborted on shutdown.
    ///
    /// The task:
    /// - Subscribes to the `EventBus` broadcast channel.
    /// - Calls `state.apply(event)` for every received event.
    /// - Handles lagged receivers gracefully by logging a warning and continuing.
    /// - Exits cleanly when the broadcast channel closes (system shutdown).
    pub fn spawn(&self) -> JoinHandle<()> {
        let state = Arc::clone(&self.state);
        let mut rx = self.bus.subscribe();

        tokio::spawn(async move {
            debug!("EventAggregator background task started");
            loop {
                match recv_handling_lag(&mut rx).await {
                    Some(event) => {
                        let mut guard = state.write().await;
                        guard.apply(&event);
                    }
                    None => {
                        // Channel closed — graceful shutdown.
                        debug!("EventAggregator background task stopping: channel closed");
                        break;
                    }
                }
            }
        })
    }

    // -----------------------------------------------------------------------
    // Snapshot query methods
    // -----------------------------------------------------------------------

    /// Returns the current task dependency graph as a snapshot.
    ///
    /// Includes all known nodes (plan steps), dependency edges, and the most
    /// recently built Mermaid diagram string.
    pub async fn current_task_graph(&self) -> TaskGraphSnapshot {
        let guard = self.state.read().await;
        TaskGraphSnapshot {
            nodes: guard.task_nodes.values().cloned().collect(),
            edges: guard.task_edges.clone(),
            mermaid: guard.mermaid_diagram.clone(),
        }
    }

    /// Returns all plan steps that are currently in the `InProgress` state.
    ///
    /// The `duration_ms` field of each `ActiveStep` is refreshed to reflect
    /// the current wall-clock elapsed time.
    pub async fn active_steps(&self) -> Vec<ActiveStep> {
        let mut guard = self.state.write().await;
        guard.refresh_active_step_durations();
        guard.active_steps.values().cloned().collect()
    }

    /// Returns the current context window utilization snapshot.
    pub async fn context_utilization(&self) -> ContextUtilization {
        let guard = self.state.read().await;
        guard.context_utilization.clone()
    }

    /// Returns the LLM routing decision log (most recent first).
    pub async fn routing_log(&self) -> Vec<RoutingDecision> {
        let guard = self.state.read().await;
        guard.routing_log.iter().rev().cloned().collect()
    }

    /// Returns the memory operation log (most recent first).
    pub async fn memory_operations(&self) -> Vec<MemoryOperation> {
        let guard = self.state.read().await;
        guard.memory_operations.iter().rev().cloned().collect()
    }

    /// Resets all aggregated state.
    ///
    /// Intended for use between test cases or when starting a fresh session.
    pub async fn reset(&self) {
        let mut guard = self.state.write().await;
        *guard = AggregatorState::default();
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Pushes `item` to `vec`, dropping the oldest entry if `vec.len() >= max_len`.
fn push_bounded<T>(vec: &mut Vec<T>, item: T, max_len: usize) {
    if vec.len() >= max_len {
        vec.remove(0);
    }
    vec.push(item);
}

/// Returns a human-readable context budget state label from a utilization percentage.
fn budget_label(pct: f32) -> String {
    match pct as u32 {
        0..=49 => "Green".to_string(),
        50..=69 => "Yellow".to_string(),
        70..=84 => "Orange".to_string(),
        85..=94 => "Red".to_string(),
        _ => "Critical".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_store::ReasoningEventStore;
    use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
    use truenorth_core::types::llm::TokenUsage;

    fn make_bus() -> EventBus {
        let store = ReasoningEventStore::open_in_memory().unwrap();
        EventBus::new(store)
    }

    fn step_started(session_id: Uuid, step_id: Uuid, step_number: usize) -> ReasoningEvent {
        ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::StepStarted {
                task_id: Uuid::new_v4(),
                plan_id: Uuid::new_v4(),
                step_id,
                step_number,
                title: format!("Step {}", step_number),
                description: "Test step".to_string(),
            },
        )
    }

    fn step_completed(session_id: Uuid, step_id: Uuid, step_number: usize) -> ReasoningEvent {
        ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::StepCompleted {
                task_id: Uuid::new_v4(),
                step_id,
                step_number,
                output_summary: "done".to_string(),
                duration_ms: 100,
            },
        )
    }

    #[tokio::test]
    async fn aggregator_tracks_active_steps() {
        let bus = make_bus();
        let agg = EventAggregator::new(bus.clone());
        let _handle = agg.spawn();

        let session_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();

        bus.emit(step_started(session_id, step_id, 1)).await.unwrap();
        // Give the background task time to process the event.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let steps = agg.active_steps().await;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_id, step_id);

        bus.emit(step_completed(session_id, step_id, 1)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let steps = agg.active_steps().await;
        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn aggregator_tracks_context_utilization() {
        let bus = make_bus();
        let agg = EventAggregator::new(bus.clone());
        let _handle = agg.spawn();

        let session_id = Uuid::new_v4();
        bus.emit(ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::LlmRouted {
                request_id: Uuid::new_v4(),
                provider: "anthropic".to_string(),
                model: "claude-3-5-sonnet".to_string(),
                usage: TokenUsage {
                    input_tokens: 1000,
                    output_tokens: 500,
                    ..Default::default()
                },
                latency_ms: 1200,
                fallback_number: 0,
            },
        ))
        .await
        .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let util = agg.context_utilization().await;
        assert_eq!(util.tokens_used, 1500); // 1000 input + 500 output
        assert_eq!(util.state_label, "Green");
    }

    #[test]
    fn push_bounded_respects_limit() {
        let mut v: Vec<i32> = Vec::new();
        for i in 0..=5 {
            push_bounded(&mut v, i, 5);
        }
        assert_eq!(v.len(), 5);
        // Oldest (0) should have been dropped; 5 is the newest.
        assert_eq!(*v.last().unwrap(), 5);
    }

    #[test]
    fn budget_label_correct() {
        assert_eq!(budget_label(30.0), "Green");
        assert_eq!(budget_label(55.0), "Yellow");
        assert_eq!(budget_label(75.0), "Orange");
        assert_eq!(budget_label(90.0), "Red");
        assert_eq!(budget_label(97.0), "Critical");
    }
}
