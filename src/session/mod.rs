//! Per-session state and lifecycle.
//!
//! A session is the unit that stops work leaking between windows. It binds to one
//! vault for its whole life, carries its own cancellation token, and holds its
//! mutable state behind one lock. Two windows on the same vault are two sessions
//! with independent state and independent cancellation.
//!
//! Expiry is explicit: [`registry::Registry::expire_due`] takes the current instant,
//! so the timing rules are testable without waiting, and [`expiry::spawn_sweeper`]
//! supplies that instant in a running process.
//!
//! Lock discipline is a compile-time rule here: no lock is held across an `await`.

#![deny(clippy::await_holding_lock)]

pub mod expiry;
pub mod registry;
pub mod state;

pub use expiry::spawn_sweeper;
pub use registry::{Admit, AdmitError, Limits, Registry};
pub use state::Session;
