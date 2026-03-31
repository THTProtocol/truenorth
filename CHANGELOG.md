# Changelog

All notable changes to TrueNorth will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-03-31

### Added

- **truenorth-core** — Shared types, traits, error types, and constants (32 files, ~5,100 lines)
  - 15 trait definitions: LlmProvider, LlmRouter, EmbeddingProvider, MemoryProvider, ToolExecutor, SkillLoader, ExecutionStrategy, AgentLoop, SessionManager, ContextBudgetManager, DeviationTracker, NegativeChecklist, HeartbeatScheduler, StateSerializer, ReasoningEngine
  - 12 type modules: llm, memory, message, event, plan, session, task, tool, skill, config, context, routing
  - Comprehensive error hierarchy: TrueNorthError, LlmError, MemoryError, ToolError, SkillError

- **truenorth-llm** — LLM provider implementations and cascading router (16 files, ~6,500 lines)
  - Providers: Anthropic (Claude), OpenAI (GPT-4), Google (Gemini), Ollama, OpenAI-compatible, Mock
  - Double-loop cascade fallback router
  - Cross-provider context serialization (pi-ai pattern)
  - Embedding: fastembed (local ONNX), OpenAI text-embedding-3-small, Mock
  - Per-provider rate limiting with exponential backoff
  - SSE stream parser

- **truenorth-memory** — Three-tier memory with Obsidian sync (23 files, ~5,900 lines)
  - Session memory: compactor, store
  - Project memory: SQLite store, Markdown writer, deduplicator
  - Identity memory: dialectic modeler, profile manager
  - Search: Tantivy BM25 full-text, semantic vector, hybrid fusion
  - Obsidian sync: file watcher, reindexer, wikilink parser
  - Consolidation: tier promotion scheduler

- **truenorth-tools** — Tool registry and WASM sandbox (19 files, ~3,700 lines)
  - Built-in tools: file_read, file_write, file_list, web_search, web_fetch, shell_exec, memory_query, mermaid_render
  - WASM sandbox: Wasmtime host, fuel metering, capability system
  - MCP adapter: client, adapter for Model Context Protocol
  - Dynamic tool registry with schema validation

- **truenorth-skills** — SKILL.md skill system (7 files, ~2,500 lines)
  - SKILL.md parser: YAML frontmatter + Markdown body
  - Trigger matcher: pattern matching, complexity assessment
  - Skill validator: schema validation, dependency checking
  - Skill registry: registration, lookup, progressive loading
  - Community skill installer: URL-based installation

- **truenorth-visual** — Visual Reasoning Layer (7 files, ~2,700 lines)
  - Event store: SQLite-backed append-only event log
  - Event bus: tokio broadcast channel with replay
  - Mermaid generator: plan → flowchart diagram
  - Aggregator: session statistics and summaries
  - Renderer: Mermaid string rendering

- **truenorth-orchestrator** — Agent loop engine (WIP)
  - State machine: 15+ states with transition rules
  - Execution modes: Direct, Sequential, Parallel, Graph, R/C/S
  - Context budget manager with compaction policies
  - Session management with handoff documents
  - Deviation tracker, negative checklist, heartbeat scheduler
  - Loop guard: step counter, semantic similarity, watchdog

- **truenorth-web** — Axum HTTP server (WIP)
  - REST API: task submission, session management, skill/tool listing
  - WebSocket: Visual Reasoning event stream
  - SSE: LLM response streaming
  - Middleware: Bearer token auth, CORS
  - A2A Agent Card endpoint

- **truenorth-cli** — Command-line interface (14 files, ~1,400 lines)
  - Commands: run, serve, resume, skill, memory, config, version
  - Output: terminal (colored ANSI), JSON mode
  - Runtime initialization with tracing setup

### Infrastructure

- GitHub Actions CI: check, test, clippy, fmt, doc, security audit
- GitHub Actions Release: cross-platform binary builds (Linux, macOS, Windows)
- Docker: multi-stage Dockerfile, docker-compose.yml
- Fly.io: fly.toml deployment config
- Dependabot: weekly Cargo + GitHub Actions updates
- 3 built-in skills: research-assistant, code-reviewer, rcs-debate
- Comprehensive documentation: ARCHITECTURE.md, SKILL_FORMAT.md, DEVELOPMENT.md, DEPLOYMENT.md
- 3 ADRs: All-Rust Architecture, Leptos Frontend, Three-Tier Memory
- NEGATIVE_CHECKLIST.md, SECURITY.md, CONTRIBUTING.md
