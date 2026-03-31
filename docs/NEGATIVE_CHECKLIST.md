# TrueNorth Negative Checklist

Anti-patterns that TrueNorth must NEVER exhibit. Every PR must verify against this list.

## Architecture

- [ ] **Never require network for core function.** Memory search, skill loading, and config must work offline.
- [ ] **Never hardcode a single LLM provider.** All provider interactions go through the LlmRouter trait.
- [ ] **Never expose API keys in logs, errors, or responses.** Keys are loaded from env/config and never serialized.
- [ ] **Never bypass the WASM sandbox for tool execution.** All third-party tools run in Wasmtime with fuel limits.
- [ ] **Never silently swallow errors.** Every error must be logged at appropriate level and propagated.

## Agent Loop

- [ ] **Never execute more than max_steps without user confirmation.** Loop guard enforces this.
- [ ] **Never skip the Negative Checklist verification.** Every loop iteration checks the checklist.
- [ ] **Never allow infinite loops.** Semantic similarity detection + step counter + wall-clock watchdog.
- [ ] **Never silently resolve R/C/S conflicts.** Synthesis must explicitly address all Critic objections.

## Memory

- [ ] **Never store raw API keys or secrets in memory tiers.** Memory content is filtered.
- [ ] **Never delete Obsidian vault files without user confirmation.** Read-only by default.
- [ ] **Never skip consolidation scheduling.** Session memory must be promoted to project tier.

## Security

- [ ] **Never serve unauthenticated endpoints when auth token is configured.** Except /health and /.well-known/agent.json.
- [ ] **Never execute shell commands without explicit user approval.** Shell tool requires confirmation.
- [ ] **Never trust WASM module output without validation.** Tool results are validated against declared schemas.

## Performance

- [ ] **Never block the tokio runtime with synchronous I/O.** Use spawn_blocking for SQLite/Tantivy.
- [ ] **Never hold locks across await points.** Use parking_lot for sync locks, tokio::sync for async.
- [ ] **Never allocate unbounded buffers.** All buffers have configurable limits.
