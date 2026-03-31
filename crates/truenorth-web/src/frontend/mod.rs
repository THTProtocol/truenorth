//! Frontend module — stub placeholder for Leptos SSR integration.
//!
//! # TODO: Leptos Integration
//!
//! This module is intentionally stubbed out.  The full Leptos dependency tree
//! (`leptos`, `leptos_meta`, `leptos_router`, `leptos_axum`) is too heavy for
//! the current build environment.
//!
//! When Leptos is added:
//!
//! 1. Add the following to `Cargo.toml`:
//!    ```toml
//!    [features]
//!    default = ["ssr"]
//!    ssr = ["leptos/ssr", "leptos_meta/ssr", "leptos_router/ssr", "leptos_axum"]
//!    hydrate = ["leptos/hydrate", "leptos_meta/hydrate", "leptos_router/hydrate"]
//!
//!    [dependencies]
//!    leptos = { version = "0.7", features = [] }
//!    leptos_meta = { version = "0.7" }
//!    leptos_router = { version = "0.7" }
//!    leptos_axum = { version = "0.7", optional = true }
//!    ```
//!
//! 2. Replace the stub structs below with real Leptos components.
//!
//! 3. Wire the Leptos router into [`crate::server::router::build_router`] via
//!    `leptos_axum::LeptosRoutes`.
//!
//! ## Planned Pages
//!
//! - [`pages::home`] — Active sessions overview
//! - [`pages::session`] — Conversation + reasoning graph for a single session
//! - [`pages::memory`] — Three-tier memory browser
//! - [`pages::skills`] — Installed skills browser
//! - [`pages::tools`] — Tool registry view
//! - [`pages::settings`] — Configuration display
//!
//! ## Planned Components
//!
//! - `reasoning_graph` — Live Mermaid flowchart (WebSocket driven)
//! - `event_timeline` — Chronological reasoning event feed
//! - `context_gauge` — Context window utilisation meter
//! - `routing_log` — LLM provider routing decision log
//! - `chat_input` — Prompt input with submit + stop controls
//! - `tool_call_card` — Individual tool call display
//! - `memory_entry` — Single memory entry display
//! - `skill_card` — Skill metadata display card
//! - `provider_badge` — LLM provider status badge

// ─── Stub pages module ────────────────────────────────────────────────────────

/// Stub page components — replace with real Leptos components.
pub mod pages {
    /// Home page stub — will show active sessions overview.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that renders the sessions list
    /// and links to individual session views.
    pub struct HomePage;

    /// Session view stub — will show conversation + reasoning graph.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that subscribes to the WebSocket
    /// stream and renders the Mermaid reasoning graph.
    pub struct SessionPage {
        /// The session UUID this page displays.
        pub session_id: uuid::Uuid,
    }

    /// Memory inspector stub — will show the three-tier memory browser.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` backed by `GET /api/v1/memory/search`.
    pub struct MemoryPage;

    /// Skills browser stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` backed by `GET /api/v1/skills`.
    pub struct SkillsPage;

    /// Tool registry view stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` backed by `GET /api/v1/tools`.
    pub struct ToolsPage;

    /// Settings page stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that renders non-secret config.
    pub struct SettingsPage;
}

// ─── Stub components module ───────────────────────────────────────────────────

/// Stub UI components — replace with real Leptos components.
pub mod components {
    /// Reasoning graph component stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that:
    /// - Opens a WebSocket to `GET /api/v1/events/ws`
    /// - Feeds events to a client-side Mermaid.js renderer
    /// - Re-renders the graph on each `TaskGraphSnapshot` event
    pub struct ReasoningGraph {
        /// Session whose events this component tracks.
        pub session_id: uuid::Uuid,
    }

    /// Event timeline component stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that renders a scrolling list
    /// of `ReasoningEvent` entries in chronological order.
    pub struct EventTimeline;

    /// Context window utilisation gauge stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that renders a progress bar
    /// showing `context_tokens / context_budget`.
    pub struct ContextGauge {
        /// Current token count.
        pub used: usize,
        /// Total budget.
        pub budget: usize,
    }

    /// LLM routing decision log stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that shows which provider was
    /// selected for each LLM call and why.
    pub struct RoutingLog;

    /// Chat input component stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that provides a text input,
    /// submit button, and stop button wired to `POST /api/v1/task`.
    pub struct ChatInput;

    /// Tool call card stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that shows a tool name, its input
    /// arguments, and the result (or error) it returned.
    pub struct ToolCallCard {
        /// Name of the tool that was called.
        pub tool_name: String,
    }

    /// Memory entry display stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that renders a single `MemoryEntry`
    /// with its content, scope badge, and importance score.
    pub struct MemoryEntry;

    /// Skill card stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that renders skill metadata:
    /// name, description, version, tags, and trigger phrases.
    pub struct SkillCard {
        /// Skill name.
        pub name: String,
    }

    /// LLM provider status badge stub.
    ///
    /// # TODO
    /// Replace with a Leptos `#[component]` that shows a provider's
    /// availability status (available / rate-limited / exhausted).
    pub struct ProviderBadge {
        /// Provider name (e.g., "anthropic", "openai").
        pub provider: String,
        /// Whether the provider is currently available.
        pub available: bool,
    }
}

// ─── Stub utils module ────────────────────────────────────────────────────────

/// Frontend utility stubs.
pub mod utils {
    /// WebSocket connection management stub.
    ///
    /// # TODO
    /// Replace with a Leptos-compatible WebSocket hook that handles:
    /// - Initial connection to `GET /api/v1/events/ws`
    /// - Automatic reconnection with exponential backoff on disconnect
    /// - Message parsing and dispatch to signal stores
    pub struct WebSocketManager {
        /// Target WebSocket URL.
        pub url: String,
    }

    impl WebSocketManager {
        /// Create a new manager targeting the given URL.
        ///
        /// # TODO
        /// In the real implementation this should initiate the WebSocket
        /// connection and return a handle to control it.
        pub fn new(url: impl Into<String>) -> Self {
            Self { url: url.into() }
        }

        /// Connect to the WebSocket server.
        ///
        /// # TODO
        /// Replace with actual WASM WebSocket API call.
        pub fn connect(&self) {
            // TODO: call browser WebSocket API
        }
    }

    /// Format a token count for display.
    ///
    /// Returns a human-readable string like `"12.3k"` for large counts.
    ///
    /// # TODO
    /// Move to a WASM-compatible utility once the frontend is activated.
    pub fn format_token_count(count: usize) -> String {
        if count >= 1_000_000 {
            format!("{:.1}M", count as f64 / 1_000_000.0)
        } else if count >= 1_000 {
            format!("{:.1}k", count as f64 / 1_000.0)
        } else {
            count.to_string()
        }
    }

    /// Format a UTC timestamp for display relative to now.
    ///
    /// # TODO
    /// Replace with a Leptos reactive timer that updates automatically.
    pub fn format_relative_time(ts: &chrono::DateTime<chrono::Utc>) -> String {
        let now = chrono::Utc::now();
        let delta = now.signed_duration_since(*ts);
        if delta.num_seconds() < 60 {
            "just now".to_string()
        } else if delta.num_minutes() < 60 {
            format!("{}m ago", delta.num_minutes())
        } else if delta.num_hours() < 24 {
            format!("{}h ago", delta.num_hours())
        } else {
            ts.format("%Y-%m-%d").to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utils::*;

    #[test]
    fn format_token_count_small() {
        assert_eq!(format_token_count(42), "42");
    }

    #[test]
    fn format_token_count_thousands() {
        assert_eq!(format_token_count(12_300), "12.3k");
    }

    #[test]
    fn format_token_count_millions() {
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }
}
