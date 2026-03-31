//! Server-Sent Events (SSE) handler for LLM response streaming.
//!
//! Clients connect to `GET /api/v1/events/sse` and receive a continuous stream
//! of SSE events sourced from the visual reasoning event broadcast channel.
//!
//! The stream stays open until the client disconnects.  Events are forwarded as
//! JSON-encoded SSE `data:` fields.

use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::server::state::AppState;

/// `GET /api/v1/events/sse` — Subscribe to the live LLM event stream.
///
/// Opens a Server-Sent Events connection.  Each event is JSON-encoded and
/// sent as a `data:` SSE field with event type `"reasoning_event"`.
///
/// Keep-alive pings are sent every 15 seconds to keep the TCP connection open
/// through proxies and load balancers.
///
/// # Client usage
///
/// ```js
/// const es = new EventSource('/api/v1/events/sse', {
///   headers: { Authorization: 'Bearer <token>' }
/// });
/// es.addEventListener('reasoning_event', (e) => {
///   const event = JSON.parse(e.data);
///   console.log(event);
/// });
/// ```
pub async fn sse_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Subscribe to the broadcast channel before building the stream so we
    // don't miss events that arrive between the subscribe call and the first poll.
    let rx = state.visual_event_tx.subscribe();

    let event_stream = broadcast_to_sse_stream(rx);

    // Apply a keep-alive ping so the connection doesn't time out behind proxies.
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("ping");

    Sse::new(event_stream).keep_alive(keep_alive)
}

/// Convert a broadcast receiver into an SSE event stream using `futures::stream::unfold`.
///
/// Consumes lagged errors silently (logs a warning) and terminates the stream
/// when the channel is closed.
fn broadcast_to_sse_stream(
    rx: broadcast::Receiver<serde_json::Value>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(value) => {
                    let data = value.to_string();
                    let event = Event::default()
                        .event("reasoning_event")
                        .data(data);
                    return Some((Ok(event), rx));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        "SSE subscriber lagged by {n} events; some events were dropped"
                    );
                    // Continue looping — don't disconnect on lag.
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // Broadcast channel shut down — end the stream.
                    return None;
                }
            }
        }
    })
}

/// Construct a "connected" handshake event for a new SSE subscriber.
///
/// This helper is used in tests to verify the initial event shape.
pub fn connected_event(agent_name: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "connected",
        "agent": agent_name,
        "message": "SSE stream connected",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_event_has_required_fields() {
        let ev = connected_event("TrueNorth");
        assert_eq!(ev["type"], "connected");
        assert_eq!(ev["agent"], "TrueNorth");
    }
}
