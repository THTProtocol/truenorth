# TrueNorth — Claude Code Context

## Project Overview
TrueNorth is a single-binary, LLM-agnostic AI orchestration harness written in Rust.

## Architecture
- 9 crates in a Cargo workspace
- `truenorth-core` defines all types and traits (the contract layer)
- Other crates implement those traits
- All external dependencies go through trait objects

## Build Commands
```bash
cargo build --workspace        # Build all crates
cargo test --workspace         # Run all tests
cargo clippy --workspace       # Lint check
cargo fmt --all -- --check     # Format check
```

## Key Patterns
- Trait objects via `Arc<dyn Trait + Send + Sync>`
- `thiserror` for error types, `anyhow` for propagation
- `tokio` async runtime; `spawn_blocking` for SQLite/Tantivy
- `parking_lot` for sync locks (never held across await points)
- `tracing` for all logging (never `println!`)

## Crate Responsibilities
| Crate | Owner of |
|-------|---------|
| core | Types, traits, errors — NO implementations |
| llm | LLM API calls, routing, embedding |
| memory | Storage, search, Obsidian sync |
| tools | Tool execution, WASM sandbox, MCP |
| skills | SKILL.md parsing, trigger matching |
| visual | Event sourcing, Mermaid rendering |
| orchestrator | Agent loop, state machine, R/C/S |
| web | HTTP API, WebSocket, SSE |
| cli | Binary entry point, commands |

## Negative Checklist
See `docs/NEGATIVE_CHECKLIST.md` — violations of these rules are blocking issues.
