//! Integration tests for truenorth-llm.
//!
//! Tests cross-crate interactions: MockProvider ↔ LlmProvider trait,
//! DefaultLlmRouter cascade logic, ContextSerializer roundtrip,
//! RateLimiter state, and stream parsing utilities.

use std::sync::Arc;

use futures::StreamExt;
use uuid::Uuid;

use truenorth_llm::{
    ContextSerializer, RateLimiter,
    providers::MockProvider,
    router::{DefaultLlmRouter, RouterConfig},
};
use truenorth_llm::providers::ArcProvider;
use truenorth_core::{
    error::LlmError,
    traits::llm_provider::LlmProvider,
    traits::llm_router::LlmRouter,
    types::llm::{
        CompletionParameters, CompletionRequest, StopReason, StreamEvent, TokenUsage,
    },
    types::message::{AgentMessage, ContentBlock, ConversationHistory, MessageContent, MessageRole},
};

// ─── Helper constructors ──────────────────────────────────────────────────────

fn make_request() -> CompletionRequest {
    CompletionRequest {
        request_id: Uuid::new_v4(),
        messages: vec![],
        tools: None,
        parameters: CompletionParameters::default(),
        session_id: Uuid::new_v4(),
        stream: false,
        required_capabilities: vec![],
    }
}

fn make_request_with_capabilities(caps: Vec<String>) -> CompletionRequest {
    CompletionRequest {
        request_id: Uuid::new_v4(),
        messages: vec![],
        tools: None,
        parameters: CompletionParameters::default(),
        session_id: Uuid::new_v4(),
        stream: false,
        required_capabilities: caps,
    }
}

fn arc_mock(name: &str) -> ArcProvider {
    Arc::new(MockProvider::with_name(name))
}

fn arc_mock_with_response(name: &str, response: &str) -> ArcProvider {
    let m = MockProvider::with_name(name);
    m.set_response(response);
    Arc::new(m)
}

// ─── 1. MockProvider implements LlmProvider correctly ────────────────────────

#[tokio::test]
async fn mock_provider_implements_llm_provider_trait() {
    let mock = MockProvider::new();
    // Trait methods are available
    assert_eq!(mock.name(), "mock");
    assert_eq!(mock.model(), "mock-model-1.0");
    assert!(mock.capabilities().supports_streaming);
    assert!(mock.capabilities().supports_tool_calling);
    assert!(mock.is_available());
}

#[tokio::test]
async fn mock_provider_returns_configured_response_text() {
    let mock = MockProvider::new();
    mock.set_response("Hello from integration test!");
    let resp = mock.complete(&make_request()).await.unwrap();
    assert!(
        resp.content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("integration test"))),
        "Expected configured response text in content blocks"
    );
}

#[tokio::test]
async fn mock_provider_records_all_calls() {
    let mock = MockProvider::new();
    for _ in 0..5 {
        let _ = mock.complete(&make_request()).await;
    }
    assert_eq!(mock.success_count(), 5);
    assert_eq!(mock.recorded_calls().len(), 5);
    assert!(mock.recorded_calls().iter().all(|c| c.succeeded));
}

#[tokio::test]
async fn mock_provider_mark_exhausted_makes_unavailable() {
    let mock = MockProvider::new();
    assert!(mock.is_available());
    mock.mark_exhausted();
    assert!(!mock.is_available(), "Exhausted provider must not be available");
}

#[tokio::test]
async fn mock_provider_mark_rate_limited_makes_unavailable() {
    let mock = MockProvider::new();
    // mark_rate_limited with a long duration — provider should be immediately unavailable
    mock.mark_rate_limited(9999);
    assert!(!mock.is_available(), "Rate-limited provider must not be available");
}

#[tokio::test]
async fn mock_provider_returns_correct_stop_reason() {
    let mock = MockProvider::new();
    let resp = mock.complete(&make_request()).await.unwrap();
    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    assert_eq!(resp.provider, "mock");
}

#[tokio::test]
async fn mock_provider_simulates_rate_limit_error() {
    let mock = MockProvider::new();
    mock.simulate_rate_limited(60);
    let err = mock.complete(&make_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::RateLimited { retry_after_secs: 60, .. }),
        "Expected RateLimited error, got: {:?}", err
    );
}

#[tokio::test]
async fn mock_provider_simulates_api_key_exhaustion() {
    let mock = MockProvider::new();
    mock.simulate_exhausted();
    let err = mock.complete(&make_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::ApiKeyExhausted { .. }),
        "Expected ApiKeyExhausted, got: {:?}", err
    );
}

#[tokio::test]
async fn mock_provider_fails_after_n_successes() {
    let mock = MockProvider::new();
    mock.simulate_rate_limit_after(2);

    assert!(mock.complete(&make_request()).await.is_ok(), "Call 1 should succeed");
    assert!(mock.complete(&make_request()).await.is_ok(), "Call 2 should succeed");
    let third = mock.complete(&make_request()).await;
    assert!(third.is_err(), "Call 3 should fail (rate limit after 2)");
    assert!(matches!(third, Err(LlmError::RateLimited { .. })));
}

#[tokio::test]
async fn mock_provider_reset_clears_all_state() {
    let mock = MockProvider::new();
    // Use mark_exhausted (sets is_exhausted state) rather than simulate_exhausted (sets failure_mode)
    mock.mark_exhausted();
    assert!(!mock.is_available(), "Exhausted provider should not be available");
    mock.reset();
    assert!(mock.is_available(), "Reset should restore availability");
    assert!(mock.complete(&make_request()).await.is_ok());
}

// ─── 2. Router cascade logic ──────────────────────────────────────────────────

#[tokio::test]
async fn router_selects_primary_provider_when_available() {
    let primary = arc_mock_with_response("primary", "Primary response");
    let fallback = arc_mock_with_response("fallback", "Fallback response");

    let router = DefaultLlmRouter::new(vec![primary, fallback]);
    let resp = router.route(&make_request()).await.unwrap();

    assert_eq!(resp.provider, "primary", "Should prefer the first provider");
}

#[tokio::test]
async fn router_cascades_to_second_when_first_rate_limited() {
    // First provider: always rate-limited
    let rate_limited = MockProvider::with_name("rate_limited");
    rate_limited.simulate_rate_limited(30);

    // Second provider: succeeds
    let working = MockProvider::with_name("working");
    working.set_response("Fallback success!");

    let providers: Vec<ArcProvider> = vec![Arc::new(rate_limited), Arc::new(working)];
    let router = DefaultLlmRouter::new(providers);

    let resp = router.route(&make_request()).await.unwrap();
    assert_eq!(resp.provider, "working", "Should fall back to second provider");
}

#[tokio::test]
async fn router_cascades_to_third_when_first_two_fail() {
    // First: rate-limited, Second: exhausted, Third: succeeds
    let p1 = MockProvider::with_name("p1_rate_limited");
    p1.simulate_rate_limited(30);

    let p2 = MockProvider::with_name("p2_exhausted");
    p2.simulate_exhausted();

    let p3 = MockProvider::with_name("p3_success");
    p3.set_response("Third provider wins!");

    let providers: Vec<ArcProvider> = vec![Arc::new(p1), Arc::new(p2), Arc::new(p3)];
    let config = RouterConfig {
        max_loops: 2,
        session_id: Uuid::new_v4(),
        snapshot_dir: "/tmp".to_string(),
        verbose_routing: false,
    };
    let router = DefaultLlmRouter::with_config(providers, config);

    let resp = router.route(&make_request()).await.unwrap();
    assert_eq!(resp.provider, "p3_success");
    assert!(resp.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text.contains("Third provider wins"))));
}

#[tokio::test]
async fn router_returns_exhausted_error_when_all_providers_fail() {
    let p1 = MockProvider::with_name("p1");
    p1.simulate_exhausted();

    let p2 = MockProvider::with_name("p2");
    p2.simulate_exhausted();

    let providers: Vec<ArcProvider> = vec![Arc::new(p1), Arc::new(p2)];
    let config = RouterConfig {
        max_loops: 2,
        session_id: Uuid::new_v4(),
        snapshot_dir: "/tmp".to_string(),
        verbose_routing: false,
    };
    let router = DefaultLlmRouter::with_config(providers, config);

    let result = router.route(&make_request()).await;
    assert!(result.is_err(), "Should fail when all providers exhausted");
}

#[tokio::test]
async fn router_with_no_providers_returns_error() {
    let router = DefaultLlmRouter::new(vec![]);
    let result = router.route(&make_request()).await;
    assert!(result.is_err(), "Router with no providers should return error");
}

#[tokio::test]
async fn router_provider_statuses_reflect_health() {
    let p1 = arc_mock("healthy");
    let router = DefaultLlmRouter::new(vec![p1]);
    let statuses = router.provider_statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].name, "healthy");
    assert!(statuses[0].available);
    assert!(!statuses[0].exhausted);
}

#[tokio::test]
async fn router_emits_events_on_successful_routing() {
    use std::sync::Mutex;
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    let events_clone = events.clone();

    let mock = arc_mock_with_response("event_provider", "Event test");
    let router = DefaultLlmRouter::new(vec![mock])
        .with_event_emitter(move |ev| {
            events_clone.lock().unwrap().push(ev);
        });

    router.route(&make_request()).await.unwrap();

    let captured = events.lock().unwrap();
    assert!(!captured.is_empty(), "At least one event should be emitted");
    // The routed event should have provider info
    let routed = captured.iter().find(|e| e["type"] == "llm_routed");
    assert!(routed.is_some(), "llm_routed event should be emitted");
    assert_eq!(routed.unwrap()["provider"], "event_provider");
}

// ─── 3. ContextSerializer roundtrip ──────────────────────────────────────────

#[tokio::test]
async fn context_serializer_roundtrip_empty_history() {
    let serializer = ContextSerializer::new();
    let history = ConversationHistory::new();
    let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
    assert!(adapted.messages.is_empty());
    assert_eq!(fidelity.thinking_blocks_converted, 0);
    assert_eq!(fidelity.images_dropped, 0);
}

#[tokio::test]
async fn context_serializer_preserves_text_messages_for_openai() {
    let serializer = ContextSerializer::new();
    let mut history = ConversationHistory::new();

    let msg = AgentMessage {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Text("Hello, world!".to_string()),
        created_at: chrono::Utc::now(),
        tool_call_id: None,
        tool_calls: vec![],
        token_count: Some(10),
    };
    history.push(msg);

    let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
    assert_eq!(adapted.messages.len(), 1);
    assert_eq!(fidelity.thinking_blocks_converted, 0);
    // Text messages are preserved without transformation
    assert_eq!(fidelity.messages_transformed, 0);
}

#[tokio::test]
async fn context_serializer_converts_thinking_blocks_for_openai() {
    let serializer = ContextSerializer::new();
    let mut history = ConversationHistory::new();

    let msg = AgentMessage {
        id: Uuid::new_v4(),
        role: MessageRole::Assistant,
        content: MessageContent::Blocks(vec![
            ContentBlock::Thinking {
                thinking: "Step 1: analyze the problem".to_string(),
                signature: Some("sig-abc123".to_string()),
            },
            ContentBlock::Text { text: "Final answer.".to_string() },
        ]),
        created_at: chrono::Utc::now(),
        tool_call_id: None,
        tool_calls: vec![],
        token_count: None,
    };
    history.push(msg);

    let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
    assert_eq!(adapted.messages.len(), 1);
    // Thinking block should be converted (not supported by openai)
    assert!(fidelity.thinking_blocks_converted > 0, "Thinking block should be converted");
}

#[tokio::test]
async fn context_serializer_drops_images_for_non_vision_provider() {
    let serializer = ContextSerializer::new();
    let mut history = ConversationHistory::new();

    let msg = AgentMessage {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Blocks(vec![
            ContentBlock::Text { text: "Look at this image".to_string() },
            ContentBlock::Image {
                mime_type: "image/png".to_string(),
                data: "base64data==".to_string(),
            },
        ]),
        created_at: chrono::Utc::now(),
        tool_call_id: None,
        tool_calls: vec![],
        token_count: None,
    };
    history.push(msg);

    // "ollama" doesn't support vision
    let (adapted, fidelity) = serializer.serialize_for_provider(&history, "ollama");
    assert_eq!(adapted.messages.len(), 1);
    assert!(fidelity.images_dropped > 0, "Image should be dropped for non-vision provider");
    assert!(!fidelity.warnings.is_empty());
}

#[tokio::test]
async fn context_serializer_preserves_images_for_openai() {
    let serializer = ContextSerializer::new();
    let mut history = ConversationHistory::new();

    let msg = AgentMessage {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Blocks(vec![
            ContentBlock::Image {
                mime_type: "image/jpeg".to_string(),
                data: "data==".to_string(),
            },
        ]),
        created_at: chrono::Utc::now(),
        tool_call_id: None,
        tool_calls: vec![],
        token_count: None,
    };
    history.push(msg);

    let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
    assert_eq!(fidelity.images_dropped, 0, "OpenAI supports vision — no images should be dropped");
    // The message should still be in adapted history
    assert_eq!(adapted.messages.len(), 1);
}

#[tokio::test]
async fn context_serializer_preserves_token_count() {
    let serializer = ContextSerializer::new();
    let mut history = ConversationHistory::new();
    let msg = AgentMessage {
        id: Uuid::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Text("Test".to_string()),
        created_at: chrono::Utc::now(),
        tool_call_id: None,
        tool_calls: vec![],
        token_count: Some(42),
    };
    history.push(msg);
    assert_eq!(history.total_tokens, 42);

    let (adapted, _) = serializer.serialize_for_provider(&history, "openai");
    assert_eq!(adapted.total_tokens, 42, "Total token count should be preserved");
}

// ─── 4. RateLimiter ──────────────────────────────────────────────────────────

#[test]
fn rate_limiter_new_provider_is_available() {
    let rl = RateLimiter::new();
    rl.register_provider("alpha");
    assert!(rl.is_available("alpha"));
}

#[test]
fn rate_limiter_mark_rate_limited_makes_unavailable() {
    let rl = RateLimiter::new();
    rl.register_provider("beta");
    rl.mark_rate_limited("beta", 300);
    assert!(!rl.is_available("beta"));
}

#[test]
fn rate_limiter_mark_exhausted_makes_permanently_unavailable() {
    let rl = RateLimiter::new();
    rl.register_provider("gamma");
    rl.mark_exhausted("gamma");
    assert!(!rl.is_available("gamma"));
    // Can still retrieve state to inspect
    let state = rl.get_state("gamma");
    assert!(state.is_exhausted);
}

#[test]
fn rate_limiter_restore_clears_all_flags() {
    let rl = RateLimiter::new();
    rl.mark_exhausted("delta");
    assert!(!rl.is_available("delta"));
    rl.restore("delta");
    assert!(rl.is_available("delta"));
}

#[test]
fn rate_limiter_record_success_resets_consecutive_failures() {
    let rl = RateLimiter::new();
    rl.register_provider("epsilon");
    rl.record_failure("epsilon");
    rl.record_failure("epsilon");
    rl.record_failure("epsilon");
    let state = rl.get_state("epsilon");
    assert_eq!(state.consecutive_failures, 3);

    rl.record_success("epsilon");
    let state = rl.get_state("epsilon");
    assert_eq!(state.consecutive_failures, 0);
    assert_eq!(state.success_count, 1);
}

#[test]
fn rate_limiter_backoff_increases_with_failures() {
    use std::time::Duration;
    let rl = RateLimiter::new();
    rl.register_provider("flaky");

    let delay_zero = rl.backoff_delay("flaky");
    assert_eq!(delay_zero, Duration::ZERO, "No failures → zero backoff");

    rl.record_failure("flaky");
    let delay_1 = rl.backoff_delay("flaky");
    assert!(delay_1 > Duration::ZERO, "After 1 failure, backoff should be positive");

    rl.record_failure("flaky");
    let delay_2 = rl.backoff_delay("flaky");
    assert!(delay_2 > Duration::ZERO, "After 2 failures, backoff should be positive");
}

#[test]
fn rate_limiter_parse_retry_after_seconds() {
    assert_eq!(RateLimiter::parse_retry_after("30", 60), 30);
    assert_eq!(RateLimiter::parse_retry_after("  120  ", 60), 120);
}

#[test]
fn rate_limiter_parse_retry_after_invalid_uses_default() {
    assert_eq!(RateLimiter::parse_retry_after("not-a-number", 45), 45);
    assert_eq!(RateLimiter::parse_retry_after("", 10), 10);
}

#[test]
fn rate_limiter_all_states_returns_all_providers() {
    let rl = RateLimiter::new();
    rl.register_provider("one");
    rl.register_provider("two");
    rl.register_provider("three");
    let states = rl.all_states();
    assert_eq!(states.len(), 3);
    assert!(states.contains_key("one"));
    assert!(states.contains_key("two"));
    assert!(states.contains_key("three"));
}

#[test]
fn rate_limiter_disabled_provider_is_unavailable() {
    let rl = RateLimiter::new();
    rl.register_provider("zeta");
    rl.mark_disabled("zeta", "manually disabled for maintenance");
    assert!(!rl.is_available("zeta"));
    let state = rl.get_state("zeta");
    assert!(state.is_manually_disabled);
}

// ─── 5. Stream utilities ──────────────────────────────────────────────────────

#[test]
fn sse_parse_line_data() {
    use truenorth_llm::stream::{parse_sse_line, SseLine};
    let line = parse_sse_line("data: {\"type\": \"text\"}");
    assert!(matches!(line, SseLine::Data(s) if s == "{\"type\": \"text\"}"));
}

#[test]
fn sse_parse_line_done_sentinel() {
    use truenorth_llm::stream::{parse_sse_line, SseLine};
    let line = parse_sse_line("data: [DONE]");
    assert!(matches!(line, SseLine::Done));
}

#[test]
fn sse_parse_line_empty() {
    use truenorth_llm::stream::{parse_sse_line, SseLine};
    let line = parse_sse_line("");
    assert!(matches!(line, SseLine::Empty));
}

#[test]
fn sse_parse_line_event_type() {
    use truenorth_llm::stream::{parse_sse_line, SseLine};
    let line = parse_sse_line("event: content_block_delta");
    assert!(matches!(line, SseLine::EventType(s) if s == "content_block_delta"));
}

#[test]
fn sse_parse_line_comment() {
    use truenorth_llm::stream::{parse_sse_line, SseLine};
    let line = parse_sse_line(": keep-alive");
    assert!(matches!(line, SseLine::Comment(_)));
}

#[test]
fn sse_split_chunk_handles_multiple_lines() {
    use truenorth_llm::stream::split_sse_chunk;
    let chunk = "data: line1\ndata: line2\n\n";
    let lines = split_sse_chunk(chunk);
    assert!(lines.len() >= 3, "Should split into multiple lines");
    assert_eq!(lines[0], "data: line1");
    assert_eq!(lines[1], "data: line2");
}

#[tokio::test]
async fn mock_provider_stream_delivers_text_deltas_and_end() {
    let mock = MockProvider::new();
    mock.set_response("Stream test content!");

    let stream = mock.stream(&make_request()).await.expect("stream should open");
    let events: Vec<_> = stream.collect().await;

    let has_text_delta = events.iter().any(|e| matches!(e, Ok(StreamEvent::TextDelta { .. })));
    let has_stream_end = events.iter().any(|e| matches!(e, Ok(StreamEvent::StreamEnd { .. })));

    assert!(has_text_delta, "Stream should produce at least one TextDelta");
    assert!(has_stream_end, "Stream should terminate with StreamEnd");
}

#[tokio::test]
async fn mock_provider_stream_end_has_correct_stop_reason() {
    let mock = MockProvider::new();
    mock.set_response("End reason test");

    let stream = mock.stream(&make_request()).await.unwrap();
    let events: Vec<_> = stream.collect().await;

    let end_event = events.iter().find_map(|e| {
        if let Ok(StreamEvent::StreamEnd { stop_reason, .. }) = e {
            Some(stop_reason.clone())
        } else {
            None
        }
    });

    assert!(end_event.is_some());
    assert_eq!(end_event.unwrap(), StopReason::EndTurn);
}

// ─── 6. TokenUsage arithmetic ────────────────────────────────────────────────

#[test]
fn token_usage_total_is_sum_of_input_and_output() {
    let usage = TokenUsage {
        input_tokens: 1000,
        output_tokens: 250,
        cache_read_tokens: 100,
        cache_write_tokens: 50,
        thinking_tokens: 200,
    };
    assert_eq!(usage.total(), 1250);
}

#[test]
fn token_usage_billed_total_includes_cache_writes() {
    let usage = TokenUsage {
        input_tokens: 1000,
        output_tokens: 250,
        cache_read_tokens: 100,
        cache_write_tokens: 50,
        thinking_tokens: 0,
    };
    assert_eq!(usage.billed_total(), 1300); // 1000 + 250 + 50
}
