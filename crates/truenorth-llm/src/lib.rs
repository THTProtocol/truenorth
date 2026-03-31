//! # truenorth-llm
//!
//! LLM provider implementations, cascading fallback router, and embedding backends
//! for the TrueNorth orchestration system.
//!
//! ## Architecture
//!
//! This crate is the **only** crate in the TrueNorth workspace that directly calls
//! LLM provider APIs. All other crates communicate through the [`LlmRouter`] trait
//! defined in `truenorth-core`.
//!
//! ### Key components:
//!
//! - **[`router::DefaultLlmRouter`]** — Implements the double-loop cascade fallback
//!   strategy. Tries each configured provider in order; on full sweep failure, retries
//!   all providers a second time; on second failure, saves session state and returns
//!   [`RouterError::AllProvidersExhausted`].
//!
//! - **[`context_serializer::ContextSerializer`]** — Cross-provider context conversion
//!   (the π-ai pattern). Translates provider-specific artifacts (Anthropic thinking traces,
//!   OpenAI reasoning prefixes) into portable format for handoff on fallback.
//!
//! - **[`providers`]** — Individual LLM backend implementations:
//!   - `anthropic` — Claude via Messages API with streaming and extended thinking
//!   - `openai` — GPT-4 / o-series via Chat Completions API
//!   - `google` — Gemini via GenerateContent API
//!   - `ollama` — Local inference via OpenAI-compatible API
//!   - `openai_compat` — Generic OpenAI-compatible backends (LM Studio, Groq, etc.)
//!   - `mock` — Deterministic mock provider for testing
//!
//! - **[`embedding`]** — Embedding provider implementations:
//!   - `fastembed_provider` — Local ONNX-based embedding (AllMiniLML6V2), gated behind
//!     `#[cfg(feature = "local-embeddings")]`
//!   - `openai_embed` — Remote OpenAI text-embedding-3-small
//!   - `mock_embed` — Deterministic mock embedder for tests
//!
//! - **[`stream`]** — SSE stream parser shared across providers.
//!
//! - **[`rate_limiter`]** — Per-provider rate limit tracking with exponential backoff.

#![warn(missing_docs)]
#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod context_serializer;
pub mod embedding;
pub mod providers;
pub mod rate_limiter;
pub mod router;
pub mod stream;

// Re-export the primary types consumers need.
pub use router::{DefaultLlmRouter, RouterConfig};
pub use context_serializer::ContextSerializer;
pub use rate_limiter::RateLimiter;

// Re-export core traits so consumers don't need to depend on truenorth-core directly
// for the most common usage patterns.
pub use truenorth_core::traits::llm_provider::{LlmProvider, StreamHandle};
pub use truenorth_core::traits::llm_router::LlmRouter;
pub use truenorth_core::traits::embedding_provider::{EmbeddingProvider, EmbeddingError, EmbeddingModelInfo};
pub use truenorth_core::types::llm::{
    CompletionParameters, CompletionRequest, CompletionResponse, NormalizedMessage,
    ProviderCapabilities, StopReason, StreamEvent, TokenUsage, ToolDefinition,
};
pub use truenorth_core::types::routing::{ProviderStatus, RoutingDecision, RouterError, SkippedProvider, SkipReason};
pub use truenorth_core::error::LlmError;
