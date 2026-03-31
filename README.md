<div align="center">

# TrueNorth

**Single-binary, LLM-agnostic AI orchestration harness with visual reasoning.**

[![CI](https://github.com/THTProtocol/truenorth/actions/workflows/ci.yml/badge.svg)](https://github.com/THTProtocol/truenorth/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)

*Not a meta-framework. One binary. Full stack. Your rules.*

</div>

---

## What is TrueNorth?

TrueNorth is a self-hosted AI orchestration system that runs as a single Rust binary. It routes prompts through any LLM provider, manages three-tier memory with Obsidian sync, executes tools in WASM sandboxes, and renders every reasoning step as a visual graph.

### Six Non-Negotiable Principles

1. **File-tree-as-program** — The directory structure IS the architecture. Clone it, read it, extend it.
2. **Three-tier memory with Obsidian sync** — Session → Project → Identity, all synced to Markdown files.
3. **LLM Router with cascading fallback** — No single provider dependency. Double-loop cascade across all configured providers.
4. **Visual Reasoning Layer** — Every decision, tool call, and state transition is an observable event rendered as a Mermaid flowchart.
5. **WASM-sandboxed skill system** — Third-party tools run in Wasmtime with fuel limits and capability restrictions.
6. **Reason/Critic/Synthesis embedded in loop** — Adversarial self-review on every complex decision.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    truenorth-cli                         │
│              (clap binary, REPL, commands)               │
├─────────────────────────────────────────────────────────┤
│                    truenorth-web                         │
│           (Axum server, Leptos frontend, SSE)            │
├─────────────────────────────────────────────────────────┤
│                truenorth-orchestrator                    │
│    (agent loop, state machine, R/C/S, strategies)        │
├──────────┬──────────┬──────────┬──────────┬─────────────┤
│ truenorth│ truenorth│ truenorth│ truenorth│  truenorth   │
│   -llm   │ -memory  │  -tools  │ -skills  │   -visual   │
│          │          │          │          │             │
│ Providers│ 3-tier   │ Registry │ SKILL.md │ Event store │
│ Router   │ SQLite   │ WASM     │ Loader   │ Mermaid gen │
│ Embedding│ Tantivy  │ MCP      │ Triggers │ Event bus   │
│ Streaming│ Obsidian │ Built-in │ Registry │ Aggregator  │
├──────────┴──────────┴──────────┴──────────┴─────────────┤
│                    truenorth-core                        │
│          (types, traits, errors, constants)               │
└─────────────────────────────────────────────────────────┘
```

## Quick Start

### From Source

```bash
git clone https://github.com/THTProtocol/truenorth.git
cd truenorth
cp .env.example .env
cp config.toml.example config.toml

# Edit .env with your API keys
# Edit config.toml with your preferences

cargo build --release
./target/release/truenorth run --task "Hello, TrueNorth"
```

### Docker

```bash
docker compose up -d
# Access at http://localhost:8080
```

## Configuration

TrueNorth uses a layered config system:

1. **`config.toml`** — Primary configuration (providers, memory, skills)
2. **`.env`** — Secrets (API keys, auth tokens)
3. **Environment variables** — Override any config value

```toml
# config.toml (minimal)
[llm]
primary_provider = "anthropic"
fallback_chain = ["openai", "ollama"]

[memory]
vault_path = "./vault"
consolidation_interval_secs = 3600

[server]
port = 8080
auth_required = true
```

## Crate Overview

| Crate | Description | Lines |
|-------|-------------|-------|
| `truenorth-core` | Shared types, traits, error types — the contract layer | ~5,100 |
| `truenorth-llm` | LLM providers, cascading router, embedding backends | ~6,500 |
| `truenorth-memory` | Three-tier memory, Tantivy FTS, Obsidian sync | ~5,900 |
| `truenorth-tools` | Tool registry, WASM sandbox, built-in tools, MCP | ~3,700 |
| `truenorth-skills` | SKILL.md parser, loader, trigger matching, registry | ~2,500 |
| `truenorth-visual` | Event store, broadcast bus, Mermaid generation | ~2,700 |
| `truenorth-orchestrator` | Agent loop, state machine, R/C/S, execution strategies | WIP |
| `truenorth-web` | Axum HTTP server, WebSocket, SSE, frontend | WIP |
| `truenorth-cli` | CLI binary, REPL, command dispatch | WIP |

## Supported LLM Providers

- **Anthropic** — Claude (Messages API, extended thinking, streaming)
- **OpenAI** — GPT-4, o-series (Chat Completions API)
- **Google** — Gemini (GenerateContent API)
- **Ollama** — Local inference (OpenAI-compatible)
- **OpenAI-Compatible** — LM Studio, Groq, Together, any compatible endpoint
- **Mock** — Deterministic test provider

## SKILL.md Standard

TrueNorth skills are Markdown files with YAML frontmatter:

```markdown
---
name: research-assistant
version: "1.0.0"
triggers:
  - "research"
  - "find information about"
required_tools:
  - web_search
  - web_fetch
---

# Research Assistant

## Instructions
You are a research assistant. When asked to research a topic...

## Examples
...
```

## Development

```bash
# Run all tests
cargo test --workspace

# Run with logging
RUST_LOG=debug cargo run -- run --task "test"

# Check lints
cargo clippy --workspace

# Format code
cargo fmt --all
```

## License

Apache-2.0. See [LICENSE](LICENSE) for details.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

<div align="center">
<b>Built by <a href="https://github.com/THTProtocol">High Table Protocol</a></b>
</div>
