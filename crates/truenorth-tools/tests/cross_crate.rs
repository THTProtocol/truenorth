//! Cross-crate integration tests verifying truenorth-tools compiles 
//! and works correctly with truenorth-core types.

use truenorth_core::types::tool::{PermissionLevel, ToolResult, ToolCall, ToolSchema, SideEffect};

#[test]
fn test_permission_level_serialization() {
    let perms = vec![
        PermissionLevel::Low,
        PermissionLevel::Medium,
        PermissionLevel::High,
    ];
    for perm in &perms {
        let json = serde_json::to_string(perm).unwrap();
        let roundtrip: PermissionLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(*perm, roundtrip);
    }
}

#[test]
fn test_permission_ordering() {
    assert!(PermissionLevel::Low < PermissionLevel::Medium);
    assert!(PermissionLevel::Medium < PermissionLevel::High);
}

#[test]
fn test_tool_result_roundtrip() {
    let result = ToolResult {
        llm_output: serde_json::json!({"files": ["README.md"]}),
        display_output: Some(serde_json::json!({"text": "Found 1 file"})),
        side_effects: vec![
            SideEffect::FileRead { path: "README.md".into(), bytes: 1024 },
        ],
        execution_ms: 42,
    };
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ToolResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.execution_ms, 42);
    assert_eq!(parsed.side_effects.len(), 1);
}

#[test]
fn test_tool_call_creation() {
    let call = ToolCall {
        call_id: "call_123".to_string(),
        name: "web_search".to_string(),
        arguments: serde_json::json!({"query": "rust async"}),
    };
    let json = serde_json::to_string(&call).unwrap();
    assert!(json.contains("web_search"));
}

#[test]
fn test_tool_schema_roundtrip() {
    let schema = ToolSchema {
        name: "file_read".to_string(),
        description: "Read a file from the filesystem".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }),
        permission_level: PermissionLevel::Low,
        example: Some("file_read({\"path\": \"/README.md\"})".to_string()),
    };
    let json = serde_json::to_string(&schema).unwrap();
    let parsed: ToolSchema = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "file_read");
    assert_eq!(parsed.permission_level, PermissionLevel::Low);
}
