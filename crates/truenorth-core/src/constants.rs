/// System-wide constants for TrueNorth.
///
/// All magic numbers and configuration defaults are centralized here.
/// Changing a constant here affects all crates that depend on `truenorth-core`.

// ─── Versioning ───────────────────────────────────────────────────────────────

/// The current TrueNorth version string.
pub const TRUENORTH_VERSION: &str = "0.1.0";

/// The schema version for serialized session state.
/// Increment this when making breaking changes to `SessionState`.
pub const STATE_SCHEMA_VERSION: &str = "1.0.0";

/// The minimum schema version that this build can migrate from.
pub const MIN_COMPATIBLE_SCHEMA_VERSION: &str = "1.0.0";

/// The SKILL.md format version this runtime is compatible with.
pub const SKILL_FORMAT_VERSION: &str = "1.0";

// ─── Context Budget Thresholds ────────────────────────────────────────────────

/// Default context utilization fraction at which compaction is triggered.
pub const DEFAULT_COMPACT_THRESHOLD: f32 = 0.70;

/// Default context utilization fraction at which handoff is triggered.
pub const DEFAULT_HANDOFF_THRESHOLD: f32 = 0.90;

/// Default context utilization fraction at which execution halts.
pub const DEFAULT_HALT_THRESHOLD: f32 = 0.98;

/// Number of tokens reserved for the next LLM response in budget calculations.
pub const DEFAULT_RESPONSE_RESERVE_TOKENS: usize = 4096;

// ─── LLM Routing ─────────────────────────────────────────────────────────────

/// Maximum number of routing loops before declaring all providers exhausted.
/// Loop 1: try all providers. Loop 2: retry all providers. Then halt.
pub const MAX_ROUTING_LOOPS: usize = 2;

/// Number of seconds to wait before retrying a rate-limited provider
/// when no retry-after header was provided.
pub const DEFAULT_RATE_LIMIT_RETRY_SECS: u64 = 60;

/// Maximum number of retries for transient network errors before failing a provider.
pub const MAX_NETWORK_RETRIES: usize = 3;

/// Timeout for a single LLM completion request in milliseconds.
pub const DEFAULT_LLM_TIMEOUT_MS: u64 = 120_000; // 2 minutes

/// Timeout for a streaming LLM request in milliseconds (initial connection).
pub const DEFAULT_STREAM_CONNECT_TIMEOUT_MS: u64 = 30_000; // 30 seconds

// ─── Agent Loop ──────────────────────────────────────────────────────────────

/// Maximum number of steps per task (loop guard).
pub const DEFAULT_MAX_STEPS_PER_TASK: usize = 50;

/// Maximum number of consecutive identical tool calls before declaring an infinite loop.
pub const INFINITE_LOOP_DETECTION_THRESHOLD: usize = 5;

/// Number of seconds to wait before the watchdog timer halts a non-responsive agent.
pub const AGENT_WATCHDOG_TIMEOUT_SECS: u64 = 300; // 5 minutes

// ─── Memory ──────────────────────────────────────────────────────────────────

/// Default semantic similarity threshold for deduplication.
/// Entries with cosine similarity above this are considered duplicates.
pub const DEFAULT_DEDUPLICATION_THRESHOLD: f32 = 0.85;

/// Default deviation threshold for plan fidelity checking.
/// Steps with semantic similarity below this are flagged as deviations.
pub const DEFAULT_DEVIATION_THRESHOLD: f32 = 0.75;

/// Maximum number of search results returned per memory query.
pub const DEFAULT_MEMORY_SEARCH_LIMIT: usize = 10;

/// Maximum content length for a memory entry (in characters).
pub const MAX_MEMORY_ENTRY_LENGTH: usize = 50_000;

/// The local embedding model name used by the default fastembed backend.
pub const DEFAULT_EMBEDDING_MODEL: &str = "all-mini-lm-l6-v2";

/// Dimensionality of the default embedding model (AllMiniLML6V2).
pub const DEFAULT_EMBEDDING_DIMENSION: usize = 384;

// ─── WASM Sandbox ────────────────────────────────────────────────────────────

/// Default maximum memory per WASM instance in bytes (64 MiB).
pub const WASM_DEFAULT_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Default CPU fuel units per WASM execution (~10 million simple operations).
pub const WASM_DEFAULT_MAX_FUEL: u64 = 10_000_000;

/// Default wall-clock timeout per WASM execution in milliseconds.
pub const WASM_DEFAULT_TIMEOUT_MS: u64 = 30_000; // 30 seconds

/// Maximum size of the WASM call stack in bytes (1 MiB).
pub const WASM_DEFAULT_MAX_STACK_BYTES: usize = 1 * 1024 * 1024;

/// Maximum number of table elements (indirect function calls) per WASM instance.
pub const WASM_DEFAULT_MAX_TABLE_ELEMENTS: u32 = 10_000;

// ─── Skills ──────────────────────────────────────────────────────────────────

/// Minimum trigger confidence score for automatic skill activation.
pub const SKILL_TRIGGER_CONFIDENCE_THRESHOLD: f32 = 0.80;

/// Maximum number of skills that can be loaded at Level 1 (Full) simultaneously.
pub const MAX_ACTIVE_SKILLS: usize = 5;

/// Maximum size of a skill body at Level 1 (Full) in tokens.
pub const SKILL_LEVEL1_MAX_TOKENS: usize = 2000;

// ─── Session ─────────────────────────────────────────────────────────────────

/// The directory name for session snapshot files.
pub const SESSIONS_DIR: &str = "sessions";

/// The directory name for the memory store files.
pub const MEMORY_DIR: &str = "memory";

/// The directory name for the embedded model cache.
pub const MODELS_DIR: &str = "models";

/// The default TrueNorth data directory (relative to home).
pub const DEFAULT_DATA_DIR: &str = ".truenorth";

/// The configuration file name.
pub const CONFIG_FILE: &str = "config.toml";

/// The negative checklist file name.
pub const NEGATIVE_CHECKLIST_FILE: &str = "NEGATIVE_CHECKLIST.md";

// ─── Web UI ──────────────────────────────────────────────────────────────────

/// Default port for the TrueNorth web UI server.
pub const DEFAULT_WEB_UI_PORT: u16 = 3000;

/// Default bind address for the web UI server.
pub const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1";

/// WebSocket path for the reasoning event stream.
pub const WEBSOCKET_REASONING_PATH: &str = "/ws/reasoning";

/// Maximum number of events to buffer in the WebSocket broadcast channel.
pub const WEBSOCKET_CHANNEL_CAPACITY: usize = 1024;

// ─── Heartbeat ───────────────────────────────────────────────────────────────

/// Default poll interval for the heartbeat scheduler in milliseconds.
pub const HEARTBEAT_POLL_INTERVAL_MS: u64 = 1_000; // 1 second

/// Maximum consecutive failures before suspending a heartbeat registration.
pub const DEFAULT_MAX_HEARTBEAT_FAILURES: u32 = 3;

// ─── Reasoning Events ─────────────────────────────────────────────────────────

/// Maximum number of live subscribers to the reasoning event broadcast channel.
/// Additional subscribers beyond this limit will receive an error.
pub const MAX_EVENT_SUBSCRIBERS: usize = 64;

/// Maximum number of events to retain per session in the in-memory buffer.
/// Older events are always available in SQLite.
pub const EVENT_BUFFER_SIZE: usize = 500;
