//! Axum router — all routes registered in one place.
//!
//! [`build_router`] constructs the full [`axum::Router`] with all API routes,
//! middleware layers, and the shared [`AppState`].

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::server::{
    handlers::{
        a2a::agent_card,
        api::{
            cancel_session, get_session, health, list_sessions, list_skills, list_tools,
            search_memory, submit_task,
        },
        sse::sse_handler,
        websocket::ws_handler,
    },
    middleware::{auth::auth_layer, cors::cors_layer},
    state::AppState,
};

/// Build the complete Axum [`Router`] with all routes and middleware.
///
/// Route table:
///
/// | Method | Path | Handler | Auth required? |
/// |--------|------|---------|---------------|
/// | `GET` | `/health` | [`health`] | No |
/// | `POST` | `/api/v1/task` | [`submit_task`] | Yes (if configured) |
/// | `GET` | `/api/v1/sessions` | [`list_sessions`] | Yes |
/// | `GET` | `/api/v1/sessions/{id}` | [`get_session`] | Yes |
/// | `DELETE` | `/api/v1/sessions/{id}` | [`cancel_session`] | Yes |
/// | `GET` | `/api/v1/events/sse` | [`sse_handler`] | Yes |
/// | `GET` | `/api/v1/events/ws` | [`ws_handler`] | Yes |
/// | `GET` | `/api/v1/skills` | [`list_skills`] | Yes |
/// | `GET` | `/api/v1/tools` | [`list_tools`] | Yes |
/// | `GET` | `/api/v1/memory/search` | [`search_memory`] | Yes |
/// | `GET` | `/.well-known/agent.json` | [`agent_card`] | No |
pub fn build_router(state: AppState) -> Router {
    // Public routes — no auth middleware applied.
    let public_routes = Router::new()
        .route("/health", get(health))
        .route("/.well-known/agent.json", get(agent_card));

    // Protected API routes — wrapped with auth middleware.
    let api_routes = Router::new()
        .route("/api/v1/task", post(submit_task))
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{id}", get(get_session).delete(cancel_session))
        .route("/api/v1/events/sse", get(sse_handler))
        .route("/api/v1/events/ws", get(ws_handler))
        .route("/api/v1/skills", get(list_skills))
        .route("/api/v1/tools", get(list_tools))
        .route("/api/v1/memory/search", get(search_memory))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_layer,
        ));

    // Combine public and protected routes, then apply global middleware.
    Router::new()
        .merge(public_routes)
        .merge(api_routes)
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_returns_200() {
        let state = AppState::new();
        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn agent_card_returns_200() {
        let state = AppState::new();
        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/agent.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn sessions_endpoint_no_auth_returns_200_when_auth_disabled() {
        let state = AppState::new(); // no auth token
        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn sessions_endpoint_with_auth_requires_token() {
        let state = AppState::builder().with_auth_token("secret").build();
        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}
