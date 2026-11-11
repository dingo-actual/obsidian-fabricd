//! Optional retrieval-prefix stage.
//!
//! Sees the redacted editing context only, never candidate text, and influences
//! the reranking query alone: first-stage Fabric retrieval continues when model
//! access is unavailable.
