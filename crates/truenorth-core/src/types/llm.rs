/// LLM types — normalized request/response types for all LLM providers.
///
/// All provider-specific formats (Anthropic, OpenAI, Ollama) are translated
/// to/from these types at the provider boundary. Calling code never deals
/// with provider-specific wire formats.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::message::{ContentBlock, MessageRole};

/// A request to an LLM provider for text completion.
///
/// Provider-neutral: each provider implementation translates this into its
/// own API request format. New providers only need to implement that
/// translation, not change this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Unique identifier for this request (for tracing and deduplication).
    pub request_id: Uuid,
    /// The conversation history, including system prompt and latest user message.
    pub messages: Vec<NormalizedMessage>,
    /// Tool definitions the model may call. None = no tool use.
    pub tools: Option<Vec<ToolDefinition>>,
    /// Generation parameters (temperature, max_tokens, etc.).
    pub parameters: CompletionParameters,
    /// The session this request belongs to (for logging and context budget tracking).
    pub session_id: Uuid,
    /// Whether to stream the response token-by-token.
    pub stream: bool,
    /// Required capabilities for routing (e.g., ["vision", "thinking"]).
    pub required_capabilities: Vec<String>,
}

/// A single normalized message for inclusion in a completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedMessage {
    /// The role of the message author.
    pub role: MessageRole,
    /// The content of the message.
    pub content: Vec<ContentBlock>,
}

/// A tool definition exposed to the LLM in a completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The tool's canonical name.
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// Generation parameters controlling LLM output behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionParameters {
    /// Maximum tokens to generate. Required.
    pub max_tokens: u32,
    /// Sampling temperature (0.0–2.0). None = provider default.
    pub temperature: Option<f32>,
    /// Top-p nucleus sampling. None = provider default.
    pub top_p: Option<f32>,
    /// Stop sequences — generation halts at any of these strings.
    pub stop_sequences: Vec<String>,
    /// Whether to enable extended thinking (Anthropic) or reasoning (OpenAI o-series).
    pub enable_thinking: bool,
    /// Token budget for extended thinking.
    pub thinking_budget: Option<u32>,
}

impl Default for CompletionParameters {
    fn default() -> Self {
        Self {
            max_tokens: 8192,
            temperature: None,
            top_p: None,
            stop_sequences: vec![],
            enable_thinking: false,
            thinking_budget: None,
        }
    }
}

/// A complete (non-streaming) response from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The response content blocks (may include text, tool calls, thinking).
    pub content: Vec<ContentBlock>,
    /// Token usage for this call.
    pub usage: TokenUsage,
    /// Which provider generated this response.
    pub provider: String,
    /// The specific model used.
    pub model: String,
    /// Why the model stopped generating.
    pub stop_reason: StopReason,
    /// Wall-clock latency of the provider call in milliseconds.
    pub latency_ms: u64,
    /// When the response was received.
    pub received_at: DateTime<Utc>,
}

/// Reason the generation stopped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StopReason {
    /// Normal completion — the model finished its response.
    EndTurn,
    /// The model stopped because it emitted a tool call.
    ToolUse,
    /// The response was truncated by the max_tokens limit.
    MaxTokens,
    /// The model hit a configured stop sequence.
    StopSequence,
    /// Content filtering stopped generation.
    ContentFilter,
}

/// Token consumption for a single LLM request.
///
/// Used for context budget management and cost tracking.
/// Some providers report cache hits which affect billing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    /// Tokens in the input/prompt (including system prompt and history).
    pub input_tokens: u32,
    /// Tokens generated in the response.
    pub output_tokens: u32,
    /// Tokens read from provider-side prompt cache (if applicable).
    pub cache_read_tokens: u32,
    /// Tokens written to provider-side prompt cache (if applicable).
    pub cache_write_tokens: u32,
    /// Tokens used for extended thinking/reasoning.
    /// May not count against context window but may be billed separately.
    pub thinking_tokens: u32,
}

impl TokenUsage {
    /// Returns the total token count (input + output).
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }

    /// Returns the total billed tokens (input + output + cache write).
    pub fn billed_total(&self) -> u32 {
        self.input_tokens + self.output_tokens + self.cache_write_tokens
    }
}

/// A single event in a streaming LLM response.
///
/// Consumers accumulate these events to build the full response.
/// The stream always terminates with `StreamEnd` or a stream error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    /// A chunk of generated text. May be a single token or multiple.
    TextDelta { delta: String },

    /// A thinking/reasoning delta (Anthropic extended thinking, o-series reasoning).
    /// Emitted separately so the UI can display them in a distinct panel.
    ThinkingDelta { delta: String },

    /// A tool call is starting — the model has decided to use a tool.
    ToolUseStart { id: String, name: String },

    /// A chunk of a tool call's input JSON (streamed incrementally).
    ToolInputDelta {
        tool_use_id: String,
        partial_json: String,
    },

    /// The tool call's input JSON is complete and ready for execution.
    ToolUseComplete {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// The stream is ending; contains final usage statistics.
    /// Always the last event in a successful stream.
    StreamEnd {
        usage: TokenUsage,
        stop_reason: StopReason,
    },

    /// An error occurred mid-stream (partial content may exist).
    StreamError { message: String },
}

/// Provider capability declaration.
///
/// Implementors must be honest here — the router uses this to make
/// routing decisions (e.g., don't route vision requests to a provider
/// that returns `supports_vision = false`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    /// Whether this provider supports streaming responses.
    pub supports_streaming: bool,
    /// Whether this provider supports structured tool/function calling.
    pub supports_tool_calling: bool,
    /// Whether this provider can process image inputs.
    pub supports_vision: bool,
    /// Whether this provider supports extended thinking/reasoning modes.
    pub supports_thinking: bool,
    /// Maximum context window in tokens (input + output combined).
    pub max_context_tokens: usize,
    /// Maximum output tokens in a single response.
    pub max_output_tokens: usize,
    /// Supported output modalities (e.g., ["text", "audio"]).
    pub output_modalities: Vec<String>,
    /// Provider name for identification.
    pub provider_name: String,
    /// Model identifier.
    pub model_name: String,
}
