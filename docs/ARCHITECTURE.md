# TrueNorth Architecture Guide

> **Version**: 0.1.0  
> **Last updated**: 2026-03-31  
> **Audience**: Contributors, integrators, and anyone who wants to understand how TrueNorth works at a systems level.

---

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Crate Map and Dependency Graph](#2-crate-map-and-dependency-graph)
3. [Data Flow: Prompt to Response](#3-data-flow-prompt-to-response)
4. [The Six Non-Negotiable Principles](#4-the-six-non-negotiable-principles)
5. [Key Design Patterns](#5-key-design-patterns)
6. [Error Handling Strategy](#6-error-handling-strategy)
7. [Async Architecture](#7-async-architecture)
8. [Security Model](#8-security-model)
9. [Configuration System](#9-configuration-system)
10. [Observability and Tracing](#10-observability-and-tracing)

---

## 1. System Overview

TrueNorth is a **single-binary, LLM-agnostic AI orchestration harness** written in Rust. It accepts tasks from the CLI or a REST/WebSocket API, routes them through any configured LLM provider, executes tools in WASM sandboxes, persists reasoning in a three-tier memory system, and renders every decision step as a live Mermaid flowchart.

The design philosophy is **zero magic, full ownership**. There are no external orchestration services, no vendor lock-in, and no hidden state. The entire system state at any moment is either in memory (deserializable from SQLite) or in Markdown files that a human can read in Obsidian.

### Full Stack Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        truenorth-cli                             │
│   clap CLI binary · REPL · commands: run, serve, skill, memory  │
│   OutputFormat: Text (coloured) or JSON                          │
├─────────────────────────────────────────────────────────────────┤
│                        truenorth-web                             │
│   Axum HTTP server · Leptos frontend · SSE task streaming        │
│   REST API /api/v1/* · WebSocket /api/v1/events/ws               │
│   Bearer token auth · CORS middleware · error types              │
├─────────────────────────────────────────────────────────────────┤
│                    truenorth-orchestrator                         │
│   Agent loop state machine · R/C/S execution strategy            │
│   Context budget manager · Session lifecycle · Deviation tracker │
│   Negative checklist · Heartbeat scheduler · Loop guard/watchdog │
├──────────────┬───────────────┬──────────────┬───────────────────┤
│ truenorth-   │  truenorth-   │  truenorth-  │  truenorth-visual │
│ llm          │  memory       │  tools       │                   │
│              │               │              │                   │
│ Providers:   │ Session tier  │ DefaultTool  │ EventBus          │
│  Anthropic   │  (in-memory   │ Registry     │ (broadcast ch.)   │
│  OpenAI      │   Arc<RwLock> │              │                   │
│  Google      │  )            │ Built-ins:   │ ReasoningEvent    │
│  Ollama      │               │  web_search  │ Store (SQLite WAL)│
│  OpenAI-compat│ Project tier │  web_fetch   │                   │
│  Mock        │  (SQLite +    │  file_read   │ EventAggregator   │
│              │   Markdown)   │  file_write  │ (background task) │
│ Router:      │               │  file_list   │                   │
│  Double-loop │ Identity tier │  shell_exec  │ MermaidGenerator  │
│  cascade     │  (SQLite)     │  memory_query│ DiagramRenderer   │
│              │               │  mermaid_    │                   │
│ Embedding:   │ SearchEngine  │  render      │ VisualReasoning   │
│  fastembed   │  BM25/Tantivy │              │ Engine (facade)   │
│  openai      │  Semantic     │ WASM sandbox │                   │
│  mock        │  Hybrid/RRF   │ (Wasmtime)   │                   │
│              │               │              │                   │
│ Context      │ Obsidian sync │ MCP adapter  │                   │
│ Serializer   │ (notify fs    │ (HTTP client)│                   │
│ (π-ai)       │  watcher)     │              │                   │
│              │               │              │                   │
│ Rate limiter │ AutoDream     │ Audio tools  │                   │
│  per-provider│ Consolidator  │              │                   │
│  exp. backoff│ Scheduler     │              │                   │
├──────────────┴───────────────┴──────────────┴───────────────────┤
│                       truenorth-skills                            │
│   SkillMarkdownParser · DefaultSkillLoader · SkillRegistry        │
│   TriggerMatcher · SkillValidator · SkillInstaller                │
├─────────────────────────────────────────────────────────────────┤
│                        truenorth-core                             │
│  Types: Task, Plan, Session, Message, MemoryEntry, ToolCall …    │
│  Traits: LlmProvider, LlmRouter, MemoryStore, Tool, Skill …     │
│  Errors: TrueNorthError, LlmError                                │
│  Constants: thresholds, limits, defaults                         │
└─────────────────────────────────────────────────────────────────┘
```

---

## 2. Crate Map and Dependency Graph

### Dependency Rules

TrueNorth enforces a **strict unidirectional dependency graph**:

```
truenorth-core          (no internal dependencies — the foundation)
     ↑
truenorth-llm           (depends on: core)
truenorth-memory        (depends on: core)
truenorth-tools         (depends on: core)
truenorth-skills        (depends on: core)
truenorth-visual        (depends on: core)
     ↑
truenorth-orchestrator  (depends on: core, llm, memory, tools, skills, visual)
     ↑
truenorth-web           (depends on: core, orchestrator, visual)
truenorth-cli           (depends on: core, orchestrator)
```

No crate depends on any crate at the same level or above it. This prevents circular dependencies and ensures that every layer can be tested in isolation.

### Crate Summaries

| Crate | LOC | Role | Key Exports |
|-------|-----|------|-------------|
| `truenorth-core` | ~5,100 | Contract layer — all shared types, traits, errors, constants | `Task`, `Plan`, `Session`, `LlmProvider`, `MemoryStore`, `TrueNorthError` |
| `truenorth-llm` | ~6,500 | All LLM provider calls, cascading router, embeddings | `DefaultLlmRouter`, `ContextSerializer`, `RateLimiter` |
| `truenorth-memory` | ~5,900 | Three-tier memory, Tantivy FTS, Obsidian sync, consolidation | `MemoryLayer`, `SearchEngine`, `AutoDreamConsolidator` |
| `truenorth-tools` | ~3,700 | Tool registry, WASM sandbox, built-in tools, MCP adapter | `DefaultToolRegistry`, `WasmtimeHost`, built-ins |
| `truenorth-skills` | ~2,500 | SKILL.md parser, loader, trigger matching, installation | `SkillMarkdownParser`, `TriggerMatcher`, `SkillRegistry` |
| `truenorth-visual` | ~2,700 | Event bus, event store, Mermaid generation, aggregator | `VisualReasoningEngine`, `EventBus`, `MermaidGenerator` |
| `truenorth-orchestrator` | WIP | Agent loop, state machine, execution strategies, session mgmt | `Orchestrator`, `AgentLoopExecutor`, `RCSExecutionStrategy` |
| `truenorth-web` | WIP | Axum HTTP/WS server, SSE, Leptos frontend | Server, handlers, middleware |
| `truenorth-cli` | WIP | Clap binary, REPL, command dispatch | `Cli`, `run()` |

### Why Each Dependency Exists

**`truenorth-core` → (nothing)**  
The zero-dependency contract layer. Contains only pure data types and trait definitions. Business logic is prohibited. This enables any crate to define a mock implementation for testing without pulling in production dependencies.

**`truenorth-llm` → `core`**  
Implements `LlmProvider` and `LlmRouter` from `core`. Owns all HTTP calls to external APIs. No other crate should ever make raw HTTP calls to LLM providers — that path runs exclusively through `truenorth-llm`.

**`truenorth-memory` → `core`**  
Implements `MemoryStore` from `core`. Owns all SQLite connections for memory tiers and all Tantivy index files. The `MemoryLayer` struct is the sole entry point — no downstream crate should open a SQLite connection directly.

**`truenorth-tools` → `core`**  
Implements `ToolRegistry` and `WasmHost` from `core`. Owns the Wasmtime engine instance. All tool execution (whether built-in, WASM, or MCP-proxied) passes through `DefaultToolRegistry`.

**`truenorth-skills` → `core`**  
Implements `SkillLoader` from `core`. Owns filesystem skill discovery and the YAML frontmatter parser. Skills are pure instruction documents; they contain no Rust code.

**`truenorth-visual` → `core`**  
Implements `ReasoningEventEmitter` from `core`. Owns the broadcast channel and SQLite event store. All components emit events through the `ReasoningEventEmitter` trait, decoupling them from the concrete `EventBus`.

**`truenorth-orchestrator` → all leaf crates**  
The integration layer. Wires together every subsystem. The `Orchestrator` struct holds `Arc<dyn LlmRouter>`, `Arc<MemoryLayer>`, `Arc<dyn ToolRegistry>`, `Arc<SkillRegistry>`, and `Arc<VisualReasoningEngine>`. It is the only crate that coordinates between multiple subsystems.

**`truenorth-web` → `orchestrator`, `visual`**  
Exposes the orchestrator over HTTP/WebSocket. Holds an `Arc<Orchestrator>` and `Arc<VisualReasoningEngine>` in Axum state. SSE task streams relay `ReasoningEvent` from the event bus to connected clients.

**`truenorth-cli` → `orchestrator`**  
Parses CLI arguments with `clap` and dispatches to the orchestrator or web server. The `run` command executes a task; `serve` starts the Axum server; `skill`, `memory`, and `config` commands operate on individual subsystems.

---

## 3. Data Flow: Prompt to Response

This section traces the complete lifecycle of a user prompt, from initial input through the full agent loop to final response.

### Step 0: Entry Point

The user submits a task in one of two ways:

```bash
# CLI
truenorth run --task "Research the latest Rust async patterns"

# REST API
POST /api/v1/task
{ "prompt": "Research the latest Rust async patterns", "stream": true }
```

Both paths converge on the orchestrator's `execute_task()` method.

### Step 1: Session Initialization (`truenorth-orchestrator/session`)

```
User prompt
     │
     ▼
DefaultSessionManager::create_or_resume(session_id)
     │
     ├─ New session: create SessionState { id, created_at, status: Active }
     └─ Resume: load SqliteStateSerializer::load(session_id) → deserialize SessionState
          │
          ▼
     AgentLoopExecutor::start(task, session_state)
```

The session manager assigns a UUID to the task. If a `session_id` is provided (resume), the state serializer loads the previous `SessionState` from SQLite, including the incomplete `Plan`, current `AgentState`, and conversation history.

### Step 2: Context Gathering (`truenorth-orchestrator/agent_loop`)

The state machine transitions: `Idle → GatheringContext`.

```
AgentState::GatheringContext
     │
     ├─ SkillRegistry::match_triggers(prompt) → Option<LoadedSkill>
     │     TriggerMatcher checks keyword, regex, and semantic similarity
     │     Confidence threshold: 0.80 (SKILL_TRIGGER_CONFIDENCE_THRESHOLD)
     │
     ├─ MemoryLayer::search_hybrid(prompt, Project, limit=10) → Vec<MemorySearchResult>
     │     Hybrid = BM25 (Tantivy) + cosine similarity (fastembed), fused with RRF
     │
     └─ MemoryLayer::search_hybrid(prompt, Identity, limit=5) → Vec<MemorySearchResult>
          Retrieves user preferences and long-term patterns
```

Retrieved memory entries and the matched skill (if any) are injected into the conversation context as system message prefixes.

### Step 3: Complexity Assessment (`truenorth-orchestrator/agent_loop`)

```
AgentState::AssessingComplexity
     │
     ▼
ComplexityScore = LlmRouter::complete(complexity_assessment_prompt)
     │
     ├─ ComplexityScore::Simple  → ExecutionMode::Direct
     ├─ ComplexityScore::Moderate → ExecutionMode::Sequential
     ├─ ComplexityScore::Complex  → ExecutionMode::RCS (Reason/Critic/Synthesis)
     └─ ComplexityScore::Graph    → ExecutionMode::Graph (parallel DAG)
```

For simple tasks, execution jumps directly to `Executing` (bypassing Planning).

### Step 4: Planning (`truenorth-orchestrator/agent_loop/planner`)

```
AgentState::Planning
     │
     ▼
Planner::generate_plan(task, context) → Plan
     │  LLM call: "Given this task, generate a step-by-step execution plan."
     │  Response is parsed into Vec<PlanStep> with titles, descriptions, tool hints
     │
     ├─ If require_plan_approval = true → AgentState::AwaitingApproval
     │      User approves/rejects via CLI prompt or WebSocket message
     │
     └─ If autonomous → AgentState::Executing
```

The generated `Plan` is persisted immediately via `SqliteStateSerializer` so that if the process crashes, it can be resumed exactly.

### Step 5: Execution (`truenorth-orchestrator/agent_loop/executor`)

```
AgentState::Executing
     │
     ├─ ExecutionMode::Direct  → DirectExecutionStrategy::execute()
     │     Single LLM call with full context. No planning overhead.
     │
     ├─ ExecutionMode::Sequential → SequentialExecutionStrategy::execute()
     │     Iterate plan steps; for each step: gather context, call LLM, run tools.
     │
     ├─ ExecutionMode::Parallel → ParallelExecutionStrategy::execute()
     │     tokio::spawn one task per independent plan step; join_all.
     │
     ├─ ExecutionMode::Graph → GraphExecutionStrategy::execute()
     │     Topological sort of TaskGraph; execute in dependency order.
     │
     └─ ExecutionMode::RCS → RCSExecutionStrategy::execute()
           See Section 4.6 for full detail.
```

### Step 6: Tool Calls (`truenorth-tools`)

When the LLM response contains a tool call request:

```
AgentState::Executing → AgentState::CallingTool { tool_name, input }
     │
     ▼
DefaultToolRegistry::execute(tool_call, context)
     │
     ├─ Built-in tools → execute directly (web_search, file_read, etc.)
     │
     ├─ WASM tools → WasmtimeHost::execute(module, input)
     │     Fuel metering: 10,000,000 units max
     │     Memory limit: 64 MiB max
     │     Capability check: filesystem/network access requires explicit grant
     │
     └─ MCP tools → McpClient::call(server_url, tool_name, input)
          HTTP POST to external MCP server; response mapped to ToolResult

     ▼
ToolResult → appended to ConversationHistory
     ▼
AgentState::CallingTool → AgentState::Executing
```

Every tool call emits `ReasoningEventPayload::ToolCalled` and `ToolResultReceived` through the event bus, making it visible in the frontend in real time.

### Step 7: Memory Storage (`truenorth-memory`)

After each significant reasoning step or tool result:

```
MemoryLayer::write(content, MemoryScope::Session, metadata)
     │
     ├─ SessionMemoryStore::write_entry(content, metadata)
     │     Arc<RwLock<HashMap<Uuid, MemoryEntry>>> — microsecond writes
     │
     └─ SearchEngine::index_entry(entry)
          Tantivy writer: BM25 index update (spawned in background)

After session ends:
     ▼
AutoDreamConsolidator::run(Project)
     │  Orient → Gather → Consolidate → Prune
     │  LLM call to extract long-term insights from session history
     └─ ProjectMemoryStore::write_entry(consolidated_insight, metadata)
          SQLite WAL-mode write + Markdown file written to vault_dir/
```

### Step 8: LLM Routing (`truenorth-llm/router`)

All LLM calls pass through `DefaultLlmRouter`, which implements the double-loop cascade:

```
DefaultLlmRouter::complete(request)
     │
     ├─ Loop 1: try providers in order [primary, fallback_1, fallback_2, ...]
     │     For each provider:
     │       ├─ RateLimiter::check() — skip if rate-limited
     │       ├─ LlmProvider::complete(request)
     │       │    ├─ Success → return CompletionResponse
     │       │    ├─ LlmError::RateLimited → mark provider, try next
     │       │    ├─ LlmError::NetworkError → retry up to MAX_NETWORK_RETRIES=3
     │       │    ├─ LlmError::ContextWindowExceeded → compact + retry this provider
     │       │    └─ LlmError::ApiKeyExhausted → permanently skip this provider
     │       └─ ContextSerializer::serialize_handoff(partial_state) if switching providers
     │            Translates provider artifacts (Claude thinking traces, OpenAI prefixes)
     │            into a portable HandoffDocument appended to the next provider's context
     │
     └─ Loop 2: retry all non-exhausted providers a second time
          │
          └─ AllProvidersExhausted → TrueNorthError::AllProvidersExhausted { session_id }
               Session state saved. User can resume with: truenorth resume <session_id>
```

### Step 9: Response and Completion

```
AgentState::Executing → AgentState::Complete { output, total_tokens }
     │
     ├─ VisualReasoningEngine::emit(ReasoningEventPayload::SessionComplete)
     │
     ├─ MemoryLayer::notify_session_end(session_id)
     │     ConsolidationScheduler::on_session_end() checks gates:
     │       - Minimum interval (default: 8 hours)
     │       - Minimum new sessions (default: 1)
     │     If both pass → tokio::spawn(AutoDreamConsolidator::run(Project))
     │
     └─ TaskResult::Success(output) → CLI prints / SSE sends "done" event
```

---

## 4. The Six Non-Negotiable Principles

### 4.1 File-Tree-as-Program

The TrueNorth directory structure is a first-class design artifact. Every component maps to a file or directory that can be opened, read, and understood without running any code.

```
~/.truenorth/
├── config.toml              ← All configuration (human-readable)
├── sessions/                ← Serialized agent states (SQLite)
│   └── <session-uuid>.db
├── memory/                  ← Three-tier memory
│   ├── project.db           ← Project-scope SQLite
│   ├── identity.db          ← Identity-scope SQLite
│   ├── tantivy_index/       ← Full-text search index
│   └── vault/               ← Obsidian-compatible Markdown
│       ├── 2026-03-31-research-rust-async.md
│       └── identity/
│           └── preferences.md
├── skills/                  ← Installed skill files
│   ├── research-assistant.md
│   ├── code-reviewer.md
│   └── rcs-debate.md
└── models/                  ← Local embedding model cache
    └── all-mini-lm-l6-v2/
```

The vault directory contains plain Markdown files that can be opened in Obsidian, edited directly, and the changes will be picked up by the filesystem watcher on the next sync.

**Implementation detail**: `MemoryLayerConfig` defaults all paths relative to `~/.truenorth/`. Each path can be overridden individually. The Obsidian vault watcher (`notify::RecommendedWatcher`) detects `.md` file changes and re-indexes them via `ObsidianReindexer`.

### 4.2 Three-Tier Memory with Obsidian Sync

Memory is organized in three tiers with distinct lifetimes, backends, and sync behaviors:

```
┌─────────────────────────────────────────────────────┐
│  Session Tier (MemoryScope::Session)                 │
│  Backend: Arc<RwLock<HashMap<Uuid, MemoryEntry>>>    │
│  Lifetime: Current conversation only                  │
│  Latency: Sub-microsecond (in-process)               │
│  Sync: None (ephemeral, cleared on session end)      │
│  Use: Conversation history, tool results, interim    │
│  work, context that won't outlive the session.       │
├─────────────────────────────────────────────────────┤
│  Project Tier (MemoryScope::Project)                 │
│  Backend: SQLite WAL-mode + Markdown files           │
│  Lifetime: Project scope (survives session restarts) │
│  Latency: ~1ms SQLite write                          │
│  Sync: Bidirectional Obsidian vault sync             │
│  Use: Research findings, decisions, code artifacts,  │
│  anything worth keeping per-project.                  │
├─────────────────────────────────────────────────────┤
│  Identity Tier (MemoryScope::Identity)               │
│  Backend: SQLite WAL-mode                            │
│  Lifetime: Permanent (across all projects)           │
│  Latency: ~1ms SQLite write                          │
│  Sync: Obsidian vault sync (identity/ subdirectory)  │
│  Use: User preferences, communication style,         │
│  long-term patterns from DialecticModeler.           │
└─────────────────────────────────────────────────────┘
```

**Search capabilities**: Every tier is indexed by `SearchEngine`, which provides:
- **Full-text search (BM25)** via Tantivy: `search_text(query, scope, limit)`
- **Semantic search (cosine)** via embedding vectors: `search_semantic(query, scope, top_k)`
- **Hybrid search (RRF)** fusing both signals: `search_hybrid(query, scope, limit)`

**Consolidation**: `AutoDreamConsolidator` runs an Orient → Gather → Consolidate → Prune cycle at configurable intervals (default: minimum 8 hours, minimum 1 new session). It uses an LLM call to extract durable insights from session history and writes them to the project tier.

**Deduplication**: Before writing to the project or identity tier, `ProjectMemoryStore` and `IdentityMemoryStore` compute cosine similarity against existing entries. Entries above `dedup_threshold` (default: 0.85) are merged rather than stored as duplicates.

### 4.3 LLM Router with Cascading Fallback

The `DefaultLlmRouter` implements the **double-loop cascade** strategy: it is the only component in the system that knows about provider failures.

```rust
// The router interface (from truenorth-core)
#[async_trait]
pub trait LlmRouter: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, RouterError>;
    async fn stream(&self, request: CompletionRequest) -> Result<Box<dyn StreamHandle>, RouterError>;
    async fn provider_status(&self) -> Vec<ProviderStatus>;
}
```

**Double-loop cascade**:
- **Loop 1**: Try each configured provider in `[primary, fallback_order...]`. On failure, categorize the error:
  - `RateLimited` → record retry-after, skip to next provider
  - `NetworkError` → retry up to `MAX_NETWORK_RETRIES=3`, then skip
  - `ContextWindowExceeded` → signal orchestrator to compact, retry same provider
  - `ApiKeyExhausted` → permanently skip for this session
  - `ModelRefusal` → do **not** fall back (content issue, not provider issue)
- **Loop 2**: Re-try all non-exhausted providers a second time.
- **Failure**: Emit `AllProvidersExhausted`, save session to SQLite, return error with resume instructions.

**Context serialization (π-ai pattern)**: When switching providers mid-session, `ContextSerializer` translates provider-specific artifacts into a `HandoffDocument` that is prepended to the new provider's context. For example, an Anthropic extended thinking trace is converted into a structured system message that OpenAI can understand.

**Rate limiter**: `RateLimiter` maintains per-provider state (last request time, retry-after window, failure count) with exponential backoff. It runs in memory (no persistence) because rate limit windows are session-scoped.

### 4.4 Visual Reasoning Layer

Every agent action emits a `ReasoningEvent` to the `EventBus`. This provides complete observability without modifying the agent logic.

```rust
// Emit an event (from any component via dependency injection)
emitter.emit(ReasoningEvent::new(
    session_id,
    ReasoningEventPayload::StepStarted {
        task_id, plan_id, step_id,
        step_number: 1,
        title: "Search for recent papers".to_string(),
        description: "Using web_search with query: ...".to_string(),
    },
)).await?;
```

**Event types** (from `ReasoningEventPayload`):
- `TaskReceived`, `PlanCreated` — lifecycle events
- `StateTransition { from, to }` — state machine changes
- `StepStarted`, `StepCompleted` — execution progress
- `ToolCalled`, `ToolResultReceived` — tool invocations
- `LlmRequestSent`, `LlmResponseReceived` — LLM calls
- `MemoryStored`, `MemoryRetrieved` — memory operations
- `RcsReasonComplete`, `RcsCriticComplete`, `RcsSynthesisComplete` — R/C/S phases
- `DeviationDetected` — plan deviation alerts
- `HeartbeatFired` — scheduled task execution
- `SessionComplete` — session end

**EventBus internals**:
- `tokio::sync::broadcast` channel with capacity 1024 (`DEFAULT_CHANNEL_CAPACITY`)
- Every emitted event is synchronously persisted to SQLite WAL-mode (`ReasoningEventStore`) before broadcasting
- `recv_handling_lag()` utility handles `RecvError::Lagged` by reading missed events from the store

**EventAggregator**: A background tokio task that subscribes to the bus and maintains live state snapshots queried by the frontend:
- `active_steps()` — currently running plan steps
- `task_graph_snapshot()` — DAG of tasks and their status
- `context_utilization()` — current token budget usage
- `routing_log()` — recent LLM provider routing decisions

**Mermaid generation**: `MermaidGenerator` is a pure function that takes a `TaskGraphSnapshot` and produces Mermaid flowchart DSL. `DiagramRenderer` wraps the output in SVG/HTML tags. The frontend uses `mermaid.js` to render the diagram client-side.

### 4.5 WASM-Sandboxed Skill System

Third-party tools run inside a Wasmtime sandbox with explicit resource limits. Built-in tools run natively but are still registered through the same `ToolRegistry` interface.

**Sandbox resource limits** (from `constants.rs`):

| Limit | Default | Config key |
|-------|---------|------------|
| Memory | 64 MiB | `sandbox.max_memory_bytes` |
| CPU fuel | 10,000,000 units | `sandbox.max_fuel` |
| Wall-clock timeout | 30 seconds | `sandbox.max_execution_ms` |
| Stack size | 1 MiB | (hardcoded) |
| Table elements | 10,000 | (hardcoded) |

**Capability-based access control** (`CapabilitySet`):
- `Filesystem(allow_paths: Vec<PathBuf>)` — explicit path allowlist
- `Network(allow_hosts: Vec<String>)` — explicit host allowlist
- `Clock` — access to current time
- `Random` — random number generation
- `Stdio` — stdin/stdout (disabled by default)

**Tool registration**:
```rust
// Every tool (built-in or WASM) implements the Tool trait
#[async_trait]
pub trait Tool: Send + Sync {
    fn schema(&self) -> &ToolSchema;
    fn permission_level(&self) -> PermissionLevel;
    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError>;
}

// Permission levels
pub enum PermissionLevel {
    None,    // read-only, no side effects
    Low,     // external reads (web fetch, memory query)
    Medium,  // writes to memory or vault
    High,    // filesystem writes, shell execution
    System,  // system-level operations (requires explicit config)
}
```

**Built-in tools** (always available, native execution):
- `web_search` — web search via configured provider
- `web_fetch` — HTTP GET with content extraction
- `file_read` — read file contents (path validation enforced)
- `file_write` — write file contents (path validation enforced)
- `file_list` — list directory contents
- `shell_exec` — run shell command (High permission, disabled by default)
- `memory_query` — query the memory layer
- `mermaid_render` — render a Mermaid diagram

**MCP adapter**: `McpClient` discovers tools from external MCP-compatible servers via HTTP. Each discovered tool is wrapped as a `McpToolAdapter` that implements `Tool` and proxies calls to the remote server.

### 4.6 Reason/Critic/Synthesis Embedded in Loop

The R/C/S execution strategy is TrueNorth's highest-quality mode. It addresses the **verification laziness** failure mode: when an LLM is asked to verify its own work in the same context window, it tends to confirm expected behavior rather than find genuine flaws.

**Protocol**:

```
┌──────────────────────────────────────────────────────────────┐
│  R/C/S — Three separate LLM calls, each with FRESH context  │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  REASON Phase                                                │
│  Input:  System prompt + task description                    │
│  Output: ReasonOutput { content, tokens_used }               │
│  Prompt: "Produce your best reasoning and plan for..."       │
│                                                              │
│  CRITIC Phase                                                │
│  Input:  System prompt + task + Reason output (fresh ctx)   │
│  Output: CriticOutput { content, approved, issues, tokens } │
│  Prompt: "Find flaws, missing considerations, failure modes" │
│  Note:   If approved=true, skip Synthesis → Complete         │
│                                                              │
│  SYNTHESIS Phase                                             │
│  Input:  System prompt + task + Reason + Critic (fresh ctx) │
│  Output: SynthesisOutput { content, resolved_conflicts }     │
│  Prompt: "Produce final response addressing all criticisms"  │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

**Key design decision**: Each phase receives a **fresh context window** — no conversation history is carried between phases. This is intentional. The Critic's job is to find flaws in the Reason output, which it can only do objectively if it hasn't already been anchored to the Reason's framing by seeing it in context.

**State machine integration**: The R/C/S strategy is represented in the state machine as:
```
Executing → Reasoning { phase: Reason, .. }
         → Reasoning { phase: Critic, .. }
         → Reasoning { phase: Synthesis, .. }  (or Complete if Critic approved)
         → Complete
```

Each phase transition emits a `ReasoningEvent` with the full phase output, making the reasoning process fully observable.

---

## 5. Key Design Patterns

### 5.1 Trait Objects for Dependency Injection

Every major subsystem is accessed through a trait object, enabling:
1. Swappable implementations (e.g., `MockLlmProvider` in tests)
2. Clean separation of interface from implementation
3. No circular dependencies between leaf crates

```rust
// The orchestrator holds trait objects, not concrete types
pub struct Orchestrator {
    llm_router: Arc<dyn LlmRouter>,
    memory: Arc<MemoryLayer>,            // concrete (only one implementation)
    tools: Arc<dyn ToolRegistry>,
    skills: Arc<SkillRegistry>,          // concrete (only one implementation)
    visual: Arc<VisualReasoningEngine>,  // concrete facade
    event_emitter: Arc<dyn ReasoningEventEmitter>,
    session_manager: Arc<dyn SessionManager>,
    state_serializer: Arc<dyn StateSerializer>,
}
```

`async-trait` is used for all async trait definitions because Rust does not yet support async functions in traits natively (though this is stabilizing in 1.75+).

### 5.2 Event Sourcing for Reasoning State

The `ReasoningEventStore` is an append-only SQLite table. Every agent action is recorded as an immutable event. This gives:
- **Full session replay**: `VisualReasoningEngine::replay(session_id, since)` returns all events for a session.
- **Crash recovery**: On restart, the orchestrator can replay events to reconstruct the live state snapshot held by `EventAggregator`.
- **Audit trail**: Every LLM call, tool invocation, and state transition is permanently recorded with timestamps.

### 5.3 Builder Pattern for Complex Construction

`MemoryLayer`, `VisualReasoningEngine`, and `Orchestrator` all use the builder pattern:

```rust
let memory = MemoryLayer::builder()
    .with_config(MemoryLayerConfig {
        memory_root: PathBuf::from("/data/truenorth/memory"),
        watch_vault: true,
        ..Default::default()
    })
    .with_embedding_provider(embedding_arc)
    .with_event_sender(event_tx)
    .build()
    .await?;
```

This pattern handles the complexity of optional components (embedding provider, event sender) without requiring massive constructor signatures.

### 5.4 Arc-Based Shared Ownership

All subsystem instances are wrapped in `Arc<>` to enable sharing across tokio tasks. `MemoryLayer` is explicitly `Clone + Send + Sync`; all internal state is behind `Arc<RwLock<>>` or `Arc<Mutex<>>` as appropriate.

The convention is:
- `Arc<RwLock<T>>` for state that is read-heavy and written rarely (session store HashMap, configuration)
- `Arc<Mutex<T>>` (via `parking_lot`) for state that is written frequently under low contention
- `tokio::sync::broadcast` for one-to-many event distribution

### 5.5 Progressive Skill Loading

Skills are loaded in three levels to minimize token usage:

```
Level 0 (Metadata only):
  name, version, description, triggers, tools_required
  ~50 tokens — loaded for all installed skills at startup

Level 1 (Full body):
  Everything in Level 0 + the complete ## Instructions section
  ~2000 tokens max (SKILL_LEVEL1_MAX_TOKENS)
  Loaded when a skill's triggers match the current prompt

Level 2 (With examples):
  Everything in Level 1 + ## Examples and ## Reference sections
  Loaded when the agent enters a complex multi-step task
  using this skill
```

At most 5 skills can be loaded at Level 1 simultaneously (`MAX_ACTIVE_SKILLS=5`).

---

## 6. Error Handling Strategy

TrueNorth uses a two-layer error handling strategy:

### Layer 1: Domain Errors (thiserror)

Each domain module defines its own typed error enum using `thiserror`. These errors are precise and carry full context:

```rust
// From truenorth-core/src/error.rs
#[derive(Debug, Error, Clone)]
pub enum LlmError {
    #[error("Rate limited by {provider}: retry after {retry_after_secs}s")]
    RateLimited { provider: String, retry_after_secs: u64 },

    #[error("Context window exceeded for {provider}/{model}: {token_count} tokens")]
    ContextWindowExceeded { provider: String, model: String, token_count: usize },

    // ...
}
```

Domain errors: `LlmError`, `MemoryError`, `ToolError`, `SkillError`, `WasmError`, `SessionError`, `ExecutionError`, `ReasoningError`, `DeviationError`, `ChecklistError`, `HeartbeatError`, `BudgetError`, `StateError`, `RegistryError`.

### Layer 2: Root Error (TrueNorthError)

`TrueNorthError` is the application-boundary error type. All domain errors convert into it via `#[from]`:

```rust
pub enum TrueNorthError {
    Llm(#[from] LlmError),
    AllProvidersExhausted { session_id: Uuid },
    Memory(#[from] MemoryError),
    Tool(#[from] ToolError),
    Session(#[from] SessionError),
    Execution(#[from] ExecutionError),
    // ...
    Internal { message: String },  // Bug indicator — always file an issue
}
```

`TrueNorthError` implements two important methods:
- `is_recoverable()` — `true` for rate limits and network errors; the agent should retry or fall back.
- `should_save_state()` — `true` for `AllProvidersExhausted` and `ContextExhausted`; session state is persisted before the error propagates to the user.

### Layer 3: anyhow for Propagation

In CLI handlers and Axum route handlers (application boundaries), errors are propagated with `anyhow::Result`. This provides ergonomic `?` propagation without losing the original error chain. The final display at the CLI or in HTTP responses uses `{:#}` (the "pretty" display) to print the full error chain.

### Error Recovery Hierarchy

```
LlmError::RateLimited         → wait retry_after_secs, try next provider
LlmError::NetworkError        → retry up to MAX_NETWORK_RETRIES=3
LlmError::ContextWindowExceeded → compact context, retry same provider
LlmError::ApiKeyExhausted     → permanently skip provider this session
LlmError::ModelRefusal        → surface to user (content policy issue)
RouterError::AllProvidersExhausted → save state, return resume instructions
ExecutionError::ContextExhausted → save state, return resume instructions
WasmError::FuelExhausted      → tool times out gracefully, execution continues
WasmError::MemoryExhausted    → tool aborted, error surfaced as ToolError
```

---

## 7. Async Architecture

### Runtime

TrueNorth uses a single multi-threaded `tokio` runtime initialized with `#[tokio::main]` in `truenorth-cli/src/main.rs`. The runtime uses all available CPU cores (`tokio::runtime::Builder::new_multi_thread()`).

### spawn_blocking for Synchronous I/O

SQLite (via `rusqlite`) and Tantivy are synchronous libraries. All calls to these libraries are wrapped in `tokio::task::spawn_blocking()` to avoid blocking the async executor thread pool:

```rust
// Pattern used throughout truenorth-memory
pub async fn write_entry(&self, content: String, metadata: HashMap<String, Value>)
    -> Result<MemoryEntry, MemoryError>
{
    let conn = self.pool.clone();
    let entry = tokio::task::spawn_blocking(move || {
        let conn = conn.lock();
        // synchronous SQLite write
        sqlite_insert(&conn, &content, &metadata)
    })
    .await
    .map_err(|e| MemoryError::StorageError { message: e.to_string() })??;

    Ok(entry)
}
```

### Background Tasks

Several subsystems spawn long-running background tasks:

| Component | Task | Cancellation |
|-----------|------|-------------|
| `MemoryLayer` | `ConsolidationScheduler::run_loop()` | Dropped when `MemoryLayer` is dropped |
| `VisualReasoningEngine` | `EventAggregator::spawn()` | Returns `JoinHandle` — caller aborts on shutdown |
| `ObsidianWatcher` | `notify::RecommendedWatcher` — fs events | Dropped with watcher handle |
| `HeartbeatScheduler` | Poll loop for scheduled agent tasks | `JoinHandle` managed by orchestrator |

### Streaming

LLM streaming uses `tokio_stream` and SSE (Server-Sent Events):
- Provider streaming: HTTP chunked response parsed by `truenorth-llm/src/stream.rs` into `StreamEvent` items
- Client streaming: Axum SSE endpoint pushes `StreamEvent` to the HTTP client
- WebSocket: `tokio::sync::broadcast` receiver in the Axum WebSocket handler

### Concurrency in the Tool Registry

`DefaultToolRegistry` uses `parking_lot::RwLock<HashMap<String, Arc<dyn Tool>>>` for the tool map. Registration (write) happens at startup; execution (read) happens at runtime. This is read-heavy, so a `RwLock` allows concurrent tool execution with no contention.

---

## 8. Security Model

### Authentication

When `TRUENORTH_AUTH_TOKEN` is set, all endpoints (except `/health` and `/.well-known/agent.json`) require:

```
Authorization: Bearer <token>
```

The auth middleware in `truenorth-web/src/server/middleware/auth.rs` validates the token using constant-time comparison to prevent timing attacks. The token is loaded from the environment variable at startup and stored in Axum state.

### WASM Sandbox

WASM tools are the primary trust boundary. The sandbox enforces:

1. **Fuel metering**: Each WASM instruction consumes fuel. When the fuel limit (default: 10,000,000 units) is exhausted, execution is terminated with `WasmError::FuelExhausted`. This prevents infinite loops and runaway computation.

2. **Memory limits**: WASM linear memory cannot exceed `WASM_DEFAULT_MAX_MEMORY_BYTES` (64 MiB). Attempts to grow beyond this return a WASM memory growth failure.

3. **Capability-based I/O**: By default, WASM modules have no filesystem or network access. The `CapabilitySet` explicitly grants:
   - Filesystem access to a specific set of allowed paths
   - Network access to a specific set of allowed hostnames
   - WASM modules cannot access paths or hosts not in their capability set

4. **No host function access**: WASM modules cannot call back into the TrueNorth process except through explicitly registered host functions that validate all parameters.

### Tool Permission Levels

The `PermissionLevel` enum enforces a least-privilege model:

| Level | Capabilities | Examples |
|-------|-------------|---------|
| `None` | Read-only, no external state | String manipulation, math |
| `Low` | External reads (web, memory) | `web_fetch`, `memory_query` |
| `Medium` | Memory writes, vault writes | `memory_store` |
| `High` | Filesystem writes, shell | `file_write`, `shell_exec` |
| `System` | System-level operations | Internal operations only |

`High` and `System` permission tools require `config.tools.allow_high_permission = true` to be enabled.

### Path Validation

All file I/O tools validate paths against an allowlist before execution. Paths outside the workspace directory are rejected. Path traversal attacks (`../`) are normalized and validated.

### Secrets Management

API keys and auth tokens are never written to `config.toml`. They must be set as environment variables (e.g., `ANTHROPIC_API_KEY`, `TRUENORTH_AUTH_TOKEN`) or in `.env` (which is `.gitignore`d). The `ProviderConfig.api_key_env` field specifies which env var to read.

---

## 9. Configuration System

TrueNorth uses a three-layer configuration system with the following precedence (highest to lowest):

```
1. Environment variables (TRUENORTH_* prefix overrides config file)
2. config.toml in the data directory
3. Built-in defaults (from TrueNorthConfig::default())
```

### Full Configuration Reference

```toml
# ~/.truenorth/config.toml

[llm]
primary = "anthropic"
fallback_order = ["openai", "ollama"]
default_context_size = 200000
default_max_tokens = 8192
default_temperature = 0.7
enable_thinking = false
thinking_budget = 10000

[[providers]]
name = "anthropic"
model = "claude-opus-4-5"
api_key_env = "ANTHROPIC_API_KEY"
enabled = true

[[providers]]
name = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"
enabled = true

[[providers]]
name = "ollama"
model = "llama3.2"
base_url = "http://localhost:11434"
enabled = true

[memory]
enable_semantic_search = true
embedding_provider = "local"     # "local" (fastembed) or "openai"
max_search_results = 10
deduplication_threshold = 0.85
compact_threshold = 0.70         # trigger compaction at 70% context usage
handoff_threshold = 0.90         # start new context window at 90%
halt_threshold = 0.98            # halt execution at 98%
auto_consolidate = true
model_cache_dir = "~/.truenorth/models"

[sandbox]
enabled = true
max_memory_bytes = 67108864      # 64 MiB
max_fuel = 10000000
max_execution_ms = 30000
allow_clock = true
allow_random = true

data_dir = "~/.truenorth"
skills_dir = "~/.truenorth/skills"
workspace_dir = "."              # defaults to current directory
log_level = "info"
enable_web_ui = true
web_ui_port = 3000
max_steps_per_task = 50
max_routing_loops = 2
require_plan_approval = false
enable_negative_checklist = true
```

### Environment Variable Overrides

| Variable | Overrides |
|----------|-----------|
| `TRUENORTH_LOG_LEVEL` | `log_level` |
| `TRUENORTH_DATA_DIR` | `data_dir` |
| `TRUENORTH_AUTH_TOKEN` | Enables bearer token auth |
| `ANTHROPIC_API_KEY` | Anthropic provider key |
| `OPENAI_API_KEY` | OpenAI provider key |
| `GOOGLE_API_KEY` | Google Gemini provider key |

---

## 10. Observability and Tracing

### Structured Logging

TrueNorth uses `tracing` for all logging. Spans and events are emitted throughout the codebase using `#[instrument]` attributes and `info!`, `debug!`, `warn!`, `error!` macros.

**Initialization**:
```bash
# Human-readable (default)
RUST_LOG=info truenorth run --task "..."

# Filter by module
RUST_LOG=truenorth_llm=debug,truenorth_memory=info truenorth run --task "..."

# JSON output (for log aggregation)
RUST_LOG=debug TRUENORTH_LOG_FORMAT=json truenorth run --task "..."
```

The `tracing-subscriber` is initialized with `EnvFilter` (respects `RUST_LOG`) and either a pretty human-readable format or JSON format (via `tracing_subscriber::fmt::json()`).

### Key Spans

| Span | Crate | Information |
|------|-------|------------|
| `llm_complete` | `truenorth-llm` | provider, model, token counts, latency |
| `memory_write` | `truenorth-memory` | scope, content_len |
| `memory_search` | `truenorth-memory` | scope, query, result_count, latency |
| `tool_execute` | `truenorth-tools` | tool_name, permission_level |
| `wasm_execute` | `truenorth-tools` | module_name, fuel_consumed |
| `skill_load` | `truenorth-skills` | skill_name, level |
| `step_run` | `truenorth-orchestrator` | step_id, strategy |

### Health Check

The `/health` endpoint (unauthenticated) returns:
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_secs": 3600
}
```

This is suitable for container health checks and load balancer probes.

### A2A Agent Card

The `/.well-known/agent.json` endpoint (unauthenticated) exposes a machine-readable agent capability description, enabling agent-to-agent discovery in multi-agent systems.

---

*This document reflects the TrueNorth 0.1.0 architecture. For the latest changes, see the [CHANGELOG](../CHANGELOG.md). For contribution guidelines, see [DEVELOPMENT.md](DEVELOPMENT.md).*
