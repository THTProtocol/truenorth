//! Cross-crate integration tests for truenorth-visual with core types.

use truenorth_core::types::event::{ReasoningEvent, ReasoningEventPayload};
use uuid::Uuid;

#[test]
fn test_reasoning_event_to_json() {
    let event = ReasoningEvent::new(
        Uuid::new_v4(),
        ReasoningEventPayload::TaskReceived {
            task_id: Uuid::new_v4(),
            title: "Cross-crate test".to_string(),
            description: "Verify visual crate works with core types".to_string(),
            execution_mode: "direct".to_string(),
            input_source: "test".to_string(),
        },
    );
    
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["payload"]["type"], "task_received");
}

#[test]
fn test_event_payload_variants_serialize() {
    let payloads = vec![
        ReasoningEventPayload::TaskReceived {
            task_id: Uuid::new_v4(),
            title: "t".into(),
            description: "d".into(),
            execution_mode: "direct".into(),
            input_source: "test".into(),
        },
        ReasoningEventPayload::PlanCreated {
            task_id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            step_count: 3,
            mermaid_diagram: "graph TD; A-->B".into(),
            estimated_tokens: 1000,
            estimated_duration_secs: 30,
        },
    ];
    
    for payload in payloads {
        let event = ReasoningEvent::new(Uuid::new_v4(), payload);
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.is_empty());
        // Verify roundtrip
        let _: ReasoningEvent = serde_json::from_str(&json).unwrap();
    }
}
