# Orchestrator Type Alignment Reference

## CompletionRequest fields
- request_id: Uuid
- messages: Vec<NormalizedMessage>
- tools: Option<Vec<ToolDefinition>>
- parameters: CompletionParameters
- session_id: Uuid
- stream: bool
- required_capabilities: Vec<String>
NO: tool_choice, metadata, system_prompt

## CompletionResponse fields
- content: Vec<ContentBlock>
- usage: TokenUsage
- provider: String
- model: String
- stop_reason: StopReason
- latency_ms: u64
- received_at: DateTime<Utc>
NO: tool_calls (tool calls are in content as ContentBlock::ToolUse variants)

## TokenUsage
- input_tokens: u32
- output_tokens: u32
- cache_read_tokens: u32
- cache_write_tokens: u32
- thinking_tokens: u32
- .total() method returns total
NO: total_tokens field

## NormalizedMessage
- role: MessageRole
- content: Vec<ContentBlock>
NO: ::user(), ::system() constructors

## DeviationSeverity
- Minor, Significant, Critical
NO: Major

## CompactionResult
- summary: String
- tokens_before: usize
- tokens_after: usize
- messages_removed: usize
- session_id: Uuid
NO: messages_before, messages_after
