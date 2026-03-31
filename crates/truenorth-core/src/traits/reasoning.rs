/// ReasoningEventEmitter trait — the visual reasoning pub/sub backbone.
///
/// Every observable action in the system emits a `ReasoningEvent`.
/// The emitter publishes to a broadcast channel (for live frontend subscribers)
/// and persists to SQLite (for replay). The frontend subscribes once and
/// receives a stream of all events.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::types::event::ReasoningEvent;

/// A subscriber handle for the live reasoning event stream.
///
/// Returned by `subscribe()` for consumers to read events from.
/// Uses tokio's broadcast channel which supports multiple receivers.
pub type EventSubscriberHandle = tokio::sync::broadcast::Receiver<ReasoningEvent>;

/// Errors from the reasoning event system.
#[derive(Debug, Error)]
pub enum ReasoningError {
    /// The event bus channel is full and older events were dropped.
    #[error("Event bus is full; {dropped} events were dropped")]
    EventBusFull { dropped: usize },

    /// A subscriber's channel has fallen behind.
    #[error("Subscriber channel is lagged by {count} events")]
    SubscriberLagged { count: u64 },

    /// Failed to persist an event to the SQLite store.
    #[error("Failed to persist event to store: {message}")]
    PersistenceError { message: String },

    /// Event replay for a session failed.
    #[error("Replay failed for session {session_id}: {message}")]
    ReplayError { session_id: Uuid, message: String },

    /// The emitter is not initialized.
    #[error("Reasoning event emitter is not initialized")]
    NotInitialized,
}

/// The reasoning event emitter: the pub/sub backbone of the Visual Reasoning Layer.
///
/// Design rationale: the Visual Reasoning Layer is the core differentiator of TrueNorth.
/// Everything that happens in the system — tool calls, memory writes, routing decisions,
/// R/C/S activations — must be observable in real-time via the frontend.
/// The emitter trait ensures every module has a single, consistent way to publish events.
/// The frontend subscribes once and receives a stream of all events.
///
/// The emitter is also a persistence layer: events are stored in SQLite for replay,
/// so users can inspect past reasoning sessions in full detail.
#[async_trait]
pub trait ReasoningEventEmitter: Send + Sync + std::fmt::Debug {
    /// Emits a reasoning event.
    ///
    /// This is a fire-and-forget operation from the caller's perspective.
    /// The emitter publishes to the broadcast channel (for live subscribers)
    /// and persists to SQLite (for replay) in parallel.
    ///
    /// If the channel is full (all subscribers are slow), older events are
    /// dropped from the channel (not from storage). Live subscribers may see
    /// a gap; the stored events are always complete.
    async fn emit(&self, event: ReasoningEvent) -> Result<(), ReasoningError>;

    /// Creates a new subscriber to the live event stream.
    ///
    /// The subscriber receives all events emitted after subscription.
    /// Events emitted before subscription are not delivered via the live channel
    /// (use `replay()` for historical events).
    fn subscribe(&self) -> EventSubscriberHandle;

    /// Returns all stored events for a session, in chronological order.
    ///
    /// Used by the frontend to populate the event timeline when opening
    /// a session view. Optionally filtered to events after `since`.
    async fn replay(
        &self,
        session_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError>;

    /// Returns the latest Mermaid diagram for a task (updated by step events).
    ///
    /// The diagram is built incrementally as steps complete. This method
    /// returns the most recent version, used when a new frontend client
    /// connects mid-execution.
    async fn current_diagram(
        &self,
        task_id: Uuid,
    ) -> Result<Option<String>, ReasoningError>;

    /// Returns the number of events stored for a session.
    async fn event_count(&self, session_id: Uuid) -> Result<usize, ReasoningError>;

    /// Deletes all stored events for a session.
    ///
    /// Called when a session is permanently deleted.
    async fn delete_session_events(&self, session_id: Uuid) -> Result<(), ReasoningError>;

    /// Returns the most recent N events for a session.
    ///
    /// Used by the frontend to quickly populate the "recent events" panel
    /// without loading the entire session history.
    async fn recent_events(
        &self,
        session_id: Uuid,
        count: usize,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError>;
}
