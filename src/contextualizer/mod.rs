//! Optional retrieval-prefix stage.
//!
//! Sees the redacted editing context only, never candidate text, and influences
//! the reranking query alone: first-stage Fabric retrieval continues when model
//! access is unavailable.
//!
//! Hosted chat completions are always streamed. Some Fireworks models refuse a
//! non-streaming request above 4096 output tokens, and a stream lets the stage
//! enforce its deadline and byte bound while the answer is still arriving. The
//! output budget therefore counts reasoning tokens as well as text, and the prefix
//! text itself is bounded downstream when the reranking query is assembled.
