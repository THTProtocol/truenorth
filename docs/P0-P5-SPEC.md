# P0-P5 Wiring Specification

## P0: Wire the Binary

### CLI Cargo.toml changes
Add dependencies:
```toml
truenorth-orchestrator = { path = "../truenorth-orchestrator" }
truenorth-llm = { path = "../truenorth-llm" }
truenorth-memory = { path = "../truenorth-memory" }
truenorth-tools = { path = "../truenorth-tools" }
truenorth-skills = { path = "../truenorth-skills" }
truenorth-visual = { path = "../truenorth-visual" }
truenorth-web = { path = "../truenorth-web" }
```

### CLI run.rs — Replace stub with real orchestrator wiring
1. Build `Orchestrator` via `Orchestrator::builder().with_config(...).build()?`
2. Create a `Task` from the user's prompt (use `Task` from `truenorth_core::types::task`)
3. Call `orchestrator.run_task(task).await?`
4. Print the result
5. For now, use default `OrchestratorConfig` — no need to parse config.toml yet

### CLI serve.rs — Replace stub with real web server
1. Build `Orchestrator`, wrap in `Arc`
2. Build `AppState` with the orchestrator reference
3. Start the web server
NOTE: AppState currently doesn't have an orchestrator field — you need to add one.

### CLI resume.rs — Replace stub with session resume
1. Build `Orchestrator`
2. Use `orchestrator.session_manager` to load the session
3. Resume the agent loop

### Web AppState changes
Add an `orchestrator: Option<Arc<Orchestrator>>` field (Option so tests still work without it).
Add `with_orchestrator()` to the builder.

### Web Cargo.toml changes
Add: `truenorth-orchestrator = { path = "../truenorth-orchestrator" }`

### Web handler wiring
- `submit_task`: Create Task from request body, call `orchestrator.run_task()`, return result
- `list_skills`: Return actual skill list if skill registry wired (or empty for now)
- `list_tools`: Same pattern
- `search_memory`: Same pattern

## P1: End-to-End Smoke Test

Create `/home/user/workspace/truenorth/tests/smoke.rs`:
- Build an Orchestrator with MockLlmProvider
- Submit a task via `orchestrator.run_task()`
- Verify it returns a TaskResult::Success
- This proves the full agent loop runs end-to-end

## P3: Live Provider Test Harness

Create `/home/user/workspace/truenorth/crates/truenorth-llm/tests/live_providers.rs`:
- `#[ignore]` tests that call real Anthropic/OpenAI APIs
- Only run when API keys are set: `if std::env::var("ANTHROPIC_API_KEY").is_err() { return; }`

## P4: Memory Integration in Agent Loop

In `AgentLoopExecutor`, the state machine should:
- During `GatheringContext`: call memory search
- After tool results: write to session memory
- On session end: trigger consolidation

Check what `AgentLoopExecutor` currently does and wire in memory calls.

## P5: WASM Sandbox Validation

Create a test that:
1. Compiles a minimal Rust function to WASM
2. Loads it via `WasmtimeHost`
3. Executes with fuel metering
4. Verifies capability denial works
