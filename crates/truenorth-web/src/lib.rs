//! # truenorth-web
//!
//! Axum HTTP server with REST API, WebSocket, Server-Sent Events, and frontend
//! stubs for the TrueNorth AI orchestration harness.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │  truenorth-web                                                    │
//! │                                                                   │
//! │  ┌───────────────────────────────────────────────────────────┐   │
//! │  │  WebServer (builder façade)                                │   │
//! │  │  ┌───────────────┐  ┌────────────────────────────────┐   │   │
//! │  │  │  AppState     │  │  Router                         │   │   │
//! │  │  │  auth_token   │  │  GET  /health                   │   │   │
//! │  │  │  event bus    │  │  POST /api/v1/task              │   │   │
//! │  │  │  sessions map │  │  GET  /api/v1/sessions          │   │   │
//! │  │  └───────────────┘  │  GET  /api/v1/events/sse        │   │   │
//! │  │                     │  GET  /api/v1/events/ws         │   │   │
//! │  │  ┌───────────────┐  │  GET  /.well-known/agent.json   │   │   │
//! │  │  │  Middleware   │  └────────────────────────────────┘   │   │
//! │  │  │  Auth         │                                        │   │
//! │  │  │  CORS         │                                        │   │
//! │  │  │  Tracing      │                                        │   │
//! │  │  └───────────────┘                                        │   │
//! │  └───────────────────────────────────────────────────────────┘   │
//! │                                                                   │
//! │  ┌───────────────────────────────────────────────────────────┐   │
//! │  │  frontend (STUB — TODO: Leptos integration)               │   │
//! │  └───────────────────────────────────────────────────────────┘   │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use truenorth_web::{WebServer, AppState};
//!
//! #[tokio::main]
//! async fn main() {
//!     let state = AppState::builder()
//!         .with_auth_token("my-secret-token")
//!         .with_agent_name("MyAgent")
//!         .build();
//!
//!     WebServer::new(state)
//!         .bind("0.0.0.0:8080")
//!         .serve()
//!         .await
//!         .expect("server failed");
//! }
//! ```
//!
//! ## Crate layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`server::state`] | [`AppState`] — shared state injected into all handlers |
//! | [`server::router`] | [`build_router`] — full Axum router with all routes |
//! | [`server::middleware`] | Auth and CORS middleware layers |
//! | [`server::handlers`] | Individual route handler implementations |
//! | [`server::errors`] | [`ApiError`] — unified HTTP error type |
//! | [`frontend`] | Stub Leptos frontend (TODO: replace with real components) |

#![warn(missing_docs)]
#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod frontend;
pub mod server;

// ─── Re-exports ───────────────────────────────────────────────────────────────

pub use server::errors::ApiError;
pub use server::router::build_router;
pub use server::state::{AppState, AppStateBuilder, SessionInfo};

// ─── WebServer builder ────────────────────────────────────────────────────────

/// High-level builder for starting the TrueNorth Axum HTTP server.
///
/// Wraps [`build_router`] and [`tokio::net::TcpListener`] into a convenient
/// façade with a fluent configuration API.
///
/// # Example
///
/// ```rust,no_run
/// use truenorth_web::{WebServer, AppState};
///
/// #[tokio::main]
/// async fn main() {
///     WebServer::new(AppState::new())
///         .bind("127.0.0.1:8080")
///         .serve()
///         .await
///         .unwrap();
/// }
/// ```
pub struct WebServer {
    state: AppState,
    bind_addr: String,
}

impl WebServer {
    /// Create a new `WebServer` with the given [`AppState`].
    ///
    /// Defaults to binding on `0.0.0.0:8080`.
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            bind_addr: "0.0.0.0:8080".to_string(),
        }
    }

    /// Override the TCP socket address to bind on.
    ///
    /// Accepts any string accepted by [`tokio::net::TcpListener::bind`],
    /// e.g. `"127.0.0.1:3000"` or `"[::]:8080"`.
    pub fn bind(mut self, addr: impl Into<String>) -> Self {
        self.bind_addr = addr.into();
        self
    }

    /// Build the router and start serving requests.
    ///
    /// This function blocks indefinitely (or until the process is killed).
    ///
    /// # Errors
    ///
    /// Returns an error if the TCP listener cannot be bound (e.g., port in use
    /// or insufficient permissions) or if the Axum server fails during operation.
    pub async fn serve(self) -> anyhow::Result<()> {
        let router = build_router(self.state);
        let listener = tokio::net::TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("TrueNorth web server listening on {}", self.bind_addr);
        axum::serve(listener, router).await?;
        Ok(())
    }
}

// ─── Public type re-exports from truenorth-visual ────────────────────────────
// These are the types most commonly needed alongside the web server.

pub use truenorth_visual::{EngineConfig, VisualReasoningEngine};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_server_default_bind_addr() {
        let server = WebServer::new(AppState::new());
        assert_eq!(server.bind_addr, "0.0.0.0:8080");
    }

    #[test]
    fn web_server_custom_bind_addr() {
        let server = WebServer::new(AppState::new()).bind("127.0.0.1:3000");
        assert_eq!(server.bind_addr, "127.0.0.1:3000");
    }

    #[test]
    fn app_state_is_clone() {
        let state = AppState::new();
        let _clone = state.clone();
    }

    #[test]
    fn build_router_does_not_panic() {
        let state = AppState::new();
        let _router = build_router(state);
    }
}
