//! REST API handlers for the TrueNorth web server.
//!
//! Provides endpoints for task submission, session management, skill listing,
//! tool listing, and memory search.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::server::errors::ApiError;
use crate::server::state::{AppState, SessionInfo};

// ── Health ────────────────────────────────────────────────────────────────────

/// Response body for `GET /health`.
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Always `"ok"` when the server is reachable.
    pub status: &'static str,
    /// API semantic version.
    pub version: String,
    /// Server UTC timestamp at the time of the request.
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// `GET /health` — Liveness probe.
///
/// Returns HTTP 200 with a JSON body indicating the server is running.
/// This endpoint is **not** protected by bearer-token auth.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        version: state.api_version.clone(),
        timestamp: Utc::now(),
    })
}

// ── Task submission ───────────────────────────────────────────────────────────

/// Request body for `POST /api/v1/task`.
#[derive(Debug, Deserialize)]
pub struct SubmitTaskRequest {
    /// Short title for the task.
    pub title: String,
    /// Full description of what the agent should accomplish.
    pub description: String,
    /// Optional hard constraints the agent must respect.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Optional execution mode override (e.g., `"direct"`, `"sequential"`).
    pub execution_mode: Option<String>,
}

/// Response body for `POST /api/v1/task`.
#[derive(Debug, Serialize)]
pub struct SubmitTaskResponse {
    /// The newly created task identifier.
    pub task_id: Uuid,
    /// The session that will execute this task.
    pub session_id: Uuid,
    /// Human-readable status message.
    pub message: String,
}

/// `POST /api/v1/task` — Submit a new task to the agent.
///
/// Creates a new session and enqueues the task for execution.
/// Returns the `task_id` and `session_id` so the caller can track progress
/// via the SSE or WebSocket streams.
///
/// # Errors
///
/// Returns HTTP 400 if the request body is missing required fields.
pub async fn submit_task(
    State(state): State<AppState>,
    Json(req): Json<SubmitTaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.title.is_empty() {
        return Err(ApiError::bad_request("title must not be empty"));
    }
    if req.description.is_empty() {
        return Err(ApiError::bad_request("description must not be empty"));
    }

    let task_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let now = Utc::now();

    // Register the session in active_sessions so it appears in listings.
    let info = SessionInfo {
        session_id,
        title: req.title.clone(),
        agent_state: "Pending".to_string(),
        created_at: now,
        updated_at: now,
        context_tokens: 0,
    };
    state.active_sessions.write().await.insert(session_id, info);

    tracing::info!(
        task_id = %task_id,
        session_id = %session_id,
        title = %req.title,
        "Task submitted"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(SubmitTaskResponse {
            task_id,
            session_id,
            message: format!("Task '{}' accepted. Session: {session_id}", req.title),
        }),
    ))
}

// ── Session management ────────────────────────────────────────────────────────

/// Response body for `GET /api/v1/sessions`.
#[derive(Debug, Serialize)]
pub struct SessionListResponse {
    /// All currently active sessions.
    pub sessions: Vec<SessionInfo>,
    /// Total number of active sessions.
    pub total: usize,
}

/// `GET /api/v1/sessions` — List all active sessions.
///
/// Returns a snapshot of all sessions currently tracked in `AppState`.
pub async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let sessions: Vec<SessionInfo> = state
        .active_sessions
        .read()
        .await
        .values()
        .cloned()
        .collect();
    let total = sessions.len();
    Json(SessionListResponse { sessions, total })
}

/// `GET /api/v1/sessions/:id` — Get details for a single session.
///
/// # Errors
///
/// Returns HTTP 404 if the session is not found.
pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let sessions = state.active_sessions.read().await;
    let info = sessions
        .get(&session_id)
        .cloned()
        .ok_or(ApiError::SessionNotFound { session_id })?;
    Ok(Json(info))
}

/// `DELETE /api/v1/sessions/:id` — Cancel and remove a session.
///
/// Removes the session from the active map and broadcasts a cancellation event.
///
/// # Errors
///
/// Returns HTTP 404 if the session is not found.
pub async fn cancel_session(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let removed = state
        .active_sessions
        .write()
        .await
        .remove(&session_id)
        .ok_or(ApiError::SessionNotFound { session_id })?;

    // Broadcast a cancellation event so WebSocket subscribers are notified.
    let event = serde_json::json!({
        "type": "session_cancelled",
        "session_id": session_id,
        "title": removed.title,
    });
    // Ignore send errors — no active subscribers is fine.
    let _ = state.visual_event_tx.send(event);

    tracing::info!(session_id = %session_id, "Session cancelled");

    Ok(StatusCode::NO_CONTENT)
}

// ── Skills ────────────────────────────────────────────────────────────────────

/// A single skill summary returned by `GET /api/v1/skills`.
#[derive(Debug, Serialize)]
pub struct SkillSummary {
    /// Skill name from SKILL.md frontmatter.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Semantic version string.
    pub version: String,
    /// Tags from SKILL.md frontmatter.
    pub tags: Vec<String>,
}

/// Response body for `GET /api/v1/skills`.
#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    /// All discovered skills.
    pub skills: Vec<SkillSummary>,
    /// Total count.
    pub total: usize,
}

/// `GET /api/v1/skills` — List installed skills.
///
/// Returns metadata for all skills discovered from the filesystem.
/// The actual skill loader is not yet wired; this returns an empty list.
///
/// # TODO
///
/// Wire this to `truenorth_skills::SkillRegistry` once the orchestrator
/// is available in `AppState`.
pub async fn list_skills(_state: State<AppState>) -> impl IntoResponse {
    // TODO: wire to AppState::skill_registry once available
    Json(SkillListResponse {
        skills: vec![],
        total: 0,
    })
}

// ── Tools ─────────────────────────────────────────────────────────────────────

/// A single tool summary returned by `GET /api/v1/tools`.
#[derive(Debug, Serialize)]
pub struct ToolSummary {
    /// Tool name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Permission level required to call this tool.
    pub permission_level: String,
}

/// Response body for `GET /api/v1/tools`.
#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    /// All registered tools.
    pub tools: Vec<ToolSummary>,
    /// Total count.
    pub total: usize,
}

/// `GET /api/v1/tools` — List registered tools.
///
/// Returns metadata for all tools registered in the tool registry.
///
/// # TODO
///
/// Wire this to `truenorth_tools::DefaultToolRegistry` once the orchestrator
/// is available in `AppState`.
pub async fn list_tools(_state: State<AppState>) -> impl IntoResponse {
    // TODO: wire to AppState::tool_registry once available
    Json(ToolListResponse {
        tools: vec![],
        total: 0,
    })
}

// ── Memory search ─────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/v1/memory/search`.
#[derive(Debug, Deserialize)]
pub struct MemorySearchQuery {
    /// The search query string.
    pub q: String,
    /// Memory scope to search in. Defaults to `"session"`.
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Maximum number of results to return. Defaults to 10.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_scope() -> String {
    "session".to_string()
}

fn default_limit() -> usize {
    10
}

/// A single memory search result.
#[derive(Debug, Serialize)]
pub struct MemorySearchResult {
    /// Memory entry identifier.
    pub id: Uuid,
    /// Excerpt of the matched content.
    pub content_excerpt: String,
    /// Relevance score (higher is better).
    pub score: f32,
    /// Memory scope the entry belongs to.
    pub scope: String,
}

/// Response body for `GET /api/v1/memory/search`.
#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    /// The original query string.
    pub query: String,
    /// Matched memory entries ordered by relevance.
    pub results: Vec<MemorySearchResult>,
    /// Total number of results.
    pub total: usize,
}

/// `GET /api/v1/memory/search` — Search memory entries.
///
/// Performs a text search across memory entries in the specified scope.
///
/// # Errors
///
/// Returns HTTP 400 if the `q` parameter is empty.
///
/// # TODO
///
/// Wire this to `truenorth_memory::MemoryLayer` once it is part of `AppState`.
pub async fn search_memory(
    _state: State<AppState>,
    Query(params): Query<MemorySearchQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if params.q.is_empty() {
        return Err(ApiError::bad_request("query parameter 'q' must not be empty"));
    }

    // TODO: wire to AppState::memory_layer once available
    Ok(Json(MemorySearchResponse {
        query: params.q,
        results: vec![],
        total: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_serialises() {
        let resp = HealthResponse {
            status: "ok",
            version: "1.0.0".to_string(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
    }

    #[test]
    fn submit_task_request_deserialises() {
        let json = r#"{"title":"test","description":"do something"}"#;
        let req: SubmitTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, "test");
        assert!(req.constraints.is_empty());
    }
}
