//! Bearer token authentication middleware.
//!
//! When [`AppState::auth_token`] is `Some`, every request must present an
//! `Authorization: Bearer <token>` header whose value matches the configured
//! token.  Requests to `/health` and `/.well-known/agent.json` are exempted
//! so that health checks and A2A discovery work without credentials.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

use crate::server::errors::ErrorBody;
use crate::server::state::AppState;

/// Paths that are always accessible without an `Authorization` header.
const EXEMPT_PATHS: &[&str] = &["/health", "/.well-known/agent.json"];

/// Axum middleware function that enforces bearer-token authentication.
///
/// # Behaviour
///
/// - If `state.auth_token` is `None` → all requests pass through (dev mode).
/// - If the request path matches an entry in [`EXEMPT_PATHS`] → pass through.
/// - Otherwise the `Authorization` header must equal `Bearer <token>`.
///   Any mismatch or absence returns HTTP 401.
pub async fn auth_layer(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    // No token configured → skip auth entirely.
    let Some(expected_token) = &state.auth_token else {
        return next.run(request).await;
    };

    // Exempt paths bypass auth.
    let path = request.uri().path();
    if EXEMPT_PATHS.iter().any(|exempt| *exempt == path) {
        return next.run(request).await;
    }

    // Extract and validate the Authorization header.
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value == format!("Bearer {expected_token}") => {
            next.run(request).await
        }
        _ => {
            let body = ErrorBody {
                error: "unauthorized".to_string(),
                message: "Missing or invalid Authorization header".to_string(),
                request_id: None,
            };
            (StatusCode::UNAUTHORIZED, Json(body)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use tower::ServiceExt;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn app_with_token(token: Option<&str>) -> Router {
        let state = if let Some(t) = token {
            AppState::builder().with_auth_token(t).build()
        } else {
            AppState::new()
        };
        Router::new()
            .route("/protected", get(ok_handler))
            .route("/health", get(ok_handler))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_layer,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn no_token_configured_allows_all() {
        let app = app_with_token(None);
        let resp = app
            .oneshot(Request::builder().uri("/protected").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn valid_bearer_token_passes() {
        let app = app_with_token(Some("secret"));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_token_returns_401() {
        let app = app_with_token(Some("secret"));
        let resp = app
            .oneshot(Request::builder().uri("/protected").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_path_exempt() {
        let app = app_with_token(Some("secret"));
        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
