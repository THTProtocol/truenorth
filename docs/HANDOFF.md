# TrueNorth — Production Handoff Prompt

> **Phase**: 4 — Handoff  
> **Date**: 2026-03-31  
> **Audience**: Senior engineers and AI coding agents continuing development after Phase 3.  
> **Purpose**: Everything you need to continue TrueNorth development without asking anyone anything.

---

## 1. Project Identity

TrueNorth is a **single-binary, LLM-agnostic AI orchestration harness** written in Rust. It accepts tasks via CLI or REST/WebSocket API, routes them through any configured LLM provider with cascading fallback, executes tools in WASM sandboxes, persists reasoning in a three-tier memory system, and renders every decision step as a live Mermaid flowchart.

- **Repo**: https://github.com/THTProtocol/truenorth  
- **Stack**: Rust 1.80+, tokio, Axum 0.8, Wasmtime 28, SQLite (rusqlite bundled), Tantivy 0.22  
- **License**: Apache-2.0

### Six Non-Negotiable Principles

1. **File-tree-as-program** — The directory structure IS the architecture. Clone it, read it, extend it.
2. **Three-tier memory with Obsidian sync** — Session → Project → Identity, all synced to Markdown files.
3. **LLM Router with cascading fallback** — No single provider dependency; double-loop cascade across all configured providers.
4. **Visual Reasoning Layer** — Every decision, tool call, and state transition is an observable event rendered as a Mermaid flowchart.
5. **WASM-sandboxed skill system** — Third-party tools run in Wasmtime with fuel limits and capability restrictions.
6. **Reason/Critic/Synthesis embedded in loop** — Adversarial self-review on every complex decision.

---

## 2. Current State

### What's Built

| Crate | LOC | Status |
|-------|-----|--------|
| `truenorth-core` | ~5,100 | Complete — all types, traits, errors, constants |
| `truenorth-llm` | ~6,500 | Complete — all 6 providers, cascading router, embeddings |
| `truenorth-memory` | ~5,900 | Complete — 3-tier store, Tantivy FTS, Obsidian sync |
| `truenorth-tools` | ~3,700 | Complete — built-in tools, WASM sandbox, MCP adapter |
| `truenorth-skills` | ~2,500 | Complete — SKILL.md parser, trigger matching, registry |
| `truenorth-visual` | ~2,700 | Complete — event bus, event store, Mermaid generator |
| `truenorth-orchestrator` | ~3,350 | WIP — logic implemented but not wired to binary |
| `truenorth-web` | ~1,500 | WIP — Axum server + routes, Leptos stubs only |
| `truenorth-cli` | ~1,400 | WIP — commands defined but not wired to orchestrator |
| **Total** | **~37,750** | **430 tests** |

### What Compiles and Passes

- `cargo build --workspace` — clean build, all 9 crates
- `cargo test --workspace` — 430 tests pass
- `cargo clippy --workspace` — passes (with known warnings, see §7)
- `cargo fmt --all -- --check` — passes

### What's Wired End-to-End

Each crate works correctly in isolation. Tests for each crate exercise its full internal functionality using mock dependencies.

### What Is NOT Wired

**The binary cannot actually run a task yet.** The three top-layer crates (`truenorth-cli`, `truenorth-web`, `truenorth-orchestrator`) implement all their internal logic but are not connected:

- `truenorth-cli/main.rs` dispatches to placeholder stubs instead of constructing an `Orchestrator`
- `truenorth-web` handlers receive HTTP requests but return stub responses; `AppState` does not hold an `Arc<Orchestrator>`
- `truenorth-orchestrator` is fully implemented but never instantiated in a running process

The primary work is wiring, not new logic: the Orchestrator, WebServer, and CLI all expose the right interfaces. They just need to be plugged together.

---

## 3. Architecture Quick Reference

### Crate Dependency DAG

```
truenorth-core          (no internal dependencies — the foundation)
        ↑
        ├── truenorth-llm      (core)
        ├── truenorth-memory   (core)
        ├── truenorth-tools    (core)
        ├── truenorth-skills   (core)
        └── truenorth-visual   (core)
                ↑
        truenorth-orchestrator (core, llm, memory, tools, skills, visual)
                ↑
        ├── truenorth-web      (core, orchestrator, visual)
        └── truenorth-cli      (core, orchestrator)
```

No crate depends on any crate at the same level or above it. Circular crate dependencies are a hard invariant violation.

### Key Types (10 Most Important)

| Type | Crate | Purpose |
|------|-------|---------|
| `Task` | `truenorth-core::types::task` | The primary unit of work — wraps a user prompt, complexity score, execution mode, and sub-task graph |
| `Plan` | `truenorth-core::types::plan` | Ordered `Vec<PlanStep>` with status tracking; persisted to SQLite immediately on creation |
| `SessionState` | `truenorth-core::types::session` | Full agent state snapshot: `AgentState` enum, `ConversationHistory`, active `Plan`, memory refs |
| `AgentState` | `truenorth-core::traits::state` | State machine enum: `Idle → Intake → GatheringContext → Planning → Executing → Complete` (15+ states) |
| `CompletionRequest` | `truenorth-core::types::llm` | Provider-agnostic LLM call envelope: messages, parameters, tool definitions, stream flag |
| `MemoryEntry` | `truenorth-core::types::memory` | A single piece of stored knowledge with `MemoryScope` (Session/Project/Identity) and metadata |
| `ToolCall` / `ToolResult` | `truenorth-core::types::tool` | Structured LLM-requested tool invocation and its typed response |
| `ReasoningEvent` | `truenorth-core::types::event` | All observable agent events (tool calls, LLM calls, state transitions) broadcast on the event bus |
| `Orchestrator` | `truenorth-orchestrator::orchestrator` | Top-level struct holding `Arc<dyn LlmRouter>`, `Arc<MemoryLayer>`, `Arc<dyn ToolRegistry>`, etc. |
| `TrueNorthConfig` | `truenorth-core::types::config` | Deserialized `config.toml` + env overlay; the single source of truth for all runtime configuration |

### Key Traits (15 Traits)

| Trait | Crate | What It Does |
|-------|-------|--------------|
| `LlmProvider` | `truenorth-core::traits::llm_provider` | Single-provider interface: `complete()`, `stream()`, `count_tokens()`, `capabilities()` |
| `LlmRouter` | `truenorth-core::traits::llm_router` | Multi-provider routing: `complete()` with cascade fallback across all configured providers |
| `EmbeddingProvider` | `truenorth-core::traits::embedding_provider` | Converts text to float vectors: `embed()`, `embed_batch()`, `model_info()` |
| `MemoryStore` | `truenorth-core::traits::memory` | Tiered memory interface: `write()`, `search_hybrid()`, `compact()`, `consolidate()` |
| `Tool` | `truenorth-core::traits::tool` | A single executable tool: `name()`, `schema()`, `execute(ToolCall, ToolContext)` |
| `ToolRegistry` | `truenorth-core::traits::tool` | Registry facade: `register()`, `execute()`, `list_tools()`, dispatches to correct executor |
| `SkillLoader` | `truenorth-core::traits::skill` | Loads a SKILL.md file from disk: `load(path)`, `validate()`, `match_triggers(prompt)` |
| `ExecutionStrategy` | `truenorth-core::traits::execution` | One execution mode: `execute(plan, context)` → `TaskResult`; implement for Direct/Sequential/RCS/etc. |
| `AgentLoop` | `truenorth-core::traits::execution` | Full agent loop lifecycle: `run(task)`, `pause()`, `resume(session_id)`, `cancel()` |
| `SessionManager` | `truenorth-core::traits::session` | Create/resume/list sessions; backed by `SqliteStateSerializer` |
| `ContextBudgetManager` | `truenorth-core::traits::context` | Tracks token usage vs. window budget; triggers compaction at configured thresholds |
| `ReasoningEventEmitter` | `truenorth-core::traits::reasoning` | Emits `ReasoningEvent` onto the broadcast bus; all components use this to publish observability data |
| `StateMachine` | `truenorth-core::traits::state` | Validates and applies `AgentState` transitions; rejects invalid transitions |
| `NegativeChecklist` | `truenorth-core::traits::checklist` | Verifies each loop iteration against all anti-pattern rules; returns `ChecklistReport` |
| `DeviationTracker` | `truenorth-core::traits::deviation` | Detects when execution diverges from plan; emits `DeviationAlert` with severity and recommended action |
| `WasmHost` | `truenorth-core::traits::wasm` | WASM sandbox host: `load_module()`, `execute()` with fuel metering and capability checks |

### Data Flow: Prompt → Response

```
User Input (CLI or POST /api/v1/task)
    │
    ▼
DefaultSessionManager::create_or_resume(session_id)
    │ Loads persisted SessionState from SQLite (if resuming)
    ▼
AgentState: Idle → GatheringContext
    │ SkillRegistry::match_triggers(prompt)         — load relevant SKILL.md
    │ MemoryLayer::search_hybrid(prompt, Project)   — inject past context
    │ MemoryLayer::search_hybrid(prompt, Identity)  — inject user preferences
    ▼
AgentState: AssessingComplexity
    │ LlmRouter::complete(complexity_prompt) → ComplexityScore
    │   Simple → ExecutionMode::Direct
    │   Moderate → ExecutionMode::Sequential
    │   Complex → ExecutionMode::RCS
    │   Graph → ExecutionMode::Graph
    ▼
AgentState: Planning (skipped for Direct)
    │ Planner::generate_plan(task, context) → Plan (persisted immediately)
    ▼
AgentState: Executing
    │ ExecutionStrategy::execute(plan, context)
    │   ↳ LlmRouter::complete / stream (each step)
    │   ↳ DefaultToolRegistry::execute(ToolCall)
    │       ├─ Built-in: direct execution
    │       ├─ WASM: WasmtimeHost (fuel-metered, capability-checked)
    │       └─ MCP: McpClient HTTP call
    │   ↳ ReasoningEventEmitter::emit(event)   — every action observable
    │   ↳ MemoryLayer::write(result, Session)  — store to session tier
    ▼
AgentState: Reflecting → Complete
    │ MemoryLayer::notify_session_end()
    │   AutoDreamConsolidator: Session → Project tier promotion
    ▼
TaskResult::Success(output)
    │ CLI: print to terminal
    └─ Web: SSE "done" event / WebSocket close
```

---

## 4. Invariants (MUST NOT VIOLATE)

These are blocking PR failures. Check every change against this list.

### Architecture

- **No circular crate dependencies.** The DAG in §3 is law. `truenorth-core` has zero internal deps.
- **No raw HTTP calls to LLM APIs outside `truenorth-llm`.** All provider calls go through `LlmRouter`.
- **No direct SQLite connections outside `truenorth-memory`.** `MemoryLayer` is the sole SQLite owner.
- **Network is never required for core function.** Memory search, skill loading, and config must work offline.
- **No single hardcoded LLM provider.** Provider is always resolved through `LlmRouter` trait.

### Agent Loop

- **No blocking on the tokio runtime.** SQLite and Tantivy operations use `tokio::task::spawn_blocking`.
- **No locks held across await points.** Use `parking_lot` for sync locks; `tokio::sync` for async.
- **No infinite loops.** Loop guard enforces: step counter + semantic similarity detection + wall-clock watchdog.
- **No silent error swallowing.** Every error is logged with `tracing` at the appropriate level and propagated.
- **No skipping Negative Checklist.** Every loop iteration must call `NegativeChecklist::verify()`.
- **R/C/S conflicts must be addressed explicitly.** Synthesis cannot silently drop Critic objections.

### Memory

- **No raw secrets in memory tiers.** Content is filtered before storage.
- **No Obsidian vault file deletion without user confirmation.** Vault is read-only by default.
- **No unbounded buffers.** All buffers have configurable limits defined in `truenorth-core::constants`.

### Security

- **Auth required on all endpoints except `/health` and `/.well-known/agent.json`** when `auth_required = true`.
- **Shell tool requires explicit user approval** on every invocation.
- **WASM tool output must be validated** against the declared schema before use.
- **API keys must never appear in logs, error messages, or serialized state.**

### WASM Sandbox

- **All third-party tools must run in Wasmtime.** No `unsafe` execution path bypasses the sandbox.
- **Fuel metering enforced:** 10,000,000 units max per execution.
- **Memory limit enforced:** 64 MiB max per module.
- **Capabilities (filesystem, network) require explicit grant** in the tool's registration metadata.

---

## 5. Development Commands

```bash
# Build everything
cargo build --workspace

# Run all 430 tests
cargo test --workspace

# Lint
cargo clippy --workspace

# Format check (CI gate)
cargo fmt --all -- --check

# Run with debug logging (once binary is wired)
RUST_LOG=debug cargo run -- run --task "Hello, TrueNorth"

# Run tests for one crate only
cargo test -p truenorth-orchestrator

# Build release binary
cargo build --release
# Output: ./target/release/truenorth
```

---

## 6. Priority Work Items (Ordered)

### P0 — Wire the Binary *(Highest priority — unlocks everything)*

The binary entry point (`truenorth-cli/src/main.rs`) currently dispatches to stubs. The work is plumbing, not new logic — all the pieces exist.

**`run` command:**
```rust
// truenorth-cli/src/commands/run.rs
let config = TrueNorthConfig::load(&cli.config_path)?;
let orchestrator = OrchestratorBuilder::new()
    .with_config(config.clone())
    .with_llm_router(build_llm_router(&config.llm)?)
    .with_memory(MemoryLayer::open(&config.memory)?)
    .with_tools(DefaultToolRegistry::with_builtins())
    .with_skills(DefaultSkillLoader::from_dir(&config.skills_dir))
    .with_visual(VisualReasoningEngine::new(&config)?)
    .build()?;

let result = orchestrator.execute_task(task_input).await?;
println!("{}", result.output);
```

**`serve` command:**
```rust
// truenorth-cli/src/commands/serve.rs
let orchestrator = Arc::new(/* same build as above */);
let state = AppState::builder()
    .with_orchestrator(orchestrator)
    .with_auth_token(config.server.auth_token.clone())
    .build();
WebServer::new(state)
    .bind(format!("0.0.0.0:{}", config.server.port))
    .serve()
    .await?;
```

**`AppState` needs an `orchestrator` field** — add `Arc<Orchestrator>` to `AppStateBuilder` in `truenorth-web/src/server/state.rs` and thread it through all task handlers.

**Estimated effort:** 2–4 hours of wiring work.

---

### P1 — End-to-End Smoke Test *(Validates P0 is complete)*

Write integration tests that exercise the full stack:

1. **CLI path:** `cargo run -- run --task "say hello" --provider mock`  
   Expected: process exits 0, output contains the mock provider's response.

2. **HTTP path:**
   ```bash
   cargo run -- serve &
   curl -X POST http://localhost:8080/api/v1/task \
     -H "Authorization: Bearer test-token" \
     -H "Content-Type: application/json" \
     -d '{"prompt": "say hello", "stream": false}'
   ```
   Expected: `{"status": "complete", "output": "..."}` with HTTP 200.

3. **SSE path:** Same POST with `"stream": true` — verify event stream contains `data:` lines and a final `data: [DONE]`.

These tests should live in a top-level `tests/` integration test directory, not inside any crate.

---

### P2 — Leptos Frontend *(Replace stubs with real UI)*

`truenorth-web/src/frontend/` currently contains placeholder stubs. Replace with working Leptos components:

- **Chat interface:** Text input → POST to `/api/v1/task` → render streaming SSE response
- **Reasoning graph panel:** WebSocket connection to `/api/v1/events/ws` → parse `ReasoningEvent` JSON → render as Mermaid diagram (use the `mermaid.js` CDN or the built-in `MermaidGenerator` output)
- **Session list:** GET `/api/v1/sessions` → display resumable sessions

The Leptos ADR (`docs/adr/002-leptos-frontend.md`) captures the rationale for Leptos over HTMX or React.

---

### P3 — Live Provider Testing *(Validate real-world behavior)*

All current tests use `MockLlmProvider`. Before claiming provider support is production-ready:

1. Set `ANTHROPIC_API_KEY` and run: `cargo test -p truenorth-llm -- --ignored live`
2. Set `OPENAI_API_KEY` and run the same.
3. Trigger a rate-limit scenario and verify cascade fallback promotes to the next provider.
4. Verify `ContextSerializer` correctly serializes a Claude thinking trace for handoff to OpenAI.

Mark live tests with `#[ignore]` and `// LIVE TEST` so CI doesn't run them without keys.

---

### P4 — Memory Integration *(Wire memory into the live agent loop)*

Memory currently works in isolation. Integration steps:

1. In `AgentLoopExecutor`, call `MemoryLayer::search_hybrid()` during `GatheringContext` state and inject results into the system prompt prefix.
2. After each `ToolResult`, call `MemoryLayer::write(content, MemoryScope::Session, metadata)`.
3. On session end, call `MemoryLayer::notify_session_end(session_id)` to trigger the `AutoDreamConsolidator`.
4. Test Obsidian vault sync: create a file in `vault_dir/`, verify it appears in `MemoryLayer::search_hybrid()` results within one watcher cycle.
5. Write an integration test that verifies a session memory entry is promoted to project tier after consolidation.

---

### P5 — WASM Tool Sandbox *(Validate WASM execution path)*

The `WasmtimeHost` is implemented but untested with real `.wasm` modules:

1. Compile a minimal Rust tool to WASM:
   ```bash
   cargo new --lib wasm-hello-tool
   # Add: [lib] crate-type = ["cdylib"]
   cargo build --target wasm32-unknown-unknown --release
   ```
2. Register the `.wasm` file with `DefaultToolRegistry::register_wasm()`.
3. Have the LLM (mock or live) call the tool and verify:
   - Fuel metering stops execution at the configured limit.
   - A module requesting filesystem access without `capability_filesystem = true` gets `WasmError::CapabilityDenied`.
   - A valid execution returns the expected `ToolResult`.

---

## 7. Known Issues & Debt

### Compiler Warnings

The workspace compiles clean but has warnings that should be resolved before the first release:

| Crate | Approx. count | Nature |
|-------|---------------|--------|
| `truenorth-orchestrator` | ~12 | Unused fields in WIP structs; dead_code on unconnected methods |
| `truenorth-llm` | ~32 | Unused import paths from provider impls; unused `allow` attributes |
| `truenorth-memory` | ~12 | Unused struct fields in consolidator; unreachable match arms |

Run `cargo build --workspace 2>&1 | grep "^warning"` to get the full current list.

### Leptos Not Integrated

`truenorth-web/src/frontend/` contains stub modules that return placeholder HTML. The Leptos dependency is in `Cargo.toml` but no real components are implemented. The HTTP server works; the frontend does not.

### No Live API Tests

All 430 tests use `MockLlmProvider`. There are zero tests that call actual Anthropic, OpenAI, Google, or Ollama endpoints. Live behavior (streaming, rate limits, token counting, model-specific quirks) is untested.

### Dockerfile Not Tested

`Dockerfile` exists and the multi-stage build is syntactically correct, but the binary entry point is not yet wired (P0), so `docker compose up` will build but the container will exit immediately. Fix P0 first, then test the Docker build.

### Session Resume Not Exercised

`SqliteStateSerializer` is implemented and tested in isolation, but the CLI `resume` command is a stub. This is part of the P0 wiring work.

---

## 8. Key Files to Read First

Read in this order — each file builds on the previous:

| Order | File | Why Read It |
|-------|------|-------------|
| 1 | `crates/truenorth-core/src/lib.rs` | All re-exports in one place; maps the entire type/trait surface |
| 2 | `docs/ARCHITECTURE.md` | Full system diagram, data flow, and rationale for every design decision |
| 3 | `docs/NEGATIVE_CHECKLIST.md` | Anti-patterns that are blocking issues — read before writing a single line |
| 4 | `crates/truenorth-orchestrator/src/orchestrator/mod.rs` | `OrchestratorBuilder` — the assembly point; shows exactly what needs wiring |
| 5 | `crates/truenorth-orchestrator/src/agent_loop/executor.rs` | The state machine loop itself — the heart of the system |
| 6 | `crates/truenorth-cli/src/main.rs` | The current binary entry point — where P0 work begins |
| 7 | `crates/truenorth-web/src/server/state.rs` | `AppState` and `AppStateBuilder` — needs `Arc<Orchestrator>` field added |
| 8 | `crates/truenorth-web/src/server/handlers/` | Current route handlers — all need orchestrator wiring |
| 9 | `crates/truenorth-llm/src/router/mod.rs` | `DefaultLlmRouter` — the cascade fallback implementation |
| 10 | `crates/truenorth-memory/src/layer.rs` | `MemoryLayer` — the unified memory facade |

---

## 9. Contact & Context

| Field | Value |
|-------|-------|
| Owner | THTProtocol |
| Contact | hightable.market@gmail.com |
| Repo | https://github.com/THTProtocol/truenorth |
| Created | 2026-03-31 |
| License | Apache-2.0 |

### Development Phases

| Phase | Description | Status |
|-------|-------------|--------|
| Phase 1 | Research paper — architecture, memory model, agent loop theory | Complete |
| Phase 2 | System design — ADRs, crate map, data flow, API contracts | Complete |
| Phase 3 | Implementation — all 9 crates, 37,750 LOC, 430 tests | Complete |
| Phase 4 | Handoff — this document, binary wiring (P0), integration testing | In progress |

### Immediate Next Action

The single highest-value action is **wiring `truenorth-cli/src/commands/run.rs`** to construct an `Orchestrator` via `OrchestratorBuilder` and call `execute_task()`. Everything else in the priority list depends on this working. Estimated time: 2–4 hours. The test for "done": `cargo run -- run --task "say hello"` exits 0 with output.

---

*Document generated: 2026-03-31. This handoff reflects the state of `main` at the end of Phase 3.*
