//! Streamed events.
//!
//! Every event carries the same four identity fields so a client can reject a
//! delivery from another session, another server lifetime, or another generation.
//! Event identifiers are opaque cursors owned by the transport, not by these
//! payloads.

use serde::{Deserialize, Serialize};

use crate::protocol::newtypes::{Generation, ResultSetId, ServerInstanceId, SessionId};

/// One delivered result set.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ResultsEvent {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Connector process that produced this event.
    pub server_instance_id: ServerInstanceId,
    /// Session the result belongs to.
    pub session_id: SessionId,
    /// Exact generation this result answers.
    pub generation: Generation,
    /// Identity of this delivery; a cache hit gets a fresh one.
    pub result_set_id: ResultSetId,
    /// Whether a contextual prefix shaped the query.
    pub query_mode: QueryMode,
    /// Provider that actually determined the final order.
    pub rerank_mode: RerankMode,
    /// What the scores mean.
    pub score_kind: ScoreKind,
    /// Model identifier, when a model contributed.
    pub model: Option<String>,
    /// Threshold policy applied.
    pub relevance_policy: RelevancePolicy,
    /// Why the list is empty, when it is.
    pub empty_reason: Option<EmptyReason>,
    /// Whether any stage was degraded.
    pub degraded: bool,
    /// Safe, bounded reasons for degradation.
    pub degradation_codes: Vec<String>,
    /// Cache provenance.
    pub cache: CacheInfo,
    /// Ordered recommendations; at most ten.
    pub results: Vec<Recommendation>,
}

/// How the search query was built.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    /// Deterministic queries from the context alone.
    Plain,
    /// A contextual prefix also shaped the query.
    Contextual,
}

/// Which reranker determined the order shown.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RerankMode {
    /// Fabric order, no scores.
    #[serde(rename = "none")]
    None,
    /// A separately managed local server.
    #[serde(rename = "llama-swap")]
    LlamaSwap,
    /// Fireworks AI.
    #[serde(rename = "fireworks")]
    Fireworks,
    /// OpenRouter.
    #[serde(rename = "openrouter")]
    OpenRouter,
}

/// What a score means.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreKind {
    /// No score; the order is Fabric's.
    Unscored,
    /// The provider's own ordering signal, not a probability.
    ProviderRaw,
}

/// How relevance was decided.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelevancePolicy {
    /// Keep provider order; claim no universal floor.
    RankOnly,
    /// Apply a threshold configured for the exact scoring profile.
    ProviderThreshold,
}

/// Why a successful result set is empty.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptyReason {
    /// Fabric returned nothing usable.
    NoCandidates,
    /// Candidates existed but none cleared the threshold.
    BelowThreshold,
    /// Every candidate was suppressed.
    AllSuppressed,
}

/// Where the result came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CacheInfo {
    /// Whether a cache entry was served.
    pub hit: bool,
    /// Age of the served entry, when it was a hit.
    pub age_ms: Option<u64>,
}

/// One recommendation.
///
/// Provenance is copied from the source record or left null. Nothing here may be
/// synthesised: no invented author, location, excerpt, or URL.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Recommendation {
    /// Source resource identity.
    pub resource_id: String,
    /// Resource title.
    pub title: String,
    /// Resource kind, when the source reports one.
    pub resource_type: Option<String>,
    /// Author, when the source reports one.
    pub author: Option<String>,
    /// Publication year, when the source reports one.
    pub year: Option<i32>,
    /// Plain-text excerpt from the source.
    pub excerpt: Option<String>,
    /// Link that opens the resource inside its source application.
    pub deep_url: Option<String>,
    /// Original source address.
    pub original_url: Option<String>,
    /// Link to the resource in the retrieval service.
    pub fabric_url: Option<String>,
    /// Page, when the source reports one; one-based.
    pub page: Option<u32>,
    /// Section, when the source reports one.
    pub section: Option<String>,
    /// Media timestamp in seconds, when the source reports one.
    pub timestamp_seconds: Option<f64>,
    /// Provider ordering signal, when one exists.
    pub score: Option<f64>,
    /// One-based position in the normalised candidate pool.
    pub source_rank: u32,
}

/// A progress update. Intermediate updates may be coalesced.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StatusEvent {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Connector process that produced this event.
    pub server_instance_id: ServerInstanceId,
    /// Session the status belongs to.
    pub session_id: SessionId,
    /// Exact generation this status describes.
    pub generation: Generation,
    /// Current pipeline stage.
    pub stage: Stage,
    /// Safe, bounded description; never provider output.
    pub message: Option<String>,
}

/// A pipeline stage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Accepted and waiting for a slot.
    Queued,
    /// Querying Fabric.
    Searching,
    /// Building a contextual prefix.
    Contextualizing,
    /// Reranking candidates.
    Reranking,
    /// Cancelled by the client.
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::{EmptyReason, QueryMode, RelevancePolicy, RerankMode, ScoreKind, Stage};

    #[test]
    fn wire_spellings_match_the_protocol() {
        assert_eq!(
            serde_json::to_string(&QueryMode::Contextual).expect("serialises"),
            "\"contextual\""
        );
        assert_eq!(
            serde_json::to_string(&RerankMode::LlamaSwap).expect("serialises"),
            "\"llama-swap\""
        );
        assert_eq!(
            serde_json::to_string(&ScoreKind::ProviderRaw).expect("serialises"),
            "\"provider_raw\""
        );
        assert_eq!(
            serde_json::to_string(&RelevancePolicy::ProviderThreshold).expect("serialises"),
            "\"provider_threshold\""
        );
        assert_eq!(
            serde_json::to_string(&EmptyReason::AllSuppressed).expect("serialises"),
            "\"all_suppressed\""
        );
        assert_eq!(
            serde_json::to_string(&Stage::Contextualizing).expect("serialises"),
            "\"contextualizing\""
        );
    }

    #[test]
    fn an_unknown_variant_is_rejected() {
        assert!(serde_json::from_str::<RerankMode>("\"bedrock\"").is_err());
        assert!(serde_json::from_str::<Stage>("\"publishing\"").is_err());
    }
}
