//! WebSocket handler for the Visual Reasoning event stream.
//!
//! Clients connect to `GET /api/v1/events/ws` and receive a continuous stream
//! of JSON-encoded visual reasoning events pushed from the broadcast channel.
//!
//! The connection stays open until either side closes it.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};

use crate::server::state::AppState;

/// `GET /api/v1/events/ws` — Upgrade to a WebSocket connection.
///
/// After the HTTP handshake the server subscribes to the visual reasoning
/// broadcast channel and forwards every event as a JSON text frame.
///
/// Messages sent from the client are currently ignored (read-only stream).
///
/// # Client usage
///
/// ```js
/// const ws = new WebSocket('ws://localhost:8080/api/v1/events/ws');
/// ws.onmessage = (e) => {
///   const event = JSON.parse(e.data);
///   console.log(event);
/// };
/// ```
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Drive the WebSocket connection after the upgrade.
///
/// Subscribes to the event broadcast channel and forwards events as text
/// frames.  Exits when the client disconnects or the channel is closed.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to the broadcast channel.
    let mut rx = state.visual_event_tx.subscribe();

    // Send a "connected" handshake so the client knows it's live.
    let handshake = serde_json::json!({
        "type": "connected",
        "agent": state.agent_name,
        "message": "WebSocket stream connected",
    });
    if sender
        .send(Message::Text(handshake.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    // Forward broadcast events to the WebSocket client.
    loop {
        tokio::select! {
            // New event from the broadcast channel.
            result = rx.recv() => {
                match result {
                    Ok(value) => {
                        let text = value.to_string();
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            // Client disconnected.
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WebSocket subscriber lagged by {n} events");
                        // Continue — do not disconnect on lag.
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Broadcast channel shut down.
                        break;
                    }
                }
            }

            // Incoming message from the client.
            maybe_msg = receiver.next() => {
                match maybe_msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to ping frames to keep the connection alive.
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    // All other client messages are ignored.
                    _ => {}
                }
            }
        }
    }

    tracing::debug!("WebSocket connection closed");
}
