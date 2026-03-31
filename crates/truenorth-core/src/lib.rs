/// `truenorth-core` — The contract layer for all TrueNorth crates.
///
/// This crate is the foundation. Every other crate in the TrueNorth workspace
/// depends on `truenorth-core`. It defines:
///
/// - **Types**: All shared data structures (Tasks, Plans, Sessions, Messages, Events, Config...)
/// - **Traits**: All inter-crate interface contracts (LlmProvider, MemoryStore, ToolRegistry...)
/// - **Errors**: The root `TrueNorthError` and the `LlmError` used by the router
/// - **Constants**: All system-wide configuration defaults and limits
///
/// ## Architecture Invariant
///
/// No module in this crate contains business logic. Every function is either
/// a pure data transformation (constructors, accessors) or a trait definition.
/// Business logic lives in the implementing crates: `truenorth-llm`,
/// `truenorth-memory`, `truenorth-tools`, etc.
///
/// ## Dependency Rule
///
/// `truenorth-core` has no dependencies on other TrueNorth crates. The dependency
/// graph is strictly unidirectional: all crates depend on `truenorth-core`,
/// `truenorth-core` depends on none of them. This prevents circular dependencies.

// ─── Feature Gate Documentation ───────────────────────────────────────────────
// No feature flags in core — all types and traits are always available.

// ─── Public Modules ───────────────────────────────────────────────────────────

/// Shared type definitions used across all TrueNorth crates.
pub mod types;

/// Trait definitions for all inter-crate interfaces.
pub mod traits;

/// The root error type and all error variants.
pub mod error;

/// System-wide constants and configuration defaults.
pub mod constants;

// ─── Convenience Re-exports ───────────────────────────────────────────────────
//
// The most commonly used types are re-exported at the crate root so that
// downstream crates can write `use truenorth_core::Task` instead of
// `use truenorth_core::types::task::Task`.

// Types
pub use types::task::{
    ComplexityScore, ExecutionMode, InputSource, SubTask, Task, TaskGraph, TaskPriority,
    TaskStatus,
};
pub use types::session::{HandoffDocument, LlmRoutingState, RateLimitedProvider, SessionId, SessionSnapshot, SessionState};
pub use types::message::{AgentMessage, ConversationHistory, ConversationTurn, MessageContent, MessageRole, ContentBlock, ToolCallInMessage};
pub use types::plan::{ExecutionMode as PlanExecutionMode, ExecutionResult, Plan, PlanStatus, PlanStep, PlanStepStatus};
pub use types::memory::{MemoryEntry, MemoryMetadata, MemoryQuery, MemoryScope, MemorySearchResult, MemorySearchType};
pub use types::tool::{PermissionLevel, SideEffect, ToolCall, ToolError, ToolResult, ToolSchema};
pub use types::skill::{SkillFrontmatter, SkillLoadLevel, SkillMetadata, SkillTrigger};
pub use types::llm::{
    CompletionParameters, CompletionRequest, CompletionResponse, NormalizedMessage,
    ProviderCapabilities, StopReason, StreamEvent, TokenUsage, ToolDefinition,
};
pub use types::routing::{ProviderStatus, RoutingDecision, RouterError, SkipReason, SkippedProvider};
pub use types::event::{
    ChecklistResultItem, DeviationSeverity, EventId, ReasoningEvent, ReasoningEventPayload,
};
pub use types::config::{LlmConfig, MemoryConfig, ProviderConfig, SandboxConfig, TrueNorthConfig};
pub use types::context::{
    BudgetAction, ContextBudget, ContextBudgetStats, ContextThresholds, ContextUtilization, TokenReservation,
};

// Traits
pub use traits::llm_provider::{LlmProvider, StreamHandle};
pub use traits::llm_router::LlmRouter;
pub use traits::embedding_provider::{EmbeddingError, EmbeddingModelInfo, EmbeddingProvider};
pub use traits::tool::{RegistryError, Tool, ToolContext, ToolDefinitionSummary, ToolRegistry};
pub use traits::skill::{LoadedSkill, Skill, SkillError, SkillLoader};
pub use traits::memory::{CompactionResult, ConsolidationReport, MemoryError, MemoryStore};
pub use traits::session::{SessionError, SessionManager, SessionSummary};
pub use traits::context::{BudgetError, ContextBudgetManager};
pub use traits::reasoning::{EventSubscriberHandle, ReasoningError, ReasoningEventEmitter};
pub use traits::execution::{
    AgentLoop, ExecutionContext, ExecutionControl, ExecutionError, ExecutionStrategy, StepResult, TaskResult,
};
pub use traits::state::{
    AgentState, RcsPhase, SnapshotInfo, StateError, StateMachine, StateSerializer,
    StateTransitionError,
};
pub use traits::deviation::{
    Deviation, DeviationAction, DeviationAlert, DeviationError, DeviationTracker,
};
pub use traits::checklist::{
    CheckPoint, ChecklistError, ChecklistItem, ChecklistReport, ChecklistSeverity,
    ChecklistVerification, NegativeChecklist,
};
pub use traits::heartbeat::{
    HeartbeatError, HeartbeatHealth, HeartbeatRegistration, HeartbeatScheduler,
};
pub use traits::wasm::{
    WasmCapabilities, WasmError, WasmExecutionResult, WasmExport, WasmHost, WasmMemoryStats,
    WasmModuleHandle, WasmResourceLimits, WasmSandboxConfig,
};

// Errors
pub use error::{LlmError, TrueNorthError};

// Constants
pub use constants::*;
