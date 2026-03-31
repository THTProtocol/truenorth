# TrueNorth Developer Guide

> **Version**: 0.1.0  
> **Rust edition**: 2021  
> **MSRV**: 1.80  
> **Last updated**: 2026-03-31

This guide covers everything you need to contribute to TrueNorth: environment setup, crate-by-crate development, adding new providers and tools, the testing strategy, CI/CD, and debugging tips.

For system architecture, see [ARCHITECTURE.md](ARCHITECTURE.md). For deployment, see [DEPLOYMENT.md](DEPLOYMENT.md).

---

## Table of Contents

1. [Getting Started](#1-getting-started)
2. [Workspace Structure](#2-workspace-structure)
3. [Crate-by-Crate Development Guide](#3-crate-by-crate-development-guide)
4. [How to Add a New LLM Provider](#4-how-to-add-a-new-llm-provider)
5. [How to Add a New Built-in Tool](#5-how-to-add-a-new-built-in-tool)
6. [How to Add a New Execution Strategy](#6-how-to-add-a-new-execution-strategy)
7. [Testing Strategy](#7-testing-strategy)
8. [CI/CD Pipeline](#8-cicd-pipeline)
9. [Release Process](#9-release-process)
10. [Code Style Guide](#10-code-style-guide)
11. [Debugging Tips](#11-debugging-tips)

---

## 1. Getting Started

### Prerequisites

| Tool | Minimum version | Install |
|------|----------------|---------|
| Rust | 1.80 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| cargo | Bundled with Rust | — |
| Git | 2.x | OS package manager |
| SQLite | 3.x (bundled) | Not required — rusqlite bundles SQLite |
| Docker | 20.x (optional) | [docs.docker.com](https://docs.docker.com/get-docker/) |

**Optional** (for specific features):
- **ONNX Runtime**: Required only if building with `--features local-embeddings`
- **Wasmtime**: Bundled as a crate dependency — no system install needed
- **Node.js**: Not required — Mermaid.js is loaded from CDN by the frontend

### Clone and Build

```bash
# Clone the repository
git clone https://github.com/THTProtocol/truenorth.git
cd truenorth

# Copy example configuration files
cp .env.example .env
cp config.toml.example config.toml

# Edit .env with your API keys (at minimum one LLM provider)
# At minimum:
#   ANTHROPIC_API_KEY=sk-ant-...
# or:
#   OPENAI_API_KEY=sk-...

# Build in debug mode (fast compile, unoptimized)
cargo build

# Build in release mode (slow compile, optimized, stripped binary)
cargo build --release

# Run a quick smoke test
./target/debug/truenorth version
```

### First Run

```bash
# Run a simple task (uses mock provider if no API key is set)
./target/debug/truenorth run --task "Hello, TrueNorth"

# Start the web server
./target/debug/truenorth serve --port 8080

# Run with verbose output
./target/debug/truenorth -vvv run --task "test task"
```

### Development Loop

The typical development loop:

```bash
# Fast feedback: check compilation without linking
cargo check --workspace

# Run tests for a specific crate
cargo test -p truenorth-llm

# Run all tests
cargo test --workspace

# Check lints (required before submitting a PR)
cargo clippy --workspace -- -D warnings

# Format code (required before submitting a PR)
cargo fmt --all

# Generate and view documentation
cargo doc --workspace --no-deps --open
```

---

## 2. Workspace Structure

```
truenorth/
├── Cargo.toml              # Workspace root — shared dependencies
├── rustfmt.toml            # Formatting rules
├── clippy.toml             # Lint configuration
├── rust-toolchain.toml     # Pinned toolchain version
├── .env.example            # Example environment variables
├── config.toml.example     # Example configuration
├── fly.toml                # Fly.io deployment configuration
├── Dockerfile              # Container build
├── docker-compose.yml      # Local Docker development
│
├── crates/
│   ├── truenorth-core/         # Contract layer (types, traits, errors)
│   ├── truenorth-llm/          # LLM providers and router
│   ├── truenorth-memory/       # Three-tier memory system
│   ├── truenorth-tools/        # Tool registry and WASM sandbox
│   ├── truenorth-skills/       # Skill loading and parsing
│   ├── truenorth-visual/       # Event bus and Mermaid generation
│   ├── truenorth-orchestrator/ # Agent loop and execution strategies
│   ├── truenorth-web/          # Axum HTTP server
│   └── truenorth-cli/          # Clap CLI binary
│
└── docs/
    ├── ARCHITECTURE.md
    ├── SKILL_FORMAT.md
    ├── DEVELOPMENT.md
    ├── DEPLOYMENT.md
    └── api/
        ├── rest.md
        └── websocket.md
```

### Workspace Cargo.toml

All dependency versions are declared once in the workspace root `Cargo.toml` under `[workspace.dependencies]`. Individual crates inherit versions with `{ workspace = true }`. To upgrade a dependency, change it once in the root — no need to touch individual crate `Cargo.toml` files.

```toml
# workspace Cargo.toml — add a new dependency here
[workspace.dependencies]
my-new-crate = "1.0"

# crate Cargo.toml — use it like this
my-new-crate.workspace = true
```

---

## 3. Crate-by-Crate Development Guide

### 3.1 truenorth-core

**Role**: The zero-dependency contract layer. Contains only types, traits, errors, and constants.

**Adding a new type**:
1. Create `crates/truenorth-core/src/types/<your-type>.rs`
2. Derive `Debug`, `Clone`, `Serialize`, `Deserialize` if the type will cross process boundaries
3. Add `pub mod <your-type>;` in `src/types/mod.rs`
4. Re-export from `src/lib.rs` for ergonomic access: `pub use types::your_type::YourType;`

**Adding a new trait**:
1. Create `crates/truenorth-core/src/traits/<your-trait>.rs`
2. Use `#[async_trait]` for async methods
3. Add `pub mod <your-trait>;` in `src/traits/mod.rs`
4. Re-export from `src/lib.rs`

**Rule**: No business logic in `truenorth-core`. If you find yourself writing a `match` statement with real decisions, the code belongs in a leaf crate.

**Adding a new error variant**:
```rust
// In crates/truenorth-core/src/error.rs
pub enum TrueNorthError {
    // ... existing variants ...

    #[error("Your new error: {message}")]
    YourNewError { message: String },
}
```

**Adding a new constant**:
```rust
// In crates/truenorth-core/src/constants.rs
/// Documentation for this constant.
pub const MY_NEW_CONSTANT: usize = 42;
```

### 3.2 truenorth-llm

**Role**: All LLM provider HTTP calls, the cascading router, and embedding backends.

**Key types**: `DefaultLlmRouter`, `ContextSerializer`, `RateLimiter`, provider structs.

**When to work in this crate**: Adding a new LLM provider, changing routing behavior, modifying rate limiting, adding embedding backends. See [Section 4](#4-how-to-add-a-new-llm-provider) for a step-by-step provider guide.

**Local embeddings feature**: Build with `--features local-embeddings` to enable fastembed (ONNX-based local embedding). This requires the ONNX runtime to be available. The default build uses OpenAI embeddings or the mock embedder.

```bash
cargo build -p truenorth-llm --features local-embeddings
```

### 3.3 truenorth-memory

**Role**: Three-tier memory, full-text search (Tantivy), semantic search (embeddings), Obsidian vault sync, and the AutoDream consolidation cycle.

**Key types**: `MemoryLayer`, `SearchEngine`, `AutoDreamConsolidator`, `ObsidianWatcher`.

**Database schema**: SQLite schemas are defined inline in `sqlite_store.rs` files using `CREATE TABLE IF NOT EXISTS`. Migrations are applied at connection time. The schema version is tracked in a `schema_version` table.

**When to work in this crate**: Modifying memory tiers, changing search behavior, updating the consolidation algorithm, adding Obsidian sync features.

**Tantivy index**: The Tantivy index is in `<memory_root>/tantivy_index/`. To rebuild it from scratch (e.g., after a schema change), delete that directory and restart. The index is rebuilt from SQLite on next startup.

### 3.4 truenorth-tools

**Role**: Tool registry, WASM sandbox (Wasmtime), built-in tool implementations, MCP adapter.

**Key types**: `DefaultToolRegistry`, `WasmtimeHost`, `CapabilitySet`, `McpClient`.

**When to work in this crate**: Adding a new built-in tool, modifying sandbox resource limits, adding MCP protocol support. See [Section 5](#5-how-to-add-a-new-built-in-tool) for a step-by-step guide.

**WASM tool development**: WASM tools are `.wasm` binaries compiled from any language that targets WASM. The host interface is defined by the exported function signatures that `WasmtimeHost` expects.

### 3.5 truenorth-skills

**Role**: SKILL.md parser, skill loader, trigger matching, skill installation.

**Key types**: `SkillMarkdownParser`, `DefaultSkillLoader`, `TriggerMatcher`, `SkillValidator`, `SkillInstaller`.

**When to work in this crate**: Changing the skill format, adding new frontmatter fields, modifying trigger matching logic.

**Adding a frontmatter field**:
1. Add the field to `SkillFrontmatter` in `truenorth-core/src/types/skill.rs`
2. Update `SkillValidator` to validate the new field
3. Update the `SKILL_FORMAT.md` documentation
4. Increment `SKILL_FORMAT_VERSION` if the field is required

### 3.6 truenorth-visual

**Role**: Event bus, persistent event store, Mermaid diagram generation, state aggregator.

**Key types**: `VisualReasoningEngine`, `EventBus`, `ReasoningEventStore`, `EventAggregator`, `MermaidGenerator`.

**When to work in this crate**: Adding new event types, changing the Mermaid diagram output, modifying the aggregator's state snapshot.

**Adding a new event type**:
1. Add a new variant to `ReasoningEventPayload` in `truenorth-core/src/types/event.rs`
2. Handle the new variant in `EventAggregator::process_event()` in `truenorth-visual/src/aggregator.rs`
3. Handle it in `MermaidGenerator` if it should appear in diagrams
4. Document it in `docs/api/websocket.md`

### 3.7 truenorth-orchestrator

**Role**: The integration layer. Wires all subsystems together into the agent loop.

**Key types**: `Orchestrator`, `AgentLoopExecutor`, `RCSExecutionStrategy`, `DefaultSessionManager`, `DefaultContextBudgetManager`.

**State machine**: The agent loop state machine is in `agent_loop/state_machine.rs`. All valid state transitions are defined in `is_valid_transition()`. Adding a new state requires:
1. Adding the new variant to `AgentState` in `truenorth-core/src/traits/state.rs`
2. Adding the valid transitions to `is_valid_transition()`
3. Implementing the state's behavior in `agent_loop/executor.rs`
4. Emitting appropriate `ReasoningEvent`s

### 3.8 truenorth-web

**Role**: Axum HTTP server, REST API handlers, WebSocket event stream, SSE task stream.

**Axum state**: All subsystems are held in `AppState` (in `server/state.rs`) and injected into handlers via Axum's `Extension` or `State` extractors.

**Adding a new API endpoint**:
1. Add the handler function in `server/handlers/`
2. Register the route in the router (in `server/mod.rs` or `lib.rs`)
3. Document the endpoint in `docs/api/rest.md`

### 3.9 truenorth-cli

**Role**: CLI binary entry point. All heavy logic is in the orchestrator.

**Adding a new subcommand**:
1. Add a new variant to the `Commands` enum in `src/commands/mod.rs`
2. Create `src/commands/my_command.rs` with the handler function
3. Add the `mod my_command;` declaration and dispatch arm in `commands/mod.rs`
4. Add clap argument declarations to the new `Commands` variant

---

## 4. How to Add a New LLM Provider

This is the most common extension point. Follow these steps to add support for a new LLM API.

### Step 1: Create the Provider Module

```bash
# Create the provider file
touch crates/truenorth-llm/src/providers/my_provider.rs
```

### Step 2: Implement the LlmProvider Trait

Every provider must implement the `LlmProvider` trait from `truenorth-core`:

```rust
// crates/truenorth-llm/src/providers/my_provider.rs
use async_trait::async_trait;
use reqwest::Client;
use truenorth_core::error::LlmError;
use truenorth_core::traits::llm_provider::{LlmProvider, StreamHandle};
use truenorth_core::types::llm::{
    CompletionRequest, CompletionResponse, ProviderCapabilities, TokenUsage,
};

pub struct MyProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl MyProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.myprovider.com/v1".to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for MyProvider {
    /// Returns the provider's name (used in routing logs and error messages).
    fn name(&self) -> &str {
        "my_provider"
    }

    /// Returns the provider's capabilities.
    ///
    /// Be conservative — only declare capabilities the provider actually supports.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_use: false,       // set to true if the API supports function calling
            vision: false,         // set to true if the API accepts image inputs
            extended_thinking: false,
            max_context_tokens: 128_000,
            max_output_tokens: 4096,
        }
    }

    /// Executes a non-streaming completion request.
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        // 1. Translate CompletionRequest → provider-native request format
        let payload = build_request_payload(&request, &self.model);

        // 2. Make the HTTP call
        let response = self
            .client
            .post(&format!("{}/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: self.name().to_string(),
                message: e.to_string(),
            })?;

        // 3. Map HTTP error codes to LlmError variants
        match response.status().as_u16() {
            200 => {}
            429 => {
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(60);
                return Err(LlmError::RateLimited {
                    provider: self.name().to_string(),
                    retry_after_secs: retry_after,
                });
            }
            401 | 403 => {
                return Err(LlmError::ApiKeyExhausted {
                    provider: self.name().to_string(),
                });
            }
            status => {
                let body = response.text().await.unwrap_or_default();
                return Err(LlmError::Other {
                    provider: self.name().to_string(),
                    message: format!("HTTP {status}: {body}"),
                });
            }
        }

        // 4. Parse the response → CompletionResponse
        let raw: serde_json::Value = response
            .json()
            .await
            .map_err(|e| LlmError::MalformedResponse {
                provider: self.name().to_string(),
                detail: e.to_string(),
            })?;

        let content = raw["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(CompletionResponse {
            content,
            stop_reason: truenorth_core::types::llm::StopReason::EndTurn,
            usage: TokenUsage {
                prompt_tokens: raw["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as usize,
                completion_tokens: raw["usage"]["completion_tokens"].as_u64().unwrap_or(0) as usize,
            },
            model: self.model.clone(),
            provider: self.name().to_string(),
            tool_calls: vec![],
        })
    }

    /// Executes a streaming completion request.
    ///
    /// If streaming is not yet implemented, return an error and the router will
    /// fall back to the non-streaming path.
    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<Box<dyn StreamHandle>, LlmError> {
        // TODO: implement streaming
        Err(LlmError::Other {
            provider: self.name().to_string(),
            message: "Streaming not yet implemented for MyProvider".to_string(),
        })
    }
}

// Helper: translate CompletionRequest → provider-native JSON
fn build_request_payload(request: &CompletionRequest, model: &str) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = request
        .messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role.as_str(),
                "content": m.content_as_string(),
            })
        })
        .collect();

    serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": request.parameters.max_tokens.unwrap_or(4096),
        "temperature": request.parameters.temperature.unwrap_or(0.7),
    })
}
```

### Step 3: Register the Provider

Add the module to `crates/truenorth-llm/src/providers/mod.rs`:

```rust
pub mod my_provider;
pub use my_provider::MyProvider;

// Add a convenience constructor
pub fn my_provider(api_key: impl Into<String>, model: impl Into<String>) -> ArcProvider {
    Arc::new(MyProvider::new(api_key, model))
}
```

### Step 4: Add to the Router's Factory

In `crates/truenorth-llm/src/router.rs`, add a case in the provider factory that builds a `MyProvider` from the config:

```rust
// In DefaultLlmRouter::from_config() or equivalent factory function
match provider_config.name.as_str() {
    // ... existing providers ...
    "my_provider" => {
        let api_key = resolve_api_key(&provider_config)?;
        providers::my_provider(api_key, &provider_config.model)
    }
    _ => return Err(/* unknown provider error */),
}
```

### Step 5: Write Tests

Create `crates/truenorth-llm/tests/my_provider_test.rs`:

```rust
use truenorth_llm::providers::MockProvider;  // Use MockProvider pattern for unit tests

#[tokio::test]
async fn test_my_provider_complete() {
    // Use the MockProvider to test routing logic, not your actual provider
    // For live API testing, use #[ignore] and run with cargo test -- --ignored
}

#[tokio::test]
#[ignore = "requires MY_PROVIDER_API_KEY environment variable"]
async fn test_my_provider_live() {
    let api_key = std::env::var("MY_PROVIDER_API_KEY")
        .expect("MY_PROVIDER_API_KEY must be set");
    let provider = MyProvider::new(api_key, "my-model");
    // ... test against the real API
}
```

### Step 6: Update Configuration

Add the provider to `config.toml.example`:

```toml
[[providers]]
name = "my_provider"
model = "my-model-name"
api_key_env = "MY_PROVIDER_API_KEY"
enabled = true
```

Add `MY_PROVIDER_API_KEY=` to `.env.example`.

### Step 7: Update Documentation

- Add the provider to the "Supported LLM Providers" table in `README.md`
- Add the provider's `api_key_env` variable to the environment variable reference in `DEPLOYMENT.md`

---

## 5. How to Add a New Built-in Tool

Built-in tools run natively (not in WASM). They are appropriate for tools that need fast execution and access to the TrueNorth Rust environment.

### Step 1: Create the Tool Module

```bash
touch crates/truenorth-tools/src/builtin/my_tool.rs
```

### Step 2: Implement the Tool Trait

```rust
// crates/truenorth-tools/src/builtin/my_tool.rs
use async_trait::async_trait;
use serde_json::Value;
use truenorth_core::traits::tool::{Tool, ToolContext};
use truenorth_core::types::tool::{
    PermissionLevel, SideEffect, ToolCall, ToolError, ToolResult, ToolSchema,
};

/// A tool that does something useful.
pub struct MyTool;

#[async_trait]
impl Tool for MyTool {
    /// The tool's schema: name, description, and JSON Schema for parameters.
    fn schema(&self) -> &ToolSchema {
        // Use lazy_static or once_cell for the schema if you need to avoid
        // re-allocating on every call.
        static SCHEMA: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| ToolSchema {
            name: "my_tool".to_string(),
            description: "Does something useful given an input string.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "The input to process"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["fast", "thorough"],
                        "description": "Processing mode",
                        "default": "fast"
                    }
                },
                "required": ["input"]
            }),
        })
    }

    /// The minimum permission level required to execute this tool.
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Low  // adjust based on what the tool does
    }

    /// Side effects this tool produces (used for audit logging).
    fn side_effects(&self) -> Vec<SideEffect> {
        vec![]  // add SideEffect::ExternalRead, SideEffect::FileWrite, etc. as appropriate
    }

    /// Execute the tool.
    async fn execute(
        &self,
        call: ToolCall,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        // 1. Extract and validate parameters
        let input = call.parameters
            .get("input")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidParameters {
                message: "missing required parameter 'input'".to_string(),
            })?;

        let mode = call.parameters
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("fast");

        // 2. Do the work
        let output = match mode {
            "thorough" => process_thorough(input),
            _ => process_fast(input),
        };

        // 3. Return the result
        Ok(ToolResult {
            tool_call_id: call.id,
            content: output,
            is_error: false,
        })
    }
}

fn process_fast(input: &str) -> String {
    format!("Fast result for: {input}")
}

fn process_thorough(input: &str) -> String {
    format!("Thorough result for: {input}")
}
```

### Step 3: Register the Tool

Add the module to `crates/truenorth-tools/src/builtin/mod.rs`:

```rust
pub mod my_tool;
pub use my_tool::MyTool;

/// Register all built-in tools with the provided registry.
pub fn register_all_builtin_tools(registry: &DefaultToolRegistry) -> Result<(), RegistryError> {
    // ... existing tools ...
    registry.register(Arc::new(MyTool))?;
    Ok(())
}
```

### Step 4: Write Tests

```rust
// In crates/truenorth-tools/src/builtin/my_tool.rs, in the #[cfg(test)] block

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_call(params: serde_json::Value) -> ToolCall {
        ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: "my_tool".to_string(),
            parameters: params,
        }
    }

    #[tokio::test]
    async fn test_basic_execution() {
        let tool = MyTool;
        let ctx = ToolContext::default();
        let result = tool.execute(make_call(json!({"input": "hello"})), &ctx).await;
        assert!(result.is_ok());
        assert!(!result.unwrap().is_error);
    }

    #[tokio::test]
    async fn test_missing_required_param() {
        let tool = MyTool;
        let ctx = ToolContext::default();
        let result = tool.execute(make_call(json!({})), &ctx).await;
        assert!(result.is_err());
    }
}
```

### Step 5: Add to the Skills SKILL_FORMAT Reference

If the new tool is useful for skill authors, add it to the "Built-in Tools Available for Skills" table in `docs/SKILL_FORMAT.md`.

---

## 6. How to Add a New Execution Strategy

Execution strategies implement `ExecutionStrategy` from `truenorth-core`. TrueNorth currently has five: Direct, Sequential, Parallel, Graph, and RCS.

### Step 1: Create the Strategy Module

```bash
touch crates/truenorth-orchestrator/src/execution_modes/my_strategy.rs
```

### Step 2: Implement the ExecutionStrategy Trait

```rust
// crates/truenorth-orchestrator/src/execution_modes/my_strategy.rs
use async_trait::async_trait;
use truenorth_core::traits::execution::{
    ExecutionContext, ExecutionError, ExecutionStrategy, StepResult,
};
use truenorth_core::types::plan::Plan;
use truenorth_core::types::task::Task;

/// A custom execution strategy.
pub struct MyExecutionStrategy {
    // add fields for dependencies (LLM router, etc.)
}

impl MyExecutionStrategy {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl ExecutionStrategy for MyExecutionStrategy {
    /// Returns a human-readable name for this strategy.
    fn name(&self) -> &str {
        "my_strategy"
    }

    /// Returns true if this strategy is appropriate for the given task.
    ///
    /// The orchestrator calls this to select the right strategy based on
    /// the task's complexity score and execution mode hint.
    fn is_applicable(&self, task: &Task, plan: &Plan) -> bool {
        // Return true only for tasks this strategy should handle.
        matches!(task.execution_mode, Some(ExecutionMode::MyMode))
    }

    /// Execute the plan and return the results.
    async fn execute(
        &self,
        task: &Task,
        plan: &Plan,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<StepResult>, ExecutionError> {
        let mut results = Vec::new();

        for step in &plan.steps {
            // Emit reasoning event for observability
            ctx.emit_step_started(step).await?;

            // Execute the step
            let result = self.execute_step(task, step, ctx).await?;

            ctx.emit_step_completed(step, &result).await?;
            results.push(result);
        }

        Ok(results)
    }
}

impl MyExecutionStrategy {
    async fn execute_step(
        &self,
        task: &Task,
        step: &PlanStep,
        ctx: &mut ExecutionContext,
    ) -> Result<StepResult, ExecutionError> {
        // implement step execution
        todo!()
    }
}
```

### Step 3: Register the Strategy

Add the module to `crates/truenorth-orchestrator/src/execution_modes/mod.rs`:

```rust
pub mod my_strategy;
pub use my_strategy::MyExecutionStrategy;
```

Add the strategy to the `Orchestrator`'s strategy selection logic in `orchestrator.rs`.

### Step 4: Add the Execution Mode Variant

If the new strategy requires a new `ExecutionMode`:

1. Add the variant to `ExecutionMode` in `truenorth-core/src/types/task.rs`
2. Handle the new variant in the orchestrator's strategy selector
3. Expose the variant via the CLI (`--mode my_mode` in the `run` command)

---

## 7. Testing Strategy

TrueNorth uses a multi-layer testing strategy. All tests must pass before a PR can be merged.

### 7.1 Unit Tests

Unit tests live in `#[cfg(test)]` modules within each source file. They test individual functions and types in isolation.

**Conventions**:
- Mock all external dependencies (LLM APIs, filesystem, network) using trait objects
- Use `tempfile::TempDir` for filesystem tests — never use hard-coded paths
- Use `truenorth-llm`'s `MockProvider` for LLM-dependent tests
- Keep unit tests fast: no network calls, no real file I/O unless absolutely necessary

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_my_pure_function() {
        assert_eq!(my_function("input"), "expected output");
    }

    #[tokio::test]
    async fn test_async_operation() {
        let tmp = TempDir::new().unwrap();
        let result = async_operation(tmp.path()).await;
        assert!(result.is_ok());
    }
}
```

### 7.2 Integration Tests

Integration tests live in `crates/<crate>/tests/` directories. They test interactions between multiple components within a crate.

**Cross-crate tests** exist in some crates (e.g., `truenorth-tools/tests/cross_crate.rs`, `truenorth-visual/tests/cross_crate.rs`) to verify that crate interfaces work correctly together.

```bash
# Run integration tests for a specific crate
cargo test -p truenorth-memory

# Run only integration tests (in tests/ subdirectory)
cargo test -p truenorth-tools --test cross_crate
```

### 7.3 Doc Tests

All public APIs should have `/// ` doc comments with runnable examples. Doc tests serve as both documentation and test coverage.

```rust
/// Parses a skill file.
///
/// # Example
///
/// ```rust,no_run
/// use truenorth_skills::parser::SkillMarkdownParser;
///
/// let parser = SkillMarkdownParser::new();
/// let skill = parser.parse_skill_file(std::path::Path::new("skill.md")).unwrap();
/// println!("Name: {}", skill.frontmatter.name);
/// ```
pub fn parse_skill_file(&self, path: &Path) -> Result<ParsedSkill, ParseError> {
    // ...
}
```

Use `no_run` for examples that require a running server, real API keys, or file system state. Use `ignore` for examples that are illustrative but not runnable.

```bash
# Run doc tests
cargo test --doc --workspace
```

### 7.4 Live API Tests

Tests that require real API keys or external services are marked with `#[ignore]` and a descriptive reason:

```rust
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY environment variable"]
async fn test_anthropic_live_completion() {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY must be set");
    // ...
}
```

Run live tests explicitly:
```bash
ANTHROPIC_API_KEY=sk-ant-... cargo test -p truenorth-llm -- --ignored
```

### 7.5 Test Utilities

| Utility | Crate | Purpose |
|---------|-------|---------|
| `MockProvider` | `truenorth-llm` | Deterministic LLM responses for testing |
| `MockEmbedder` | `truenorth-llm` | Fixed embedding vectors for testing |
| `tempfile::TempDir` | `tempfile` | Temporary directories, auto-cleaned on drop |
| `tokio-test` | `tokio-test` | Async test utilities, mock timers |
| `pretty_assertions` | `pretty_assertions` | Better `assert_eq!` diffs |

### 7.6 Running All Tests

```bash
# Run all tests in the workspace
cargo test --workspace

# Run with output (don't capture stdout)
cargo test --workspace -- --nocapture

# Run a specific test by name
cargo test test_name_filter

# Run tests with logging
RUST_LOG=debug cargo test --workspace

# Check test compilation without running
cargo test --workspace --no-run
```

---

## 8. CI/CD Pipeline

The CI pipeline runs on every push and pull request via GitHub Actions (`.github/workflows/ci.yml`).

### Pipeline Stages

```
1. check      cargo check --workspace
2. fmt        cargo fmt --all -- --check
3. clippy     cargo clippy --workspace -- -D warnings
4. test       cargo test --workspace
5. doc        cargo doc --workspace --no-deps
6. build      cargo build --release
```

All stages must pass for a PR to be eligible for merge. The pipeline runs on:
- Ubuntu (primary)
- macOS (secondary — ensures cross-platform compilation)

### Lint Configuration

Lint rules are in `clippy.toml`. Current required lints (enforced as errors in CI):

```toml
# clippy.toml
msrv = "1.80"
```

Per-crate opt-ins in individual `lib.rs` files:
```rust
#![warn(missing_docs)]        // truenorth-llm, truenorth-tools, truenorth-orchestrator
#![warn(clippy::unwrap_used)] // truenorth-tools
```

### Format Configuration

```toml
# rustfmt.toml
edition = "2021"
max_width = 100
use_small_heuristics = "Default"
```

Run `cargo fmt --all` before committing. The CI check will fail if formatting differs from what `rustfmt` would produce.

---

## 9. Release Process

### Version Bumping

1. Update `version` in `Cargo.toml` (workspace root — all crates inherit):

```toml
[workspace.package]
version = "0.2.0"  # bump here
```

2. Update `TRUENORTH_VERSION` in `truenorth-core/src/constants.rs`:

```rust
pub const TRUENORTH_VERSION: &str = "0.2.0";
```

3. Increment `STATE_SCHEMA_VERSION` if the session state format changed (breaking migration required).

4. Increment `SKILL_FORMAT_VERSION` if the SKILL.md format changed.

### Creating a Release

```bash
# Run all checks locally before tagging
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
cargo build --release

# Tag the release
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

The tag push triggers a GitHub Actions release workflow that:
1. Builds release binaries for Linux (x86_64 and aarch64), macOS (x86_64 and aarch64)
2. Creates a GitHub Release with the binaries attached
3. Builds and pushes a Docker image to the registry
4. Updates the `latest` Docker tag

### Binary Optimization

The `[profile.release]` settings in `Cargo.toml` produce a highly optimized binary:

```toml
[profile.release]
opt-level = 3
lto = "thin"         # Link-time optimization (reduces binary size and improves performance)
strip = "symbols"    # Strip debug symbols (smaller binary)
codegen-units = 1    # Single codegen unit (best optimization, slower compile)
panic = "abort"      # Abort on panic (no unwinding overhead, smaller binary)
```

For the smallest possible binary (at the cost of some performance), use:
```bash
cargo build --profile release-small
```

---

## 10. Code Style Guide

### General Principles

1. **Clarity over cleverness**: Code is read more often than written. Prefer clear variable names and straightforward logic over compact one-liners.

2. **Document public APIs**: Every `pub` function, struct, enum, and trait must have a doc comment. Use `cargo doc --no-deps --open` to review your docs.

3. **Error handling**: Never use `.unwrap()` in production code (only in tests with clear explanation). Use `?` for propagation. Use `thiserror` for error types. Use `anyhow` in application boundaries.

4. **No magic numbers**: Every numeric constant belongs in `truenorth-core/src/constants.rs` with a descriptive name and doc comment.

5. **Instrument async functions**: Add `#[instrument(skip(...))]` to public async functions, especially those in hot paths. Skip large parameters (`skip(self, payload)`) to avoid log bloat.

### Naming Conventions

| Pattern | Convention | Example |
|---------|-----------|---------|
| Types | `PascalCase` | `MemoryLayer`, `DefaultLlmRouter` |
| Functions | `snake_case` | `complete()`, `build_request_payload()` |
| Constants | `SCREAMING_SNAKE_CASE` | `DEFAULT_MAX_STEPS_PER_TASK` |
| Modules | `snake_case` | `context_serializer`, `rate_limiter` |
| Trait implementations | prefix with Default/Sqlite/Wasmtime as appropriate | `DefaultLlmRouter`, `SqliteStateSerializer` |
| Error enums | suffix with `Error` | `LlmError`, `MemoryError` |
| Builder structs | suffix with `Builder` | `MemoryLayerBuilder`, `OrchestratorBuilder` |

### Module Organization

Within each module file, organize code in this order:

```rust
// 1. Module-level doc comment (//!)
// 2. Feature flags / lint attributes (#![...])
// 3. use imports (grouped: std, external crates, internal crates)
// 4. Constants
// 5. Public types (structs, enums)
// 6. Private types
// 7. impl blocks (public methods before private)
// 8. Trait implementations
// 9. #[cfg(test)] module
```

### Async Guidelines

- Use `async fn` for I/O-bound functions. Never use `async fn` for purely CPU-bound computation.
- Wrap synchronous blocking calls (SQLite, Tantivy, file I/O in hot paths) in `tokio::task::spawn_blocking`.
- Never call `.await` while holding a `parking_lot::MutexGuard` or `parking_lot::RwLockWriteGuard` — this is a deadlock risk.
- Use `tokio::time::timeout()` for all external I/O with configurable timeouts.

### Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(llm): add GroqProvider for fast inference
fix(memory): prevent duplicate entries in semantic dedup
docs(skills): update SKILL_FORMAT.md with new triggers section
test(tools): add integration tests for WASM sandbox limits
refactor(orchestrator): extract session handoff into separate module
chore: bump wasmtime to 29.0
```

---

## 11. Debugging Tips

### Enable Tracing Output

```bash
# Basic info logging
RUST_LOG=info cargo run -- run --task "test"

# Debug logging for a specific crate
RUST_LOG=truenorth_llm=debug cargo run -- run --task "test"

# Trace everything (very verbose — good for debugging)
RUST_LOG=trace cargo run -- run --task "test"

# Multiple filter levels
RUST_LOG=truenorth_orchestrator=debug,truenorth_llm=info,warn cargo run -- run --task "test"

# JSON structured logging (useful for log aggregation tools)
TRUENORTH_LOG_FORMAT=json RUST_LOG=debug cargo run -- run --task "test" | jq
```

### Key Log Spans

When debugging an issue, filter for these spans to see what's happening:

| Issue | Filter |
|-------|--------|
| LLM routing decisions | `RUST_LOG=truenorth_llm::router=debug` |
| Memory reads/writes | `RUST_LOG=truenorth_memory=debug` |
| Tool execution | `RUST_LOG=truenorth_tools=debug` |
| Agent state transitions | `RUST_LOG=truenorth_orchestrator::agent_loop=debug` |
| Skill loading | `RUST_LOG=truenorth_skills=debug` |

### Inspecting the SQLite Databases

```bash
# Open the event store
sqlite3 ~/.truenorth/events.db ".tables"
sqlite3 ~/.truenorth/events.db "SELECT event_type, session_id, created_at FROM events ORDER BY created_at DESC LIMIT 20;"

# Open the session state store
sqlite3 ~/.truenorth/sessions/<session-uuid>.db ".tables"
sqlite3 ~/.truenorth/sessions/<session-uuid>.db "SELECT key, updated_at FROM state;"

# Open the memory store
sqlite3 ~/.truenorth/memory/project.db "SELECT scope, substr(content, 1, 100), created_at FROM memory_entries ORDER BY created_at DESC LIMIT 10;"
```

### Replaying Events

The CLI supports replaying stored reasoning events for a session:

```bash
# Replay all events for a session
truenorth session replay <session-uuid>

# Replay events as JSON (for programmatic processing)
truenorth --format json session replay <session-uuid>
```

### Debugging the WASM Sandbox

To see what a WASM tool is doing:

```bash
# Enable WASM trace logging
RUST_LOG=truenorth_tools::sandbox=trace cargo run -- run --task "test with wasm tool"
```

The trace output includes:
- Module name and function being called
- Fuel consumed per call
- Memory usage before and after
- Capability checks (pass/fail)

### Profiling

For performance profiling:

```bash
# Build with debug info in release mode (profile.bench)
cargo build --profile bench

# Profile with perf (Linux)
perf record --call-graph dwarf ./target/bench/truenorth run --task "test"
perf report

# Profile with Instruments (macOS)
xcrun xctrace record --template "CPU Profiler" --launch -- ./target/bench/truenorth run --task "test"
```

### Common Issues

**"All providers exhausted" on startup**:
- Check that at least one `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` is set in `.env`
- Run `truenorth config show` to verify the provider configuration
- Use `RUST_LOG=truenorth_llm=debug` to see why each provider is failing

**SQLite "database is locked" errors**:
- TrueNorth uses SQLite in WAL mode, which supports concurrent readers. "locked" errors usually mean another process has the database open in exclusive mode.
- Check for zombie TrueNorth processes: `ps aux | grep truenorth`
- If running multiple TrueNorth instances, use different `data_dir` paths

**Memory exhaustion in WASM tools**:
- The default limit is 64 MiB. Increase it in `config.toml`: `sandbox.max_memory_bytes = 134217728` (128 MiB)
- If the tool consistently exceeds limits, it may need to be rewritten to use streaming I/O

**Tantivy "index is corrupted" errors**:
- Delete `~/.truenorth/memory/tantivy_index/` and restart — TrueNorth will rebuild the index from SQLite

---

*For deployment instructions, see [DEPLOYMENT.md](DEPLOYMENT.md). For the API reference, see [docs/api/rest.md](api/rest.md) and [docs/api/websocket.md](api/websocket.md).*
