//! Live provider integration tests.
//!
//! These tests call real LLM APIs and are marked `#[ignore]` by default.
//! Run them with:
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... cargo test -p truenorth-llm --test live_providers -- --ignored
//! OPENAI_API_KEY=sk-... cargo test -p truenorth-llm --test live_providers -- --ignored
//! ```

use truenorth_core::types::llm::{CompletionParameters, CompletionRequest};
use truenorth_core::types::llm::NormalizedMessage;
use truenorth_core::types::message::{ContentBlock, MessageRole};
use uuid::Uuid;

fn simple_request(prompt: &str) -> CompletionRequest {
    CompletionRequest {
        request_id: Uuid::new_v4(),
        messages: vec![NormalizedMessage {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: prompt.to_string(),
            }],
        }],
        parameters: CompletionParameters {
            temperature: Some(0.0),
            max_tokens: 100,
            ..Default::default()
        },
        tools: None,
        session_id: Uuid::new_v4(),
        stream: false,
        required_capabilities: vec![],
    }
}

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn test_anthropic_live_completion() {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("Skipping: ANTHROPIC_API_KEY not set");
            return;
        }
    };

    use truenorth_core::traits::llm_provider::LlmProvider;
    use truenorth_llm::providers::anthropic::AnthropicProvider;

    let provider = AnthropicProvider::new(api_key, "claude-sonnet-4-20250514".to_string());
    let request = simple_request("Reply with exactly: TRUENORTH_OK");
    let response = provider.complete(&request).await;

    match response {
        Ok(resp) => {
            assert!(!resp.content.is_empty(), "Response should have content");
            assert!(resp.usage.output_tokens > 0, "Should have used output tokens");
            println!("Anthropic response: {:?}", resp.content);
        }
        Err(e) => {
            panic!("Anthropic completion failed: {e}");
        }
    }
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_openai_live_completion() {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("Skipping: OPENAI_API_KEY not set");
            return;
        }
    };

    use truenorth_core::traits::llm_provider::LlmProvider;
    use truenorth_llm::providers::openai::OpenAiProvider;

    let provider = OpenAiProvider::new(api_key, "gpt-4o-mini".to_string());
    let request = simple_request("Reply with exactly: TRUENORTH_OK");
    let response = provider.complete(&request).await;

    match response {
        Ok(resp) => {
            assert!(!resp.content.is_empty(), "Response should have content");
            assert!(resp.usage.output_tokens > 0, "Should have used output tokens");
            println!("OpenAI response: {:?}", resp.content);
        }
        Err(e) => {
            panic!("OpenAI completion failed: {e}");
        }
    }
}
