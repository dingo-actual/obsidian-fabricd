//! Provider-neutral reranking seam and its adapters.
//!
//! Accepts a redacted query, immutable bounded candidate documents, and a
//! cancellation token. Every returned hit is validated before it is mapped back,
//! because an out-of-range or duplicate index would attach a score to the wrong
//! resource.
