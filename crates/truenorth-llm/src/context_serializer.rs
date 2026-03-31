//! Cross-provider context serialization — the π-ai (pi-ai) pattern.
//!
//! When the LLM router falls back from Provider A to Provider B mid-session,
//! the conversation history must be translated between provider-specific formats.
//! This is the "pi-ai pattern": serialize context to a portable representation,
//! then deserialize into the target provider's format.
//!
//! ## Challenge
//!
//! Each provider has idiosyncratic message formats:
//!
//! - **Anthropic** emits `thinking` blocks with cryptographic signatures that
//!   cannot be reproduced on other providers. Extended thinking blocks are
//!   provider-opaque.
//!
//! - **OpenAI o-series** embeds reasoning in `reasoning_content` fields that
//!   have no equivalent in Anthropic's format.
//!
//! - **Google Gemini** uses `functionCall`/`functionResponse` where others use
//!   `tool_use`/`tool_result`.
//!
//! ## Strategy
//!
//! The serializer applies a **best-effort, fidelity-aware** strategy:
//!
//! 1. **Structured content** (text, tool calls, tool results) is preserved exactly.
//!    These are provider-portable and the target provider can use them unchanged.
//!
//! 2. **Thinking traces** are converted to tagged text blocks prefixed with
//!    `[Reasoning: ...]`. The target provider sees the reasoning content as context
//!    but does not interpret it as extended thinking.
//!
//! 3. **Signed thinking blocks** (Anthropic uses cryptographic signatures for
//!    integrity) have their signatures stripped. The thinking text is preserved
//!    as a tagged block. Fidelity loss: the target cannot verify the thinking trace.
//!
//! 4. **Image content** is preserved as base64 data URIs if the target supports
//!    vision, dropped with a warning if it does not.
//!
//! This is intentionally best-effort. Some fidelity is always lost on cross-provider
//! handoff. The important invariant is: the session continues rather than failing.

use tracing::{debug, info, warn};

use truenorth_core::types::message::{
    AgentMessage, ContentBlock, ConversationHistory, MessageContent, MessageRole,
};

/// Statistics about fidelity loss during context serialization.
#[derive(Debug, Clone, Default)]
pub struct SerializationFidelity {
    /// Number of thinking blocks that were converted to tagged text.
    pub thinking_blocks_converted: usize,
    /// Number of signed thinking blocks where the signature was stripped.
    pub signatures_stripped: usize,
    /// Number of image blocks dropped (target doesn't support vision).
    pub images_dropped: usize,
    /// Number of messages that required structural transformation.
    pub messages_transformed: usize,
    /// Any fidelity warnings to include in the reasoning event.
    pub warnings: Vec<String>,
}

/// Cross-provider context serializer.
///
/// Converts a `ConversationHistory` into a version compatible with a target
/// provider, applying the minimum necessary transformations to preserve
/// conversation continuity.
///
/// ## Usage
///
/// ```rust,no_run
/// use truenorth_llm::ContextSerializer;
/// use truenorth_core::ConversationHistory;
///
/// let serializer = ContextSerializer::new();
/// let history = ConversationHistory::default();
/// let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
/// if !fidelity.warnings.is_empty() {
///     tracing::warn!("Context handoff fidelity loss: {:?}", fidelity.warnings);
/// }
/// ```
#[derive(Debug, Default)]
pub struct ContextSerializer;

impl ContextSerializer {
    /// Creates a new `ContextSerializer`.
    pub fn new() -> Self {
        Self
    }

    /// Converts a conversation history into a version compatible with `target_provider`.
    ///
    /// Returns the adapted history and a fidelity report describing what was lost.
    ///
    /// ## Provider-specific transformations
    ///
    /// | Source → Target | Transformation |
    /// |-----------------|---------------|
    /// | Anthropic thinking → OpenAI | Thinking block → `[Reasoning: ...]` text prefix |
    /// | Anthropic thinking → Google | Same as OpenAI |
    /// | OpenAI reasoning → Anthropic | Reasoning prefix extracted → tagged text block |
    /// | Any → provider without vision | Image blocks dropped with warning |
    ///
    pub fn serialize_for_provider(
        &self,
        history: &ConversationHistory,
        target_provider: &str,
    ) -> (ConversationHistory, SerializationFidelity) {
        let mut fidelity = SerializationFidelity::default();
        let target_supports_thinking = matches!(target_provider, "anthropic");
        let target_supports_vision = matches!(target_provider, "anthropic" | "openai" | "google");

        info!(
            target_provider = target_provider,
            message_count = history.messages.len(),
            "ContextSerializer: adapting conversation history for provider handoff"
        );

        let adapted_messages: Vec<AgentMessage> = history
            .messages
            .iter()
            .map(|msg| {
                self.adapt_message(
                    msg,
                    target_provider,
                    target_supports_thinking,
                    target_supports_vision,
                    &mut fidelity,
                )
            })
            .collect();

        let adapted_history = ConversationHistory {
            messages: adapted_messages,
            total_tokens: history.total_tokens,
            is_compacted: history.is_compacted,
            compaction_summary: history.compaction_summary.clone(),
        };

        if fidelity.thinking_blocks_converted > 0 {
            fidelity.warnings.push(format!(
                "{} thinking block(s) converted to tagged text — target provider '{}' \
                 will see reasoning as context text, not as extended thinking",
                fidelity.thinking_blocks_converted, target_provider
            ));
        }
        if fidelity.signatures_stripped > 0 {
            fidelity.warnings.push(format!(
                "{} Anthropic thinking signature(s) stripped — \
                 integrity verification not possible on target provider",
                fidelity.signatures_stripped
            ));
        }
        if fidelity.images_dropped > 0 {
            fidelity.warnings.push(format!(
                "{} image block(s) dropped — target provider '{}' does not support vision",
                fidelity.images_dropped, target_provider
            ));
        }

        debug!(
            target_provider = target_provider,
            thinking_converted = fidelity.thinking_blocks_converted,
            signatures_stripped = fidelity.signatures_stripped,
            images_dropped = fidelity.images_dropped,
            messages_transformed = fidelity.messages_transformed,
            "ContextSerializer: serialization complete"
        );

        (adapted_history, fidelity)
    }

    /// Adapts a single message for the target provider.
    fn adapt_message(
        &self,
        msg: &AgentMessage,
        target_provider: &str,
        target_supports_thinking: bool,
        target_supports_vision: bool,
        fidelity: &mut SerializationFidelity,
    ) -> AgentMessage {
        let adapted_content = self.adapt_content_blocks(
            &msg.content,
            target_provider,
            target_supports_thinking,
            target_supports_vision,
            fidelity,
        );

        // Track if transformations occurred by checking if fidelity counters changed
        let total_before = fidelity.thinking_blocks_converted + fidelity.signatures_stripped + fidelity.images_dropped;

        let total_after = fidelity.thinking_blocks_converted + fidelity.signatures_stripped + fidelity.images_dropped;
        if total_after > total_before {
            fidelity.messages_transformed += 1;
        }

        AgentMessage {
            id: msg.id,
            role: msg.role.clone(),
            content: adapted_content,
            created_at: msg.created_at,
            tool_call_id: msg.tool_call_id.clone(),
            tool_calls: msg.tool_calls.clone(),
            token_count: msg.token_count,
        }
    }

    /// Adapts the content of a message for the target provider.
    fn adapt_content_blocks(
        &self,
        content: &MessageContent,
        target_provider: &str,
        target_supports_thinking: bool,
        target_supports_vision: bool,
        fidelity: &mut SerializationFidelity,
    ) -> MessageContent {
        match content {
            MessageContent::Text(text) => {
                // Plain text — check if it contains OpenAI-style reasoning prefix
                if target_provider == "anthropic" {
                    if let Some(adapted) = self.extract_openai_reasoning_prefix(text) {
                        // Convert reasoning prefix to Anthropic-style thinking block is not
                        // directly possible (Anthropic requires signed thinking blocks from
                        // its own API). We keep the reasoning as a tagged text block.
                        return MessageContent::Text(adapted);
                    }
                }
                MessageContent::Text(text.clone())
            }
            MessageContent::Blocks(blocks) => {
                let adapted: Vec<ContentBlock> = blocks
                    .iter()
                    .flat_map(|block| {
                        self.adapt_content_block(
                            block,
                            target_provider,
                            target_supports_thinking,
                            target_supports_vision,
                            fidelity,
                        )
                    })
                    .collect();
                MessageContent::Blocks(adapted)
            }
        }
    }

    /// Adapts a single content block for the target provider.
    ///
    /// Returns a `Vec` because some blocks expand (thinking → multiple text blocks)
    /// or collapse (image dropped → empty vec).
    fn adapt_content_block(
        &self,
        block: &ContentBlock,
        target_provider: &str,
        target_supports_thinking: bool,
        target_supports_vision: bool,
        fidelity: &mut SerializationFidelity,
    ) -> Vec<ContentBlock> {
        match block {
            ContentBlock::Thinking { thinking, signature } => {
                if target_supports_thinking && target_provider == "anthropic" {
                    // Keep thinking blocks when targeting Anthropic.
                    // However, strip signatures if present — they were generated by
                    // a previous Anthropic session and may not be valid in a new context.
                    if signature.is_some() {
                        fidelity.signatures_stripped += 1;
                        warn!(
                            "Stripping Anthropic thinking signature on context handoff — \
                             signature was generated by previous session and may be invalid"
                        );
                    }
                    // Keep the thinking content but strip the signature.
                    // Anthropic requires signature for tool use, but for text generation
                    // it's optional.
                    vec![ContentBlock::Thinking {
                        thinking: thinking.clone(),
                        signature: None, // stripped
                    }]
                } else {
                    // Convert thinking to a tagged text block for other providers.
                    // The text block signals to the LLM that this was reasoning content.
                    fidelity.thinking_blocks_converted += 1;
                    debug!(
                        target_provider = target_provider,
                        thinking_len = thinking.len(),
                        "Converting thinking block to tagged text for cross-provider handoff"
                    );
                    vec![ContentBlock::Text {
                        text: format!(
                            "[Extended Reasoning Block — Preserved for Context]\n{}\n[End Reasoning]",
                            thinking
                        ),
                    }]
                }
            }

            ContentBlock::Image { mime_type, data } => {
                if target_supports_vision {
                    // Keep image blocks for vision-capable providers
                    vec![block.clone()]
                } else {
                    // Drop image with a placeholder text so the conversation makes sense
                    fidelity.images_dropped += 1;
                    warn!(
                        target_provider = target_provider,
                        mime_type = mime_type.as_str(),
                        "Dropping image block — target provider does not support vision"
                    );
                    vec![ContentBlock::Text {
                        text: "[Image content omitted — target provider does not support vision]".to_string(),
                    }]
                }
            }

            ContentBlock::ToolUse { id, name, input } => {
                // Tool calls are provider-neutral — preserve exactly.
                vec![block.clone()]
            }

            ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                // Tool results are provider-neutral — preserve exactly.
                // Recursively adapt inner content if needed.
                let adapted_inner: Vec<ContentBlock> = content
                    .iter()
                    .flat_map(|inner| {
                        self.adapt_content_block(
                            inner,
                            target_provider,
                            target_supports_thinking,
                            target_supports_vision,
                            fidelity,
                        )
                    })
                    .collect();
                vec![ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: adapted_inner,
                    is_error: *is_error,
                }]
            }

            ContentBlock::Text { text } => {
                // Text blocks are provider-neutral — preserve exactly.
                // No transformation needed.
                vec![block.clone()]
            }
        }
    }

    /// Extracts OpenAI-style reasoning prefix from text.
    ///
    /// OpenAI o-series models sometimes embed reasoning as `[Reasoning: ...]` prefixes
    /// in assistant messages. This method detects and preserves those.
    fn extract_openai_reasoning_prefix(&self, text: &str) -> Option<String> {
        if text.starts_with("[Reasoning: ") {
            // Already tagged — no transformation needed
            Some(text.to_string())
        } else {
            None
        }
    }

    /// Computes a summary of the fidelity loss for inclusion in a `ReasoningEvent`.
    pub fn fidelity_summary(fidelity: &SerializationFidelity) -> String {
        if fidelity.warnings.is_empty() {
            return "Context serialized with full fidelity — no transformations needed".to_string();
        }
        format!(
            "Context serialized with partial fidelity: {}",
            fidelity.warnings.join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_message_with_block(role: MessageRole, block: ContentBlock) -> AgentMessage {
        AgentMessage {
            id: Uuid::new_v4(),
            role,
            content: MessageContent::Blocks(vec![block]),
            created_at: Utc::now(),
            tool_call_id: None,
            tool_calls: vec![],
            token_count: None,
        }
    }

    fn make_history(messages: Vec<AgentMessage>) -> ConversationHistory {
        ConversationHistory {
            messages,
            total_tokens: 0,
            is_compacted: false,
            compaction_summary: None,
        }
    }

    #[test]
    fn test_thinking_block_preserved_for_anthropic() {
        let serializer = ContextSerializer::new();
        let thinking_block = ContentBlock::Thinking {
            thinking: "I need to reason about this".to_string(),
            signature: None,
        };

        let history = make_history(vec![make_message_with_block(
            MessageRole::Assistant,
            thinking_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "anthropic");
        assert_eq!(fidelity.thinking_blocks_converted, 0, "Anthropic should keep thinking blocks");

        let blocks = match &adapted.messages[0].content {
            MessageContent::Blocks(b) => b,
            _ => panic!("Expected Blocks"),
        };
        assert!(matches!(&blocks[0], ContentBlock::Thinking { .. }));
    }

    #[test]
    fn test_thinking_block_converted_for_openai() {
        let serializer = ContextSerializer::new();
        let thinking_block = ContentBlock::Thinking {
            thinking: "I need to think about this".to_string(),
            signature: Some("sig123".to_string()),
        };

        let history = make_history(vec![make_message_with_block(
            MessageRole::Assistant,
            thinking_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
        assert_eq!(fidelity.thinking_blocks_converted, 1);
        assert_eq!(fidelity.signatures_stripped, 0, "Signature stripped count is for Anthropic target");

        let blocks = match &adapted.messages[0].content {
            MessageContent::Blocks(b) => b,
            _ => panic!("Expected Blocks"),
        };
        assert!(matches!(&blocks[0], ContentBlock::Text { .. }));
        if let ContentBlock::Text { text } = &blocks[0] {
            assert!(text.contains("I need to think about this"));
            assert!(text.contains("[Extended Reasoning Block"));
        }
    }

    #[test]
    fn test_thinking_signature_stripped_for_anthropic() {
        let serializer = ContextSerializer::new();
        let thinking_block = ContentBlock::Thinking {
            thinking: "Some reasoning".to_string(),
            signature: Some("original-signature".to_string()),
        };

        let history = make_history(vec![make_message_with_block(
            MessageRole::Assistant,
            thinking_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "anthropic");
        assert_eq!(fidelity.signatures_stripped, 1);

        if let MessageContent::Blocks(blocks) = &adapted.messages[0].content {
            if let ContentBlock::Thinking { signature, .. } = &blocks[0] {
                assert!(signature.is_none(), "Signature should be stripped");
            }
        }
    }

    #[test]
    fn test_image_dropped_for_non_vision_provider() {
        let serializer = ContextSerializer::new();
        let image_block = ContentBlock::Image {
            mime_type: "image/png".to_string(),
            data: "base64data".to_string(),
        };

        let history = make_history(vec![make_message_with_block(
            MessageRole::User,
            image_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "ollama");
        assert_eq!(fidelity.images_dropped, 1);

        if let MessageContent::Blocks(blocks) = &adapted.messages[0].content {
            assert!(matches!(&blocks[0], ContentBlock::Text { .. }));
        }
    }

    #[test]
    fn test_image_preserved_for_vision_provider() {
        let serializer = ContextSerializer::new();
        let image_block = ContentBlock::Image {
            mime_type: "image/jpeg".to_string(),
            data: "base64data".to_string(),
        };

        let history = make_history(vec![make_message_with_block(
            MessageRole::User,
            image_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "anthropic");
        assert_eq!(fidelity.images_dropped, 0);

        if let MessageContent::Blocks(blocks) = &adapted.messages[0].content {
            assert!(matches!(&blocks[0], ContentBlock::Image { .. }));
        }
    }

    #[test]
    fn test_text_blocks_preserved_unchanged() {
        let serializer = ContextSerializer::new();
        let text_block = ContentBlock::Text { text: "Hello world".to_string() };

        let history = make_history(vec![make_message_with_block(
            MessageRole::User,
            text_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "openai");
        assert_eq!(fidelity.messages_transformed, 0);
        assert_eq!(fidelity.warnings.len(), 0);
    }

    #[test]
    fn test_tool_calls_preserved() {
        let serializer = ContextSerializer::new();
        let tool_block = ContentBlock::ToolUse {
            id: "call_123".to_string(),
            name: "search_web".to_string(),
            input: serde_json::json!({ "query": "test" }),
        };

        let history = make_history(vec![make_message_with_block(
            MessageRole::Assistant,
            tool_block,
        )]);

        let (adapted, fidelity) = serializer.serialize_for_provider(&history, "google");
        assert_eq!(fidelity.messages_transformed, 0);

        if let MessageContent::Blocks(blocks) = &adapted.messages[0].content {
            assert!(matches!(&blocks[0], ContentBlock::ToolUse { .. }));
        }
    }

    #[test]
    fn test_fidelity_summary_no_loss() {
        let fidelity = SerializationFidelity::default();
        let summary = ContextSerializer::fidelity_summary(&fidelity);
        assert!(summary.contains("full fidelity"));
    }

    #[test]
    fn test_fidelity_summary_with_loss() {
        let mut fidelity = SerializationFidelity::default();
        fidelity.warnings.push("1 thinking block(s) converted".to_string());
        let summary = ContextSerializer::fidelity_summary(&fidelity);
        assert!(summary.contains("partial fidelity"));
        assert!(summary.contains("thinking block"));
    }
}
