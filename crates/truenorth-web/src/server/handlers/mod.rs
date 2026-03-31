//! HTTP handler modules for the TrueNorth web server.
//!
//! Each sub-module handles a distinct slice of the API surface:
//!
//! | Module | Routes |
//! |--------|--------|
//! | [`api`] | REST API: tasks, sessions, skills, tools, memory |
//! | [`sse`] | `GET /api/v1/events/sse` — LLM response SSE stream |
//! | [`websocket`] | `GET /api/v1/events/ws` — Visual Reasoning WebSocket |
//! | [`a2a`] | `GET /.well-known/agent.json` — A2A Agent Card |

pub mod a2a;
pub mod api;
pub mod sse;
pub mod websocket;
