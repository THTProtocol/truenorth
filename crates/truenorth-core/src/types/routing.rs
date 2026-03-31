/// Routing types — the LLM router's decision and status types.
///
/// The router is the only component that calls LLM providers directly.
/// These types represent routing decisions, provider health, and errors
/// that occur at the routing layer (distinct from provider-level errors).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A record of a single routing decision.
///
/// Emitted as a `ReasoningEvent::LlmRouted` after every successful completion.
/// Used by the Visual Reasoning Layer to display which provider was used
/// and whether any fallback occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// The provider that was ultimately selected.
    pub selected_provider: String,
    /// The model used by the selected provider.
    pub selected_model: String,
    /// Which routing loop this was (1 = first attempt, 2 = after full sweep failure).
    pub loop_number: usize,
    /// Which attempt within the loop this was (1 = primary, 2+ = fallbacks).
    pub attempt_number: usize,
    /// Providers that were skipped before this one was selected.
    pub providers_skipped: Vec<SkippedProvider>,
    /// Wall-clock latency of the provider call in milliseconds.
    pub latency_ms: u64,
    /// When this routing decision was made.
    pub decided_at: DateTime<Utc>,
}

/// A provider that was skipped during routing, with the reason it was skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedProvider {
    /// The provider name.
    pub name: String,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// The reason a provider was skipped during routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkipReason {
    /// Provider is rate-limited; retry-after time is included.
    RateLimited { retry_after_secs: u64 },
    /// Provider API key is exhausted or invalid.
    Exhausted,
    /// Provider doesn't support a required capability.
    MissingCapability { capability: String },
    /// Provider was manually marked unavailable.
    ManuallyDisabled,
    /// Provider returned an error on a previous attempt.
    PreviousError { error: String },
}

/// The current health status of a single registered provider.
///
/// Returned by `LlmRouter::provider_statuses()` for display in the
/// Visual Reasoning Layer's provider health panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// Provider name (matches `LlmProvider::name()`).
    pub name: String,
    /// The model currently in use.
    pub model: String,
    /// Whether this provider is currently usable.
    pub available: bool,
    /// If rate-limited, when the limit expires.
    pub rate_limit_expires: Option<DateTime<Utc>>,
    /// Whether this provider has been permanently marked as exhausted.
    pub exhausted: bool,
    /// Number of successful completions this session.
    pub success_count: u64,
    /// Number of failures this session.
    pub failure_count: u64,
    /// Last time this provider successfully responded.
    pub last_success_at: Option<DateTime<Utc>>,
    /// Last time this provider failed.
    pub last_failure_at: Option<DateTime<Utc>>,
}

/// Errors specific to the routing layer, distinct from provider-level errors.
///
/// Router errors represent systemic failures (all providers exhausted) as
/// opposed to individual provider failures that the router can work around.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum RouterError {
    /// Every configured provider has been tried `max_loops` times and all failed.
    ///
    /// Session state has been saved. The operator should restore API key
    /// availability and resume with `truenorth resume`.
    #[error(
        "All {provider_count} providers exhausted after {loops} loop(s). \
         Session state saved to {snapshot_path}. \
         Resume with: truenorth resume {session_id}"
    )]
    AllProvidersExhausted {
        provider_count: usize,
        loops: usize,
        session_id: String,
        snapshot_path: String,
    },

    /// The request cannot be routed because it requires a capability
    /// no available provider supports.
    #[error("No available provider supports the required capabilities: {required:?}")]
    NoCapableProvider { required: Vec<String> },

    /// The router was called with an empty provider list.
    #[error("Router has no configured providers")]
    NoProvidersConfigured,

    /// A model refusal — not a routing issue, should not cause provider fallback.
    #[error("Model refused to generate (not a routing issue): {reason}")]
    ContentRefusal { reason: String },

    /// The router itself encountered an internal error.
    #[error("Router internal error: {message}")]
    Internal { message: String },
}
