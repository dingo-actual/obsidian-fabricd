//! Scheduling, orchestration, and result selection.
//!
//! At most one generation per session is active and one is pending; obsolete
//! pending work is replaced rather than queued. Publication is a conditional
//! state transition under the session lock, so stale work cannot publish. This
//! module parses no provider JSON.
