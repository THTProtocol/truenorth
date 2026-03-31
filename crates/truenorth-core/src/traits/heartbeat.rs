/// HeartbeatScheduler trait — persistent agent scheduling.
///
/// Some agent tasks are not triggered by user prompts but by time —
/// monitoring a codebase for drift, checking for new emails, running
/// nightly memory consolidation. This trait manages those scheduled agents
/// with circuit-breaker semantics and SQLite-backed persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::types::task::Task;

/// A registered heartbeat agent: a task that runs on a recurring schedule.
///
/// Inspired by Paperclip's persistent agent model. Each registration has
/// a task template that is cloned on each tick and dispatched to the AgentLoop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatRegistration {
    /// Unique identifier for this registration.
    pub id: String,
    /// Human-readable description of what this agent does.
    pub description: String,
    /// The interval between heartbeat ticks.
    #[serde(with = "duration_secs")]
    pub interval: Duration,
    /// The task template to execute on each tick.
    pub task_template: Task,
    /// Whether this heartbeat is currently active (ticking).
    pub active: bool,
    /// The number of ticks fired since registration.
    pub tick_count: u64,
    /// When the next tick is scheduled.
    pub next_tick: DateTime<Utc>,
    /// Maximum number of consecutive failures before the heartbeat is suspended.
    pub max_consecutive_failures: u32,
    /// Current consecutive failure count.
    pub consecutive_failures: u32,
    /// When this registration was created.
    pub registered_at: DateTime<Utc>,
}

/// Serialization helper for Duration as seconds.
mod duration_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_secs)
    }
}

/// The health status of a heartbeat registration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HeartbeatHealth {
    /// Running normally — last tick succeeded.
    Healthy,
    /// Last tick failed but within the failure threshold.
    Degraded { failures: u32 },
    /// Exceeded failure threshold; suspended until manually resumed.
    Suspended { reason: String },
    /// Manually deactivated.
    Inactive,
}

/// Errors from the heartbeat scheduler.
#[derive(Debug, Error)]
pub enum HeartbeatError {
    /// No registration with this ID exists.
    #[error("Registration not found: {id}")]
    RegistrationNotFound { id: String },

    /// A tick execution failed.
    #[error("Tick execution failed: {reason}")]
    TickFailed { reason: String },

    /// The scheduler is not running.
    #[error("Scheduler is not running")]
    SchedulerNotRunning,

    /// An I/O or storage error occurred.
    #[error("Heartbeat storage error: {message}")]
    StorageError { message: String },

    /// A duplicate registration ID was provided.
    #[error("Registration '{id}' already exists")]
    DuplicateRegistration { id: String },
}

/// Schedules and manages persistent heartbeat agents.
///
/// Design rationale: some agent tasks are not triggered by user prompts but by
/// time — monitoring a codebase for drift, checking for new emails, running
/// nightly memory consolidation. Paperclip's heartbeat model is the reference.
///
/// The scheduler runs as a background Tokio task. Registered heartbeats are
/// checked on each tick interval; when due, a new Task is synthesized from
/// the template and dispatched to the AgentLoop. Failures are tracked per-
/// registration with circuit-breaker semantics: after `max_consecutive_failures`
/// the registration is suspended to prevent runaway retries.
///
/// Heartbeat state persists in SQLite so that a TrueNorth restart resumes all
/// active heartbeats without re-registration.
#[async_trait]
pub trait HeartbeatScheduler: Send + Sync + std::fmt::Debug {
    /// Registers a new heartbeat agent.
    ///
    /// Returns the registration ID. If a heartbeat with the same ID already
    /// exists, it is updated with the new configuration (idempotent update).
    async fn register(
        &self,
        registration: HeartbeatRegistration,
    ) -> Result<String, HeartbeatError>;

    /// Removes a heartbeat registration permanently.
    ///
    /// Also removes the stored SQLite record so it does not resume on restart.
    async fn deregister(&self, id: &str) -> Result<(), HeartbeatError>;

    /// Suspends a heartbeat (stops ticking but retains the registration).
    ///
    /// The heartbeat can be resumed later without re-registering.
    async fn suspend(&self, id: &str) -> Result<(), HeartbeatError>;

    /// Resumes a suspended heartbeat.
    ///
    /// Resets the consecutive failure count and schedules the next tick.
    async fn resume_heartbeat(&self, id: &str) -> Result<(), HeartbeatError>;

    /// Manually fires a heartbeat tick outside of its schedule.
    ///
    /// Used for testing and for on-demand trigger of persistent tasks.
    /// Does not affect the regular tick schedule.
    async fn fire_now(&self, id: &str) -> Result<(), HeartbeatError>;

    /// Returns the health status of a specific registration.
    async fn check_health(&self, id: &str) -> Result<HeartbeatHealth, HeartbeatError>;

    /// Returns health status for all registrations.
    async fn health_report(&self) -> Result<Vec<(String, HeartbeatHealth)>, HeartbeatError>;

    /// Starts the background scheduler loop.
    ///
    /// Must be called once at startup. The scheduler polls all registrations
    /// on a configurable `poll_interval` (default: 1 second) and fires
    /// those that are due. Spawns a background Tokio task.
    async fn start(&self) -> Result<(), HeartbeatError>;

    /// Gracefully shuts down the scheduler.
    ///
    /// Allows in-flight ticks to complete before stopping the background task.
    async fn shutdown(&self) -> Result<(), HeartbeatError>;

    /// Returns all registered heartbeats.
    async fn list_all(&self) -> Result<Vec<HeartbeatRegistration>, HeartbeatError>;

    /// Returns a specific registration by ID.
    async fn get(
        &self,
        id: &str,
    ) -> Result<HeartbeatRegistration, HeartbeatError>;

    /// Updates the next tick time for a registration.
    ///
    /// Called internally by the scheduler after each tick fires.
    async fn update_next_tick(
        &self,
        id: &str,
        next_tick: DateTime<Utc>,
    ) -> Result<(), HeartbeatError>;
}
