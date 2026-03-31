/// TrueNorthError — the root error type for the TrueNorth system.
///
/// Every error in the system is either a variant here or a nested error type
/// specific to its domain (e.g., `MemoryError`, `ToolError`) that gets wrapped
/// into `TrueNorthError` at the application boundary.

use thiserror::Error;
use uuid::Uuid;

/// All possible failures from an LLM provider call.
///
/// Variants are intentionally distinct so the router can make intelligent
/// routing decisions (e.g., `RateLimited` → wait/skip vs. `ApiKeyExhausted`
/// → permanently skip vs. `NetworkError` → retry on next provider).
#[derive(Debug, Error, Clone, serde::Serialize, serde::Deserialize)]
pub enum LlmError {
    /// The provider returned HTTP 429 or equivalent.
    /// Contains the retry-after duration if the provider specified one.
    #[error("Rate limited by {provider}: retry after {retry_after_secs}s")]
    RateLimited {
        provider: String,
        retry_after_secs: u64,
    },

    /// API key is invalid, expired, or the account has no remaining quota.
    /// This provider should be permanently skipped for this session.
    #[error("API key exhausted or invalid for provider {provider}")]
    ApiKeyExhausted { provider: String },

    /// The provider accepted the request but the model refused to generate
    /// (content policy, safety filter, length limit, etc.).
    /// The router should NOT fall back — this is a content issue, not a provider issue.
    #[error("Model refused to generate: {reason}")]
    ModelRefusal { reason: String },

    /// Transient network failure (DNS, TCP, TLS). Retry on next provider.
    #[error("Network error communicating with {provider}: {message}")]
    NetworkError { provider: String, message: String },

    /// The provider returned an unparseable or unexpected response format.
    #[error("Malformed response from {provider}: {detail}")]
    MalformedResponse { provider: String, detail: String },

    /// The request exceeded the model's context window.
    /// The orchestrator should compact context and retry.
    #[error("Context window exceeded for {provider}/{model}: {token_count} tokens")]
    ContextWindowExceeded {
        provider: String,
        model: String,
        token_count: usize,
    },

    /// The provider's streaming connection was interrupted mid-response.
    /// Partial content may be available.
    #[error("Stream interrupted from {provider} after {bytes_received} bytes")]
    StreamInterrupted {
        provider: String,
        bytes_received: usize,
        partial_output: Option<String>,
    },

    /// An unexpected error that doesn't fit the other categories.
    #[error("Unexpected error from {provider}: {message}")]
    Other { provider: String, message: String },
}

/// The root error type for TrueNorth.
///
/// Used at application boundaries (CLI command handlers, API endpoints, web handlers).
/// Internal modules use their own domain-specific error types for precision.
#[derive(Debug, Error)]
pub enum TrueNorthError {
    /// An error from an LLM provider.
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    /// All LLM providers were exhausted.
    #[error("All LLM providers exhausted. Session saved. Resume with: truenorth resume {session_id}")]
    AllProvidersExhausted { session_id: Uuid },

    /// A memory layer error.
    #[error("Memory error: {0}")]
    Memory(#[from] crate::traits::memory::MemoryError),

    /// A tool execution error.
    #[error("Tool error: {0}")]
    Tool(#[from] crate::types::tool::ToolError),

    /// A session management error.
    #[error("Session error: {0}")]
    Session(#[from] crate::traits::session::SessionError),

    /// An execution strategy or agent loop error.
    #[error("Execution error: {0}")]
    Execution(#[from] crate::traits::execution::ExecutionError),

    /// A WASM sandbox error.
    #[error("WASM error: {0}")]
    Wasm(#[from] crate::traits::wasm::WasmError),

    /// A skill loading error.
    #[error("Skill error: {0}")]
    Skill(#[from] crate::traits::skill::SkillError),

    /// A context budget error.
    #[error("Context budget error: {0}")]
    Budget(#[from] crate::traits::context::BudgetError),

    /// A state serialization error.
    #[error("State error: {0}")]
    State(#[from] crate::traits::state::StateError),

    /// A reasoning event emitter error.
    #[error("Reasoning error: {0}")]
    Reasoning(#[from] crate::traits::reasoning::ReasoningError),

    /// A deviation tracker error.
    #[error("Deviation error: {0}")]
    Deviation(#[from] crate::traits::deviation::DeviationError),

    /// A checklist verification error.
    #[error("Checklist error: {0}")]
    Checklist(#[from] crate::traits::checklist::ChecklistError),

    /// A heartbeat scheduler error.
    #[error("Heartbeat error: {0}")]
    Heartbeat(#[from] crate::traits::heartbeat::HeartbeatError),

    /// Configuration loading or parsing error.
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// An internal assertion failed — this is a bug in TrueNorth.
    #[error("Internal error: {message} (this is a bug — please report it)")]
    Internal { message: String },

    /// An operation timed out.
    #[error("Operation timed out after {timeout_ms}ms: {operation}")]
    Timeout { operation: String, timeout_ms: u64 },

    /// A feature is not implemented in this version.
    #[error("Not implemented: {feature}")]
    NotImplemented { feature: String },
}

impl TrueNorthError {
    /// Returns true if this error is recoverable (the agent should retry or fall back).
    pub fn is_recoverable(&self) -> bool {
        match self {
            TrueNorthError::Llm(LlmError::RateLimited { .. }) => true,
            TrueNorthError::Llm(LlmError::NetworkError { .. }) => true,
            TrueNorthError::Llm(LlmError::StreamInterrupted { .. }) => true,
            TrueNorthError::Io(_) => false,
            TrueNorthError::AllProvidersExhausted { .. } => false,
            TrueNorthError::Internal { .. } => false,
            _ => false,
        }
    }

    /// Returns true if the session state should be saved before propagating this error.
    pub fn should_save_state(&self) -> bool {
        matches!(
            self,
            TrueNorthError::AllProvidersExhausted { .. }
                | TrueNorthError::Execution(
                    crate::traits::execution::ExecutionError::ContextExhausted { .. }
                )
                | TrueNorthError::Execution(
                    crate::traits::execution::ExecutionError::LlmExhausted { .. }
                )
        )
    }
}
