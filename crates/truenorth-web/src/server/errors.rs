//! HTTP error response formatting for the TrueNorth API.
//!
//! [`ApiError`] is the single error type returned from all Axum handlers.
//! It implements [`axum::response::IntoResponse`] so it can be returned
//! directly from handler functions.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Structured API error body returned as JSON.
///
/// All error responses use this shape so that API clients can reliably parse them.
///
/// ```json
/// {
///   "error": "not_found",
///   "message": "Session 550e8400-... not found",
///   "request_id": "abc123"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Machine-readable error code (snake_case).
    pub error: String,
    /// Human-readable description of the error.
    pub message: String,
    /// Optional trace identifier for correlating server-side logs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// All possible API errors that the TrueNorth web server can return.
///
/// Each variant maps to a specific HTTP status code via its [`IntoResponse`]
/// implementation.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The requested resource was not found (HTTP 404).
    #[error("not found: {message}")]
    NotFound {
        /// Human-readable description of which resource was not found.
        message: String,
    },

    /// The request is malformed or missing required fields (HTTP 400).
    #[error("bad request: {message}")]
    BadRequest {
        /// Description of the validation failure.
        message: String,
    },

    /// The client is not authenticated (HTTP 401).
    #[error("unauthorized: {message}")]
    Unauthorized {
        /// Description of the authentication failure.
        message: String,
    },

    /// The client lacks permission for this resource (HTTP 403).
    #[error("forbidden: {message}")]
    Forbidden {
        /// Description of the authorisation failure.
        message: String,
    },

    /// A requested session was not found by UUID (HTTP 404).
    #[error("session {session_id} not found")]
    SessionNotFound {
        /// The session UUID that was not found.
        session_id: Uuid,
    },

    /// An internal server error occurred (HTTP 500).
    #[error("internal server error: {message}")]
    Internal {
        /// Description of the internal error (safe to expose to the API caller).
        message: String,
    },

    /// The service is temporarily unavailable (HTTP 503).
    #[error("service unavailable: {message}")]
    ServiceUnavailable {
        /// Description of why the service is unavailable.
        message: String,
    },
}

impl ApiError {
    /// Construct a `NotFound` error with the given message.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound { message: message.into() }
    }

    /// Construct a `BadRequest` error with the given message.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest { message: message.into() }
    }

    /// Construct an `Unauthorized` error with the given message.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized { message: message.into() }
    }

    /// Construct an `Internal` error with the given message.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal { message: message.into() }
    }

    /// Map the error variant to its HTTP status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ApiError::NotFound { .. } | ApiError::SessionNotFound { .. } => StatusCode::NOT_FOUND,
            ApiError::BadRequest { .. } => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized { .. } => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden { .. } => StatusCode::FORBIDDEN,
            ApiError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::ServiceUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Machine-readable error code for the JSON body.
    pub fn error_code(&self) -> &'static str {
        match self {
            ApiError::NotFound { .. } | ApiError::SessionNotFound { .. } => "not_found",
            ApiError::BadRequest { .. } => "bad_request",
            ApiError::Unauthorized { .. } => "unauthorized",
            ApiError::Forbidden { .. } => "forbidden",
            ApiError::Internal { .. } => "internal_server_error",
            ApiError::ServiceUnavailable { .. } => "service_unavailable",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = ErrorBody {
            error: self.error_code().to_string(),
            message: self.to_string(),
            request_id: None,
        };
        (status, Json(body)).into_response()
    }
}

/// Convenience conversion from `anyhow::Error` to `ApiError::Internal`.
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal { message: e.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_are_correct() {
        assert_eq!(ApiError::not_found("x").status_code(), StatusCode::NOT_FOUND);
        assert_eq!(ApiError::bad_request("x").status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(ApiError::unauthorized("x").status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(ApiError::internal("x").status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn session_not_found_uses_not_found_status() {
        let id = Uuid::new_v4();
        assert_eq!(
            ApiError::SessionNotFound { session_id: id }.status_code(),
            StatusCode::NOT_FOUND
        );
    }
}
