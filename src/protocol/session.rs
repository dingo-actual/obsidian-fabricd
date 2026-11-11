//! Session lifecycle and health messages.

use serde::{Deserialize, Serialize};

use crate::protocol::events::RerankMode;
use crate::protocol::newtypes::{Generation, ServerInstanceId, SessionId, VaultId};

/// Requests creation or recovery of one session identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionCreateRequest {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Client-chosen session identity.
    pub session_id: SessionId,
    /// Vault this session binds to; it cannot be rebound later.
    pub vault_id: VaultId,
}

/// Confirms a session and reports the state a client must resume from.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionCreateResponse {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Session identity.
    pub session_id: SessionId,
    /// Bound vault.
    pub vault_id: VaultId,
    /// Connector process identity; a change invalidates volatile sessions.
    pub server_instance_id: ServerInstanceId,
    /// Highest accepted generation; zero before any context.
    pub latest_generation: Generation,
    /// Idle timeout after which the session expires.
    pub idle_timeout_seconds: u64,
}

/// Acknowledges a context or cancellation request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextAcceptance {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Session the generation belongs to.
    pub session_id: SessionId,
    /// Generation the connector now holds.
    pub generation: Generation,
    /// Whether the generation was installed.
    pub accepted: bool,
    /// Whether this was an identical retry that started no new work.
    pub duplicate: bool,
}

/// Cached process, protocol, and provider state. Never triggers a provider call.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HealthResponse {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Versions this build speaks.
    pub supported_protocol_versions: Vec<u32>,
    /// Connector build version.
    pub server_version: String,
    /// Connector process identity.
    pub server_instance_id: ServerInstanceId,
    /// Overall readiness.
    pub status: ServerStatus,
    /// Declared byte limits.
    pub limits: Limits,
    /// Retrieval service state.
    pub fabric: FabricHealth,
    /// Optional prefix stage state.
    pub contextualizer: ContextualizerHealth,
    /// Reranking stage state.
    pub reranker: RerankerHealth,
}

/// Overall readiness. A live process reports `200` even when degraded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    /// Every configured stage is usable.
    Ok,
    /// The process serves requests but retrieval is impaired.
    Degraded,
    /// Retrieval cannot run at all.
    NotReady,
}

/// Declared request and event byte limits.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Limits {
    /// Largest accepted request body.
    pub request_bytes: u64,
    /// Largest emitted event.
    pub event_bytes: u64,
}

/// What the connector currently knows about a provider.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderObservation {
    /// No call has succeeded or failed yet.
    Unknown,
    /// The last call succeeded.
    Ok,
    /// The last call failed or timed out.
    Unavailable,
    /// Configuration or credentials are wrong.
    Misconfigured,
    /// The stage is switched off.
    Disabled,
}

/// Retrieval service state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FabricHealth {
    /// Whether a credential is present.
    pub configured: bool,
    /// Last observed state.
    pub last_status: ProviderObservation,
}

/// Optional prefix stage state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextualizerHealth {
    /// Configured provider.
    pub mode: ContextualizerMode,
    /// Last observed state.
    pub last_status: ProviderObservation,
}

/// Which prefix provider is configured.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextualizerMode {
    /// The stage is switched off.
    Disabled,
    /// Fireworks AI.
    Fireworks,
    /// OpenRouter.
    OpenRouter,
}

/// Reranking stage state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RerankerHealth {
    /// Configured provider.
    pub mode: RerankMode,
    /// Last observed state.
    pub last_status: ProviderObservation,
}

#[cfg(test)]
mod tests {
    use super::{ContextualizerMode, ProviderObservation, ServerStatus};

    #[test]
    fn readiness_spellings_match_the_protocol() {
        assert_eq!(
            serde_json::to_string(&ServerStatus::NotReady).expect("serialises"),
            "\"not_ready\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderObservation::Misconfigured).expect("serialises"),
            "\"misconfigured\""
        );
        assert_eq!(
            serde_json::to_string(&ContextualizerMode::Disabled).expect("serialises"),
            "\"disabled\""
        );
    }
}
