//! Derived SQLite cache and feedback store.
//!
//! Runs on a dedicated bounded worker thread, never on a Tokio reactor thread.
//! Stores derived data only: never raw context envelopes, note text, session
//! tokens, credentials, or event streams. Migrations live in `migrations/`.
