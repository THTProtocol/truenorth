//! Integration smoke tests for TrueNorth core types.
//!
//! Verify that all primary types instantiate, serialize, and roundtrip correctly.

use truenorth_core::types::memory::{MemoryEntry, MemoryScope};
use truenorth_core::types::llm::{CompletionRequest, CompletionParameters};
use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use truenorth_core::error::LlmError;
use uuid::Uuid;

#[test]
fn test_core_types_instantiate() {
    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        scope: MemoryScope::Session,
        content: "test entry".to_string(),
        metadata: Default::default(),
        embedding: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        importance: 0.5,
        retrieval_count: 0,
    };
    assert_eq!(entry.scope, MemoryScope::Session);
}

#[test]
fn test_completion_request_serialization() {
    let request = CompletionRequest {
        request_id: Uuid::new_v4(),
        messages: vec![],
        parameters: CompletionParameters::default(),
        tools: None,
        session_id: Uuid::new_v4(),
        stream: false,
        required_capabilities: vec![],
    };
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("request_id"));
}

#[test]
fn test_error_types() {
    let err = LlmError::RateLimited {
        provider: "openai".to_string(),
        retry_after_secs: 30,
    };
    let msg = format!("{}", err);
    assert!(msg.contains("openai"));
}

#[test]
fn test_reasoning_event_serialization() {
    let event = ReasoningEvent::new(
        Uuid::new_v4(),
        ReasoningEventPayload::TaskReceived {
            task_id: Uuid::new_v4(),
            title: "Test task".to_string(),
            description: "A smoke test".to_string(),
            execution_mode: "direct".to_string(),
            input_source: "test".to_string(),
        },
    );
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("task_received"));
}

#[test]
fn test_memory_scope_variants() {
    let scopes = vec![MemoryScope::Session, MemoryScope::Project, MemoryScope::Identity];
    for scope in scopes {
        let json = serde_json::to_string(&scope).unwrap();
        let roundtrip: MemoryScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, roundtrip);
    }
}
