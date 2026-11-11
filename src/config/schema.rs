//! Typed configuration schema.
//!
//! Every section rejects unknown keys, so a misspelled privacy or fallback setting
//! fails validation instead of silently taking a default. Every section except
//! `version` is optional and resolves to the documented baseline.

use serde::{Deserialize, Serialize};

pub mod providers;

pub use providers::{
    AdapterPresence, Contextualizer, ContextualizerProvider, HostedAdapter, HostedProvider,
    LlamaSwapAdapter, Reranker, RerankerProvider,
};

/// The schema version this build understands; any other value is fatal.
pub const SUPPORTED_VERSION: u32 = 1;

/// Root of the configuration file.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Schema version.
    pub version: u32,
    /// Loopback listener and session limits.
    #[serde(default)]
    pub server: Server,
    /// Fabric destination and retrieval limits.
    #[serde(default)]
    pub fabric: Fabric,
    /// Egress policy: what may leave the machine.
    #[serde(default)]
    pub privacy: Privacy,
    /// Bounded context and query sizing.
    #[serde(default)]
    pub context: Context,
    /// Optional retrieval-prefix stage.
    #[serde(default)]
    pub contextualizer: Contextualizer,
    /// Reranking chain.
    #[serde(default)]
    pub reranker: Reranker,
    /// Result selection and relevance policy.
    #[serde(default)]
    pub results: Results,
    /// Stage deadlines.
    #[serde(default)]
    pub timeouts: Timeouts,
    /// Per-stage concurrency limits.
    #[serde(default)]
    pub concurrency: Concurrency,
    /// Hosted inference budgets.
    #[serde(default)]
    pub budgets: Budgets,
    /// Derived cache settings.
    #[serde(default)]
    pub cache: Cache,
    /// Log verbosity. Payload capture is not implemented and cannot be enabled.
    #[serde(default)]
    pub logging: Logging,
}

/// `[server]` — how the loopback listener behaves.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    /// Literal loopback address. Hostname resolution is outside the protocol.
    pub bind: String,
    /// Listener port.
    pub port: u16,
    /// Environment variable holding the local bearer token.
    pub token_env: String,
    /// Maximum live sessions. The registry refuses rather than evicts.
    pub max_sessions: usize,
    /// Idle timeout after which a session is dropped and its work cancelled.
    pub session_idle_seconds: u64,
    /// Maximum request body in bytes.
    pub request_limit_bytes: u64,
    /// Maximum outbound event in bytes.
    pub event_limit_bytes: u64,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_owned(),
            port: 47_322,
            token_env: "OBSIDIAN_FABRICD_TOKEN".to_owned(),
            max_sessions: 16,
            session_idle_seconds: 900,
            request_limit_bytes: 131_072,
            event_limit_bytes: 262_144,
        }
    }
}

/// `[fabric]` — the read-only Fabric destination.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fabric {
    /// Provider origin, which must be HTTPS.
    pub base_url: String,
    /// Search path below the origin.
    pub search_path: String,
    /// Environment variable holding the Fabric API key.
    pub api_key_env: String,
    /// Nonsecret account label used in the cache namespace.
    pub account_namespace: String,
    /// Nonsecret epoch, rotated when credentials or account scope change.
    pub credential_epoch: String,
    /// Developer base id, sent only when configured.
    pub base_id: String,
    /// Number of deterministic queries per pipeline (at most three).
    pub query_count: u32,
    /// Whether the deployment supports the multi-query array.
    pub multi_query: bool,
    /// Candidate pool size kept after normalisation.
    pub candidate_count: u32,
    /// Maximum decoded response body in bytes.
    pub max_response_bytes: u64,
}

impl Default for Fabric {
    fn default() -> Self {
        Self {
            base_url: "https://api.fabric.so".to_owned(),
            search_path: "/v2/search".to_owned(),
            api_key_env: "FABRIC_API_KEY".to_owned(),
            account_namespace: "personal-fabric".to_owned(),
            credential_epoch: "initial-setup".to_owned(),
            base_id: String::new(),
            query_count: 2,
            multi_query: false,
            candidate_count: 25,
            max_response_bytes: 4_194_304,
        }
    }
}

/// `[privacy]` — the egress policy applied before anything leaves the machine.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Privacy {
    /// Detect and redact recognisable secret forms.
    pub redact_secrets: bool,
    /// Hosted providers the operator explicitly permits as fallback destinations.
    pub allowed_hosted_providers: Vec<HostedProvider>,
    /// Property names that may be sent remotely. The effective set is the
    /// intersection with the plugin's own allowlist.
    pub property_allowlist: Vec<String>,
    /// Whether the note title may be sent remotely.
    pub include_document_title_remote: bool,
    /// Whether headings may be sent remotely.
    pub include_headings_remote: bool,
    /// Operator-supplied redaction patterns, compiled once at startup.
    pub custom_redaction_patterns: Vec<String>,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            redact_secrets: true,
            allowed_hosted_providers: Vec::new(),
            property_allowlist: Vec::new(),
            include_document_title_remote: false,
            include_headings_remote: true,
            custom_redaction_patterns: Vec::new(),
        }
    }
}

/// `[context]` — sizing limits measured in Unicode scalar values.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Context {
    /// Budget for distinct semantic text.
    pub max_semantic_chars: u32,
    /// per-query limit for Fabric queries.
    pub max_query_chars: u32,
    /// per-document limit for reranker input.
    pub max_candidate_chars: u32,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            max_semantic_chars: 12_000,
            max_query_chars: 1_000,
            max_candidate_chars: 8_000,
        }
    }
}

/// `[results]` — selection and relevance policy.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Results {
    /// Cards shown per result set.
    pub display_count: u32,
    /// How relevance is decided.
    pub relevance_policy: RelevancePolicy,
    /// Lifetime of a dismissal.
    pub dismissal_ttl_hours: u32,
    /// Optional tie-break away from recently shown resources.
    pub recent_tie_break: bool,
}

impl Default for Results {
    fn default() -> Self {
        Self {
            display_count: 5,
            relevance_policy: RelevancePolicy::RankOnly,
            dismissal_ttl_hours: 24,
            recent_tie_break: false,
        }
    }
}

/// How the connector decides that a candidate is relevant enough to show.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelevancePolicy {
    /// Keep provider order; claim no universal relevance floor.
    RankOnly,
    /// Apply the threshold configured for the exact scoring profile.
    ProviderThreshold,
}

/// `[timeouts]` — stage ceilings in milliseconds.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Timeouts {
    /// TCP connect timeout for outbound requests.
    pub connect_ms: u64,
    /// Fabric stage budget.
    pub fabric_ms: u64,
    /// Contextualizer stage deadline.
    pub contextualizer_ms: u64,
    /// Local reranker stage ceiling.
    pub local_reranker_ms: u64,
    /// Hosted reranker stage ceiling.
    pub hosted_reranker_ms: u64,
    /// One overall deadline shared by every stage, retry, and fallback.
    pub overall_pipeline_ms: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect_ms: 3_000,
            fabric_ms: 10_000,
            contextualizer_ms: 10_000,
            local_reranker_ms: 30_000,
            hosted_reranker_ms: 15_000,
            overall_pipeline_ms: 45_000,
        }
    }
}

/// `[concurrency]` — per-stage in-flight limits.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Concurrency {
    /// Active pipelines across all sessions.
    pub pipelines: u32,
    /// Concurrent Fabric requests.
    pub fabric: u32,
    /// Concurrent contextualizer calls.
    pub contextualizer: u32,
    /// Concurrent local rerank calls.
    pub local_reranker: u32,
    /// Concurrent hosted rerank calls.
    pub hosted_reranker: u32,
}

impl Default for Concurrency {
    fn default() -> Self {
        Self {
            pipelines: 2,
            fabric: 3,
            contextualizer: 2,
            local_reranker: 1,
            hosted_reranker: 4,
        }
    }
}

/// `[budgets]` — hosted inference cost control.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Budgets {
    /// Hosted inference requests per minute, shared across sessions.
    pub hosted_requests_per_minute: u32,
    /// Burst allowance above the per-minute rate.
    pub hosted_burst: u32,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            hosted_requests_per_minute: 30,
            hosted_burst: 5,
        }
    }
}

/// `[cache]` — the derived SQLite cache.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cache {
    /// Whether the cache is used at all.
    pub enabled: bool,
    /// `"auto"` for the platform data directory, otherwise an absolute path.
    pub path: String,
    /// Lifetime of a successful ranking entry.
    pub final_ttl_hours: u32,
    /// Lifetime of a persisted contextual prefix.
    pub prefix_ttl_hours: u32,
    /// Lifetime of a degraded result.
    pub degraded_ttl_seconds: u32,
    /// Database size budget in bytes.
    pub max_bytes: u64,
    /// Ranked-entry budget.
    pub max_entries: u64,
    /// Maximum retention of feedback rows.
    pub feedback_retention_days: u32,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "auto".to_owned(),
            final_ttl_hours: 6,
            prefix_ttl_hours: 168,
            degraded_ttl_seconds: 300,
            max_bytes: 104_857_600,
            max_entries: 5_000,
            feedback_retention_days: 30,
        }
    }
}

/// `[logging]` — log verbosity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    /// Highest level to log.
    pub level: String,
    /// Payload capture. Not implemented, so `true` is a configuration error.
    pub log_payloads: bool,
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            log_payloads: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, SUPPORTED_VERSION};

    #[test]
    fn the_supported_version_is_one() {
        assert_eq!(SUPPORTED_VERSION, 1);
    }

    #[test]
    fn omitted_sections_resolve_to_documented_defaults() {
        let config: Config = toml::from_str("version = 1\n").expect("a version-only file parses");

        assert_eq!(config.server.port, 47_322);
        assert_eq!(config.fabric.query_count, 2);
        assert!(config.cache.enabled);
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn an_unknown_section_is_rejected() {
        let error = toml::from_str::<Config>("version = 1\n[telemetry]\nlevel = \"info\"\n")
            .expect_err("an unknown section must be rejected");

        assert!(error.to_string().contains("telemetry"));
    }
}
