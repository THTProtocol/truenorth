//! # truenorth-tools
//!
//! The tool execution layer for TrueNorth. This crate provides:
//!
//! - **[`DefaultToolRegistry`]**: The canonical `ToolRegistry` implementation backed by
//!   a thread-safe `HashMap`. Supports dynamic tool registration, fuzzy discovery,
//!   permission-gated execution, and MCP tool discovery.
//!
//! - **Built-in tools**: A suite of ready-to-use tools for web search, URL fetching,
//!   file I/O, shell execution, memory queries, and Mermaid diagram rendering.
//!
//! - **WASM sandbox**: A Wasmtime-backed `WasmHost` implementation with fuel metering,
//!   memory limits, and capability-based filesystem/network access control.
//!
//! - **MCP adapter**: HTTP-based discovery and execution of tools hosted on MCP-compatible
//!   external servers, wrapped as native `Tool` implementations.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use truenorth_tools::{DefaultToolRegistry, builtin::register_all_builtin_tools};
//! use truenorth_core::traits::tool::ToolRegistry;
//!
//! #[tokio::main]
//! async fn main() {
//!     let registry = DefaultToolRegistry::new();
//!     register_all_builtin_tools(&registry).expect("failed to register built-ins");
//!     let tools = registry.list_tools();
//!     println!("Registered {} tools", tools.len());
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::unwrap_used)]

pub mod audio;
pub mod builtin;
pub mod mcp;
pub mod registry;
pub mod sandbox;

// Re-export the primary types that callers need.
pub use registry::DefaultToolRegistry;
pub use sandbox::capabilities::CapabilitySet;
pub use sandbox::fuel::FuelMeter;
pub use sandbox::wasmtime_host::WasmtimeHost;
