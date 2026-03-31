/// Configuration types — the full TrueNorth configuration schema.
///
/// All configuration is loaded from `config.toml` in the TrueNorth data directory.
/// The configuration schema is versioned. Unrecognized fields are ignored to
/// allow forward compatibility.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The root configuration struct for TrueNorth.
///
/// Deserializes from `config.toml`. All fields are optional with sensible defaults
/// so that a minimal config file works out of the box.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrueNorthConfig {
    /// LLM provider configuration.
    pub llm: LlmConfig,
    /// Memory system configuration.
    pub memory: MemoryConfig,
    /// WASM sandbox configuration.
    pub sandbox: SandboxConfig,
    /// Individual provider configurations (primary + fallbacks).
    pub providers: Vec<ProviderConfig>,
    /// Path to the data directory (defaults to ~/.truenorth).
    pub data_dir: PathBuf,
    /// Path to the skills directory.
    pub skills_dir: PathBuf,
    /// Path to the workspace root (project files).
    pub workspace_dir: PathBuf,
    /// Log level: "error", "warn", "info", "debug", "trace".
    pub log_level: String,
    /// Whether to enable the web UI server.
    pub enable_web_ui: bool,
    /// The port to bind the web UI server on.
    pub web_ui_port: u16,
    /// Maximum number of steps per task before halting (loop guard).
    pub max_steps_per_task: usize,
    /// Maximum number of LLM routing loops before halting.
    pub max_routing_loops: usize,
    /// Whether to require user approval for plans (PAUL mode).
    pub require_plan_approval: bool,
    /// Whether to enable the negative checklist verifier.
    pub enable_negative_checklist: bool,
}

impl Default for TrueNorthConfig {
    fn default() -> Self {
        let home = dirs_placeholder::home_dir();
        Self {
            llm: LlmConfig::default(),
            memory: MemoryConfig::default(),
            sandbox: SandboxConfig::default(),
            providers: vec![],
            data_dir: home.join(".truenorth"),
            skills_dir: home.join(".truenorth").join("skills"),
            workspace_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            log_level: "info".to_string(),
            enable_web_ui: true,
            web_ui_port: 3000,
            max_steps_per_task: 50,
            max_routing_loops: 2,
            require_plan_approval: false,
            enable_negative_checklist: true,
        }
    }
}

/// Helper module to resolve home directory without pulling in `dirs` crate.
mod dirs_placeholder {
    use std::path::PathBuf;
    pub fn home_dir() -> PathBuf {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// LLM provider routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// The name of the primary (preferred) provider.
    /// Must match a name in `providers`.
    pub primary: String,
    /// Fallback provider names in priority order.
    /// The router tries these in order if the primary fails.
    pub fallback_order: Vec<String>,
    /// Default context window size in tokens.
    pub default_context_size: usize,
    /// Default maximum tokens for generation.
    pub default_max_tokens: u32,
    /// Default temperature for generation.
    pub default_temperature: f32,
    /// Whether to enable extended thinking by default.
    pub enable_thinking: bool,
    /// Default thinking budget in tokens.
    pub thinking_budget: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            primary: "anthropic".to_string(),
            fallback_order: vec!["openai".to_string()],
            default_context_size: 200_000,
            default_max_tokens: 8192,
            default_temperature: 0.7,
            enable_thinking: false,
            thinking_budget: 10_000,
        }
    }
}

/// Memory system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Whether semantic (embedding-based) search is enabled.
    pub enable_semantic_search: bool,
    /// The embedding provider backend: "local" (fastembed) or "openai".
    pub embedding_provider: String,
    /// Maximum number of search results to return per query.
    pub max_search_results: usize,
    /// Semantic deduplication similarity threshold (0.0–1.0).
    pub deduplication_threshold: f32,
    /// Context compaction threshold (0.0–1.0).
    /// Compaction triggers when context utilization exceeds this value.
    pub compact_threshold: f32,
    /// Handoff threshold (0.0–1.0).
    /// A new context window is started when utilization exceeds this.
    pub handoff_threshold: f32,
    /// Halt threshold (0.0–1.0).
    /// Execution halts when utilization exceeds this.
    pub halt_threshold: f32,
    /// Whether to auto-consolidate memory between sessions.
    pub auto_consolidate: bool,
    /// Path to the local embedding model cache directory.
    pub model_cache_dir: PathBuf,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        let home = dirs_placeholder::home_dir();
        Self {
            enable_semantic_search: true,
            embedding_provider: "local".to_string(),
            max_search_results: 10,
            deduplication_threshold: 0.85,
            compact_threshold: 0.70,
            handoff_threshold: 0.90,
            halt_threshold: 0.98,
            auto_consolidate: true,
            model_cache_dir: home.join(".truenorth").join("models"),
        }
    }
}

/// WASM sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Maximum memory per WASM instance in bytes.
    pub max_memory_bytes: usize,
    /// CPU fuel units per WASM execution.
    pub max_fuel: u64,
    /// Wall-clock timeout per WASM execution in milliseconds.
    pub max_execution_ms: u64,
    /// Whether WASM sandboxing is enabled.
    /// If false, WASM tools execute with native permissions (development only).
    pub enabled: bool,
    /// Whether to allow WASM modules to access the system clock.
    pub allow_clock: bool,
    /// Whether to allow WASM modules to generate random numbers.
    pub allow_random: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MiB
            max_fuel: 10_000_000,
            max_execution_ms: 30_000,
            enabled: true,
            allow_clock: true,
            allow_random: true,
        }
    }
}

/// Configuration for a single LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// The provider's name (e.g., "anthropic", "openai", "ollama").
    pub name: String,
    /// The specific model to use (e.g., "claude-opus-4-5").
    pub model: String,
    /// API key (may also be set via environment variable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Environment variable name to read the API key from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Base URL override (for Ollama, OpenAI-compatible endpoints, proxies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Provider-specific extra configuration as key-value pairs.
    #[serde(default)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
    /// Whether this provider is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}
