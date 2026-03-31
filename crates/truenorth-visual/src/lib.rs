//! # truenorth-visual
//!
//! **Visual Reasoning Layer** for TrueNorth.
//!
//! This crate implements the event bus, persistent event store, Mermaid diagram
//! generation, and state aggregation that power TrueNorth's real-time visual
//! reasoning UI.
//!
//! ## Architecture
//!
//! ```text
//!  Agent Loop / Orchestrator
//!      │
//!      │  EventBus::emit(ReasoningEvent)
//!      ▼
//!  ┌──────────────────────────────────────────────────────────┐
//!  │  EventBus (tokio::sync::broadcast, capacity: 1024)       │
//!  │  ┌─────────────────────────────────────────────────────┐ │
//!  │  │  ReasoningEventStore (SQLite WAL)                   │ │
//!  │  │  — every event persisted append-only                │ │
//!  │  └─────────────────────────────────────────────────────┘ │
//!  └──────────────────────────────────────────────────────────┘
//!      │                            │
//!      │ subscribe()                │ subscribe()
//!      ▼                            ▼
//!  WebSocket Broadcaster       EventAggregator (background task)
//!  (truenorth-web)             — running state: task graph, active
//!                                steps, context util, routing log
//!      │
//!      ▼
//!  Leptos Frontend (mermaid.js render)
//! ```
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`EventBus`] | Central broadcast channel + persistence |
//! | [`ReasoningEventStore`] | SQLite WAL-mode append-only event store |
//! | [`EventAggregator`] | Background task that maintains state snapshots |
//! | [`MermaidGenerator`] | Pure-function Mermaid DSL generation |
//! | [`DiagramRenderer`] | SVG/HTML wrapping of Mermaid source |
//! | [`VisualReasoningEngine`] | Convenience facade over all of the above |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use truenorth_visual::{VisualReasoningEngine, EngineConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = EngineConfig::default();
//!     let engine = VisualReasoningEngine::open(config).unwrap();
//!     let handle = engine.spawn();
//!
//!     // Obtain a dyn ReasoningEventEmitter for dependency injection.
//!     let emitter = engine.as_emitter();
//! }
//! ```

// ── Module declarations ─────────────────────────────────────────────────────

/// Persistent SQLite-backed event store.
pub mod event_store;

/// Tokio broadcast event bus with dual persistence.
pub mod event_bus;

/// Mermaid DSL diagram generation.
pub mod mermaid;

/// SVG and HTML rendering of Mermaid diagrams.
pub mod renderer;

/// Background event aggregator providing live state snapshots.
pub mod aggregator;

/// Local types: `StoredEvent`, `TaskGraphSnapshot`, `ActiveStep`, etc.
pub mod types;

// ── Re-exports ───────────────────────────────────────────────────────────────

pub use aggregator::EventAggregator;
pub use event_bus::{recv_handling_lag, EventBus, DEFAULT_CHANNEL_CAPACITY};
pub use event_store::ReasoningEventStore;
pub use mermaid::MermaidGenerator;
pub use renderer::{DiagramRenderer, RenderError};
pub use types::{
    ActiveStep, ContextUtilization, MemoryOperation, RoutingDecision, StoredEvent,
    TaskEdge, TaskGraphSnapshot, TaskNode,
};

// Re-export the core types most commonly needed alongside this crate.
pub use truenorth_core::traits::reasoning::{
    EventSubscriberHandle, ReasoningError, ReasoningEventEmitter,
};
pub use truenorth_core::types::event::{EventId, ReasoningEvent, ReasoningEventPayload};

// ── VisualReasoningEngine facade ─────────────────────────────────────────────

use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Configuration for the `VisualReasoningEngine`.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Path to the SQLite database file.
    ///
    /// If `None`, an in-memory database is used (suitable for tests).
    pub db_path: Option<PathBuf>,

    /// Broadcast channel capacity.
    ///
    /// Defaults to [`DEFAULT_CHANNEL_CAPACITY`] (1024). Increase for
    /// deployments with many slow subscribers.
    pub channel_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            db_path: None,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

impl EngineConfig {
    /// Creates a configuration that persists events to the given file path.
    pub fn with_db(path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: Some(path.into()),
            ..Default::default()
        }
    }
}

/// High-level facade that wires together the event bus, event store,
/// aggregator, and diagram renderer into a single initialisation point.
///
/// ## Lifecycle
///
/// 1. Create with `VisualReasoningEngine::open(config)`.
/// 2. Call `spawn()` to start the background aggregator task.
/// 3. Pass `as_emitter()` to agent components that need to emit events.
/// 4. Query `aggregator()` from Axum server functions for frontend data.
/// 5. On shutdown, abort the handle returned by `spawn()`.
#[derive(Debug, Clone)]
pub struct VisualReasoningEngine {
    bus: EventBus,
    aggregator: EventAggregator,
    renderer: DiagramRenderer,
}

impl VisualReasoningEngine {
    /// Opens the engine with the given configuration.
    ///
    /// Opens (or creates) the SQLite event store, constructs the event bus,
    /// and wires the aggregator. Call `spawn()` to start background processing.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` if the database cannot be
    /// opened or the schema migration fails.
    pub fn open(config: EngineConfig) -> Result<Self, ReasoningError> {
        let store = match config.db_path {
            Some(path) => ReasoningEventStore::open(path)?,
            None => ReasoningEventStore::open_in_memory()?,
        };

        let bus = EventBus::with_capacity(store, config.channel_capacity);
        let aggregator = EventAggregator::new(bus.clone());
        let renderer = DiagramRenderer::new();

        Ok(Self {
            bus,
            aggregator,
            renderer,
        })
    }

    /// Spawns the background aggregator task.
    ///
    /// Returns a `JoinHandle` that should be stored and aborted on graceful
    /// shutdown to avoid resource leaks.
    pub fn spawn(&self) -> JoinHandle<()> {
        self.aggregator.spawn()
    }

    /// Returns a reference to the `EventBus`.
    ///
    /// Used to emit events or subscribe to the live stream.
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Returns a reference to the `EventAggregator`.
    ///
    /// Used by Axum server functions to query state snapshots for the frontend.
    pub fn aggregator(&self) -> &EventAggregator {
        &self.aggregator
    }

    /// Returns a reference to the `DiagramRenderer`.
    pub fn renderer(&self) -> &DiagramRenderer {
        &self.renderer
    }

    /// Returns the engine's `EventBus` as a `Arc<dyn ReasoningEventEmitter>`.
    ///
    /// Use this for dependency injection into agent components that accept
    /// a `dyn ReasoningEventEmitter`.
    pub fn as_emitter(&self) -> Arc<dyn ReasoningEventEmitter> {
        Arc::new(self.bus.clone())
    }

    /// Emits a `ReasoningEvent` through the bus.
    ///
    /// Convenience wrapper — equivalent to `engine.bus().emit(event).await`.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` if the SQLite write fails.
    pub async fn emit(&self, event: ReasoningEvent) -> Result<(), ReasoningError> {
        self.bus.emit(event).await
    }

    /// Returns all stored events for a session, in chronological order.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub async fn replay(
        &self,
        session_id: uuid::Uuid,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError> {
        self.bus.replay(session_id, since).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::event::ReasoningEventPayload;
    use uuid::Uuid;

    #[tokio::test]
    async fn engine_open_in_memory() {
        let engine = VisualReasoningEngine::open(EngineConfig::default()).unwrap();
        let _handle = engine.spawn();

        let session_id = Uuid::new_v4();
        let event = ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::HeartbeatFired {
                registration_id: "test".to_string(),
                tick_count: 1,
                next_tick_in_secs: 60,
            },
        );

        engine.emit(event).await.unwrap();

        let events = engine.replay(session_id, None).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn engine_aggregator_wired_to_bus() {
        let engine = VisualReasoningEngine::open(EngineConfig::default()).unwrap();
        let _handle = engine.spawn();

        let session_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();

        engine
            .emit(ReasoningEvent::new(
                session_id,
                ReasoningEventPayload::StepStarted {
                    task_id: Uuid::new_v4(),
                    plan_id: Uuid::new_v4(),
                    step_id,
                    step_number: 1,
                    title: "Test step".to_string(),
                    description: "Does something".to_string(),
                },
            ))
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let steps = engine.aggregator().active_steps().await;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_id, step_id);
    }
}
