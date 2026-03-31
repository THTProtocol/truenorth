//! Middleware stack for the TrueNorth Axum server.
//!
//! This module re-exports the individual middleware layers so they can be
//! applied in [`crate::server::router`].

pub mod auth;
pub mod cors;

pub use auth::auth_layer;
pub use cors::cors_layer;
