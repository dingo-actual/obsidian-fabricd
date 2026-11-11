//! Per-session state and lifecycle.
//!
//! Holds the vault binding, the generation fence, the cancellation token, the
//! terminal outcome, the subscriber, and the delivery ledger. No session lock is
//! held across a network or database `await`, and no provider module is imported
//! here: one session never receives or cancels another session's work.
