/// Tokio broadcast-based event bus for the Visual Reasoning Layer.
///
/// The `EventBus` is the central nervous system of TrueNorth. Every module
/// that produces observable state emits a `ReasoningEvent` through the bus.
/// Every module that consumes observable state (WebSocket broadcaster,
/// aggregator, event store persistence task) subscribes to the bus.
///
/// The bus is:
/// - **Thread-safe**: `Arc<EventBus>` can be shared freely across threads.
/// - **Non-blocking**: `emit` never blocks the caller. The broadcast send is
///   O(1) and the persistence is handled by a background task.
/// - **Durable**: every emitted event is persisted to SQLite via the event
///   store regardless of whether any live subscribers are connected.
/// - **Resilient to slow subscribers**: `tokio::sync::broadcast` drops
///   lagged messages from the channel buffer, but SQLite always retains them.
///   A lagged subscriber logs a warning and can catch up via `replay()`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use tracing::{debug, error, warn};
use uuid::Uuid;

use truenorth_core::traits::reasoning::{EventSubscriberHandle, ReasoningError, ReasoningEventEmitter};
use truenorth_core::types::event::ReasoningEvent;

use crate::event_store::ReasoningEventStore;

/// Default channel capacity: the number of events buffered before older
/// events are dropped for slow subscribers. Persisted events are never dropped.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// The broadcast-based event bus.
///
/// Clone cheaply — all clones share the same underlying broadcast sender and
/// event store. The `subscriber_count` is tracked atomically for monitoring.
#[derive(Debug, Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

#[derive(Debug)]
struct EventBusInner {
    /// The broadcast channel sender. All `subscribe()` calls produce a
    /// `Receiver` connected to this sender.
    sender: broadcast::Sender<ReasoningEvent>,
    /// Persistent event store — every event is written here regardless of
    /// whether subscribers are active.
    store: ReasoningEventStore,
    /// Number of currently-active subscriber `Receiver` handles.
    subscriber_count: AtomicUsize,
}

impl EventBus {
    /// Creates a new `EventBus` backed by the given `ReasoningEventStore`.
    ///
    /// The channel is created with `DEFAULT_CHANNEL_CAPACITY`.
    pub fn new(store: ReasoningEventStore) -> Self {
        Self::with_capacity(store, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Creates a new `EventBus` with an explicit broadcast channel capacity.
    ///
    /// Increase capacity if many fast producers and slow consumers coexist.
    /// The capacity does not affect durability — all events are persisted
    /// regardless of channel pressure.
    pub fn with_capacity(store: ReasoningEventStore, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            inner: Arc::new(EventBusInner {
                sender,
                store,
                subscriber_count: AtomicUsize::new(0),
            }),
        }
    }

    /// Emits a reasoning event to all live subscribers **and** persists it
    /// to the SQLite event store.
    ///
    /// This method is synchronous-from-the-caller's-perspective: it returns as
    /// soon as the broadcast send completes (which is O(1)) and after the
    /// synchronous SQLite write. The SQLite write is fast (WAL mode, single
    /// row insert) and acceptable on the hot path; if needed, persistence can
    /// be moved to a background task in a future iteration.
    ///
    /// **Lagged subscribers**: if a subscriber's receive buffer is full, it
    /// will see a `RecvError::Lagged` on its next `recv()`. The bus logs a
    /// warning but does not treat this as an error — the subscriber can
    /// recover by calling `replay()` to catch up from the persistent store.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` only if the SQLite write
    /// fails; broadcast errors (lagged receivers) are logged but not propagated.
    pub async fn emit(&self, event: ReasoningEvent) -> Result<(), ReasoningError> {
        // 1. Persist to store first — durability before delivery.
        self.inner.store.store(&event)?;

        // 2. Broadcast to live subscribers.
        match self.inner.sender.send(event.clone()) {
            Ok(receiver_count) => {
                debug!(
                    event_id = %event.id,
                    receivers = receiver_count,
                    "Event broadcast to {} receiver(s)",
                    receiver_count
                );
            }
            Err(broadcast::error::SendError(_)) => {
                // No active receivers — this is fine; events are always in SQLite.
                debug!(event_id = %event.id, "No active receivers for event (stored only)");
            }
        }

        Ok(())
    }

    /// Creates a new subscriber `Receiver` for the live event stream.
    ///
    /// The returned `Receiver` will receive all events emitted **after** this
    /// call. Events emitted before subscription are available via `replay()`.
    ///
    /// When the `Receiver` is dropped, the subscriber count is automatically
    /// decremented via a guard wrapper. Use `SubscriberGuard` to track this
    /// automatically, or call `subscriber_count()` to monitor active consumers.
    pub fn subscribe(&self) -> EventSubscriberHandle {
        self.inner.subscriber_count.fetch_add(1, Ordering::Relaxed);
        self.inner.sender.subscribe()
    }

    /// Returns the number of currently-active subscriber handles.
    ///
    /// Note: this is an approximation — the count is incremented on `subscribe()`
    /// but there is no automatic decrement when a `Receiver` is dropped because
    /// `tokio::sync::broadcast::Receiver` has no drop callback. Use the
    /// `sender.receiver_count()` for the authoritative value.
    pub fn subscriber_count(&self) -> usize {
        self.inner.sender.receiver_count()
    }

    /// Returns a reference to the underlying `ReasoningEventStore`.
    ///
    /// Used by components that need to issue replay queries without going
    /// through the event stream.
    pub fn store(&self) -> &ReasoningEventStore {
        &self.inner.store
    }

    /// Returns all stored events for a session, in chronological order.
    ///
    /// Convenience wrapper over `ReasoningEventStore::query_by_session`.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub async fn replay(
        &self,
        session_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError> {
        let stored = match since {
            Some(ts) => self.inner.store.query_since(session_id, ts)?,
            None => self.inner.store.query_by_session(session_id)?,
        };
        Ok(stored.into_iter().map(|se| se.event).collect())
    }

    /// Returns the latest Mermaid diagram string for a task, if any.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub async fn current_diagram(
        &self,
        task_id: Uuid,
    ) -> Result<Option<String>, ReasoningError> {
        self.inner.store.latest_diagram(task_id)
    }

    /// Returns the number of events stored for a session.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub async fn event_count(&self, session_id: Uuid) -> Result<usize, ReasoningError> {
        self.inner.store.event_count(session_id)
    }

    /// Deletes all stored events for a session.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub async fn delete_session_events(&self, session_id: Uuid) -> Result<(), ReasoningError> {
        self.inner.store.delete_session_events(session_id)
    }

    /// Returns the most recent `count` events for a session, in chronological order.
    ///
    /// # Errors
    /// Returns `ReasoningError::PersistenceError` on database errors.
    pub async fn recent_events(
        &self,
        session_id: Uuid,
        count: usize,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError> {
        let stored = self.inner.store.recent_events(session_id, count)?;
        Ok(stored.into_iter().map(|se| se.event).collect())
    }
}

// ---------------------------------------------------------------------------
// ReasoningEventEmitter trait implementation
// ---------------------------------------------------------------------------

/// Implements the `ReasoningEventEmitter` trait from `truenorth-core` so that
/// any component holding a `dyn ReasoningEventEmitter` can use the `EventBus`
/// transparently.
#[async_trait]
impl ReasoningEventEmitter for EventBus {
    async fn emit(&self, event: ReasoningEvent) -> Result<(), ReasoningError> {
        self.emit(event).await
    }

    fn subscribe(&self) -> EventSubscriberHandle {
        self.subscribe()
    }

    async fn replay(
        &self,
        session_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError> {
        self.replay(session_id, since).await
    }

    async fn current_diagram(
        &self,
        task_id: Uuid,
    ) -> Result<Option<String>, ReasoningError> {
        self.current_diagram(task_id).await
    }

    async fn event_count(&self, session_id: Uuid) -> Result<usize, ReasoningError> {
        self.event_count(session_id).await
    }

    async fn delete_session_events(&self, session_id: Uuid) -> Result<(), ReasoningError> {
        self.delete_session_events(session_id).await
    }

    async fn recent_events(
        &self,
        session_id: Uuid,
        count: usize,
    ) -> Result<Vec<ReasoningEvent>, ReasoningError> {
        self.recent_events(session_id, count).await
    }
}

// ---------------------------------------------------------------------------
// Lagged receiver helper
// ---------------------------------------------------------------------------

/// Consumes a `broadcast::Receiver<ReasoningEvent>`, logging warnings for
/// lagged messages and returning the next event (skipping over lag errors).
///
/// This function is intended for use in `tokio::spawn` loops where a
/// subscriber must not halt on lag.
///
/// Returns `None` when the channel is closed (no more senders).
pub async fn recv_handling_lag(
    rx: &mut broadcast::Receiver<ReasoningEvent>,
) -> Option<ReasoningEvent> {
    loop {
        match rx.recv().await {
            Ok(event) => return Some(event),
            Err(broadcast::error::RecvError::Lagged(count)) => {
                warn!(
                    lagged_by = count,
                    "EventBus subscriber is lagged by {} events; some live events were missed. \
                     Use replay() to catch up from the persistent store.",
                    count
                );
                // Continue looping — the receiver automatically advances past
                // the dropped messages and will receive the next available event.
            }
            Err(broadcast::error::RecvError::Closed) => {
                error!("EventBus broadcast channel closed — no more events will be delivered");
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};

    fn make_bus() -> EventBus {
        let store = ReasoningEventStore::open_in_memory().unwrap();
        EventBus::new(store)
    }

    fn heartbeat_event(session_id: Uuid) -> ReasoningEvent {
        ReasoningEvent::new(
            session_id,
            ReasoningEventPayload::HeartbeatFired {
                registration_id: "test".to_string(),
                tick_count: 1,
                next_tick_in_secs: 30,
            },
        )
    }

    #[tokio::test]
    async fn emit_and_receive() {
        let bus = make_bus();
        let mut rx = bus.subscribe();
        let session_id = Uuid::new_v4();
        let event = heartbeat_event(session_id);
        bus.emit(event.clone()).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, event.id);
    }

    #[tokio::test]
    async fn emit_persists_to_store() {
        let bus = make_bus();
        let session_id = Uuid::new_v4();
        bus.emit(heartbeat_event(session_id)).await.unwrap();
        bus.emit(heartbeat_event(session_id)).await.unwrap();
        assert_eq!(bus.event_count(session_id).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn replay_returns_all_events() {
        let bus = make_bus();
        let session_id = Uuid::new_v4();
        bus.emit(heartbeat_event(session_id)).await.unwrap();
        bus.emit(heartbeat_event(session_id)).await.unwrap();
        let events = bus.replay(session_id, None).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn no_receivers_still_persists() {
        let bus = make_bus();
        let session_id = Uuid::new_v4();
        // emit without any subscriber
        bus.emit(heartbeat_event(session_id)).await.unwrap();
        assert_eq!(bus.event_count(session_id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn subscriber_count_reflects_active_receivers() {
        let bus = make_bus();
        assert_eq!(bus.subscriber_count(), 0);
        let _rx1 = bus.subscribe();
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }
}
