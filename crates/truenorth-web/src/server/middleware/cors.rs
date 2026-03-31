//! CORS configuration for the TrueNorth API.
//!
//! The returned layer allows any origin in development and can be tightened
//! for production deployments by specifying an explicit origin list.

use tower_http::cors::{Any, CorsLayer};

/// Build a permissive [`CorsLayer`] suitable for development.
///
/// Allows:
/// - Any origin
/// - Methods: GET, POST, PUT, DELETE, OPTIONS
/// - Headers: `Content-Type`, `Authorization`
/// - Credentials: not included (required when `allow_origin` is `Any`)
///
/// For production use, replace `Any` with `AllowOrigin::list([...])`.
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ])
}
