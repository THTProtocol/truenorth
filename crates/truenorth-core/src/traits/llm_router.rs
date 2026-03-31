/// LlmRouter trait — the cascading fallback router.
///
/// The router is the ONLY component that calls `LlmProvider` methods.
/// All other components interact with the router, never directly with providers.
/// This ensures the fallback logic is executed uniformly for all LLM calls.

use async_trait::async_trait;

use crate::types::llm::{CompletionRequest, CompletionResponse};
use crate::types::routing::{ProviderStatus, RoutingDecision, RouterError};
use crate::traits::llm_provider::StreamHandle;

/// The LLM Router trait: a stateful multiplexer over multiple `LlmProvider` implementations.
///
/// The router's core contract (from Phase 1, non-negotiable spec):
/// 1. Try primary provider
/// 2. On failure, try each fallback in configured order
/// 3. On full sweep failure, start loop 2
/// 4. On second loop failure, halt and save state
///
/// The router is the ONLY component that calls `LlmProvider` methods.
/// All other components interact with the router, never directly with providers.
/// This ensures the fallback logic is executed uniformly for all LLM calls.
#[async_trait]
pub trait LlmRouter: Send + Sync + std::fmt::Debug {
    /// Routes a completion request to the best available provider.
    ///
    /// Selection order: primary provider → fallback_order from config.
    /// Providers that don't support required capabilities are skipped.
    ///
    /// The router logs every routing decision as a `ReasoningEvent::LlmRouted`
    /// before returning the response. Callers do not need to log routing separately.
    async fn route(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, RouterError>;

    /// Routes a streaming completion request.
    ///
    /// Same fallback logic as `route`, but returns a `StreamHandle`.
    ///
    /// If the stream is interrupted mid-response, the router does NOT automatically
    /// fall back — the partial response may be useful. Instead, `StreamEvent::StreamError`
    /// is emitted, and the orchestrator decides whether to retry or proceed.
    async fn route_stream(
        &self,
        request: &CompletionRequest,
    ) -> Result<StreamHandle, RouterError>;

    /// Returns the current status of all registered providers.
    ///
    /// Used by the Visual Reasoning Layer to display provider health indicators.
    fn provider_statuses(&self) -> Vec<ProviderStatus>;

    /// Manually marks a specific provider as unavailable.
    ///
    /// Used by the orchestrator when external signals indicate provider issues
    /// (e.g., a webhook from an external monitoring system).
    fn mark_provider_unavailable(&self, provider_name: &str, reason: &str);

    /// Restores a previously unavailable provider to active rotation.
    ///
    /// Called automatically when a rate limit expires, or manually via CLI.
    fn restore_provider(&self, provider_name: &str);

    /// Returns the provider that would be selected for a request without making the call.
    ///
    /// Used by the orchestrator for pre-flight routing decisions (e.g., cost estimation
    /// before approving a plan in PAUL mode).
    fn would_route_to(&self, request: &CompletionRequest) -> Option<String>;

    /// Returns the last routing decision made by this router.
    ///
    /// Used to populate `ReasoningEvent::LlmRouted` and for debugging.
    fn last_routing_decision(&self) -> Option<RoutingDecision>;

    /// Returns the number of providers currently available (not exhausted or rate-limited).
    fn available_provider_count(&self) -> usize;
}
