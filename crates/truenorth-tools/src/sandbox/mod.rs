//! WASM sandbox infrastructure for TrueNorth.
//!
//! This module provides the capability-based security sandbox that isolates
//! third-party tool execution. Key components:
//!
//! - [`wasmtime_host::WasmtimeHost`] — implements `WasmHost`-equivalent
//!   functionality using Wasmtime with fuel metering and WASI capability grants.
//! - [`capabilities::CapabilitySet`] — models the filesystem and network
//!   grants that a specific WASM invocation receives.
//! - [`fuel::FuelMeter`] — tracks per-invocation fuel consumption and warns
//!   at 80% utilisation.

pub mod capabilities;
pub mod fuel;
pub mod wasmtime_host;
