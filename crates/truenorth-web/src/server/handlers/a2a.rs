//! A2A (Agent-to-Agent) Agent Card endpoint.
//!
//! Implements the `/.well-known/agent.json` discovery endpoint specified by the
//! Agent-to-Agent (A2A) protocol.  Clients (other agents or orchestration
//! frameworks) fetch this URL to discover the capabilities and API surface of
//! this TrueNorth instance.
//!
//! This endpoint is always accessible without authentication.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use crate::server::state::AppState;

/// The complete A2A Agent Card document.
///
/// Structure follows the informal A2A Agent Card specification used by Google's
/// Agent2Agent protocol draft.  All fields are serialised to camelCase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Schema version for the Agent Card format.
    pub schema_version: String,
    /// The agent's human-readable name.
    pub name: String,
    /// A short description of the agent's purpose.
    pub description: String,
    /// Semantic version of the TrueNorth API.
    pub api_version: String,
    /// List of skills this agent supports.
    pub skills: Vec<AgentSkillEntry>,
    /// Supported interaction protocols.
    pub protocols: Vec<String>,
    /// Base URL of this agent's REST API.
    pub api_base_url: Option<String>,
    /// Additional agent metadata.
    pub metadata: serde_json::Value,
}

/// Compact description of a skill exposed in the Agent Card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkillEntry {
    /// Skill name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Trigger phrases that activate this skill.
    pub triggers: Vec<String>,
}

/// `GET /.well-known/agent.json` — Serve the A2A Agent Card.
///
/// Returns a JSON document describing this TrueNorth instance's identity,
/// capabilities, and API surface.  This endpoint is exempt from authentication
/// so that external agents can perform unauthenticated discovery.
pub async fn agent_card(State(state): State<AppState>) -> impl IntoResponse {
    let card = build_agent_card(&state);
    Json(card)
}

/// Construct the [`AgentCard`] from the current [`AppState`].
///
/// Exported as a public function so the card can be unit-tested without
/// spinning up a full HTTP server.
pub fn build_agent_card(state: &AppState) -> AgentCard {
    AgentCard {
        schema_version: "1.0".to_string(),
        name: state.agent_name.clone(),
        description: state.agent_description.clone(),
        api_version: state.api_version.clone(),
        skills: vec![
            AgentSkillEntry {
                name: "task_execution".to_string(),
                description: "Submit a task and receive streamed agent responses".to_string(),
                triggers: vec!["run".to_string(), "execute".to_string(), "do".to_string()],
            },
            AgentSkillEntry {
                name: "memory_search".to_string(),
                description: "Search the agent's three-tier memory layer".to_string(),
                triggers: vec!["recall".to_string(), "remember".to_string(), "search".to_string()],
            },
            AgentSkillEntry {
                name: "session_management".to_string(),
                description: "List, inspect, and cancel active agent sessions".to_string(),
                triggers: vec!["sessions".to_string(), "status".to_string()],
            },
        ],
        protocols: vec!["rest".to_string(), "sse".to_string(), "websocket".to_string()],
        api_base_url: None,
        metadata: serde_json::json!({
            "provider": "TrueNorth",
            "open_source": true,
            "repository": "https://github.com/THTProtocol/truenorth",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_card_serialises_to_camel_case() {
        let state = AppState::new();
        let card = build_agent_card(&state);
        let json = serde_json::to_value(&card).unwrap();
        // Verify camelCase field names.
        assert!(json.get("schemaVersion").is_some());
        assert!(json.get("apiVersion").is_some());
        assert!(json.get("apiBaseUrl").is_some());
    }

    #[test]
    fn agent_card_has_required_fields() {
        let state = AppState::builder()
            .with_agent_name("TestAgent")
            .with_api_version("2.0.0")
            .build();
        let card = build_agent_card(&state);
        assert_eq!(card.name, "TestAgent");
        assert_eq!(card.api_version, "2.0.0");
        assert!(!card.skills.is_empty());
        assert!(!card.protocols.is_empty());
    }
}
