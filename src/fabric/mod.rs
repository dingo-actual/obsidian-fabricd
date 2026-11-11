//! Read-only Fabric search adapter.
//!
//! Owns one HTTP client per destination, the search request and its private
//! response DTOs, the retry and rate-limit budget, and failure classification.
//! Only search requests exist here: no module may create, edit, move, tag, or
//! delete a Fabric resource.
