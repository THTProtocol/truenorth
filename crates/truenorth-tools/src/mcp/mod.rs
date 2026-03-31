//! Model Context Protocol (MCP) integration.
//!
//! This module enables TrueNorth to discover and invoke tools hosted on
//! external MCP-compatible servers. The two key components are:
//!
//! - [`client::McpClient`] — HTTP client that speaks the MCP wire protocol
//!   to discover and invoke tools on a remote server.
//! - [`adapter::McpToolAdapter`] — wraps a discovered MCP tool as a native
//!   `Tool` trait implementation so it integrates seamlessly with the
//!   [`crate::DefaultToolRegistry`].
//!
//! # MCP Wire Protocol (simplified)
//!
//! MCP servers expose two key endpoints:
//! - `GET /tools` — returns a list of available tool definitions.
//! - `POST /tools/{name}/invoke` — invokes a tool with JSON arguments and
//!   returns a JSON result.
//!
//! TrueNorth uses a simple HTTP/JSON transport. WebSocket and stdio transports
//! are planned for a future release.

pub mod adapter;
pub mod client;
