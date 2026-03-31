# ADR-0001: All-Rust Architecture

## Status: Accepted

## Context

TrueNorth could be built with TypeScript (Node.js), Python, or Rust. The decision affects performance, deployment complexity, type safety, and the developer experience.

## Decision

Build the entire stack in Rust, including the frontend (Leptos), backend (Axum), and all subsystems.

## Rationale

1. **Single binary deployment** — `cargo build --release` produces one executable. No runtime dependencies, no `node_modules`, no Python virtualenv.
2. **Type safety across the full stack** — Traits enforce contracts between crates at compile time. A malformed tool result or mistyped memory entry is caught before it reaches production.
3. **Performance** — Rust's zero-cost abstractions mean the orchestration loop adds negligible overhead. The bottleneck is always the LLM API, not the harness.
4. **Memory safety** — No garbage collector pauses, no null pointer exceptions, no data races. The borrow checker eliminates entire classes of bugs.
5. **WASM ecosystem** — Wasmtime is the reference WASM runtime, written in Rust. Native integration is seamless.

## Consequences

- Higher initial development cost (Rust's learning curve)
- Smaller contributor pool than TypeScript/Python
- Leptos is less mature than React (but sufficient for our needs)
- Compile times are longer (mitigated by workspace caching)

## Alternatives Considered

- **TypeScript + Next.js**: Faster initial development, but runtime errors and deployment complexity
- **Python + FastAPI**: Excellent ML ecosystem, but GIL limits concurrency and type safety is opt-in
- **Go**: Good for services, but lacks the trait system and WASM integration
