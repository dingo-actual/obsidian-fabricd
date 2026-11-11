//! The single error envelope, shared by HTTP responses and stream events.
//!
//! Messages are bounded and sanitised at the point they are produced: provider
//! bodies, URLs with credentials, request text, and stack traces never reach this
//! type.

use serde::{Deserialize, Serialize};

/// A stable machine-readable failure category.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request could not be authenticated or parsed.
    InvalidRequest,
    /// The bearer token is missing or wrong.
    Unauthorized,
    /// The request is authenticated but not permitted.
    Forbidden,
    /// The session is unknown, likely because the connector restarted.
    SessionNotFound,
    /// The client and connector do not share a protocol version.
    ProtocolVersionMismatch,
    /// This generation was already used with different content.
    GenerationConflict,
    /// A newer generation has been accepted.
    StaleGeneration,
    /// The body exceeds the request limit.
    PayloadTooLarge,
    /// The connector is at its concurrency limit.
    CapacityLimited,
    /// A required provider or credential is not ready.
    NotReady,
    /// The connector is shutting down.
    ShuttingDown,
    /// Feedback names an unknown, expired, or foreign result set.
    InvalidFeedback,
    /// The retrieval service could not be reached or refused the request.
    FabricUnavailable,
    /// An inference provider failed and no permitted alternative succeeded.
    ProviderFailure,
    /// An unexpected internal failure.
    Internal,
}

/// One failure.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ErrorEnvelope {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Machine-readable category.
    pub code: ErrorCode,
    /// Pipeline component that failed, when the failure belongs to one.
    pub stage: Option<String>,
    /// Human-readable, sanitised explanation.
    pub message: String,
    /// Whether retrying the same request could succeed.
    pub retryable: bool,
    /// Suggested delay before retrying, when one is known.
    pub retry_after_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, ErrorEnvelope};

    #[test]
    fn every_category_has_a_snake_case_spelling() {
        let codes = [
            (ErrorCode::InvalidRequest, "invalid_request"),
            (ErrorCode::Unauthorized, "unauthorized"),
            (ErrorCode::Forbidden, "forbidden"),
            (ErrorCode::SessionNotFound, "session_not_found"),
            (
                ErrorCode::ProtocolVersionMismatch,
                "protocol_version_mismatch",
            ),
            (ErrorCode::GenerationConflict, "generation_conflict"),
            (ErrorCode::StaleGeneration, "stale_generation"),
            (ErrorCode::PayloadTooLarge, "payload_too_large"),
            (ErrorCode::CapacityLimited, "capacity_limited"),
            (ErrorCode::NotReady, "not_ready"),
            (ErrorCode::ShuttingDown, "shutting_down"),
            (ErrorCode::InvalidFeedback, "invalid_feedback"),
            (ErrorCode::FabricUnavailable, "fabric_unavailable"),
            (ErrorCode::ProviderFailure, "provider_failure"),
            (ErrorCode::Internal, "internal"),
        ];

        for (code, spelling) in codes {
            assert_eq!(
                serde_json::to_string(&code).expect("serialises"),
                format!("\"{spelling}\"")
            );
        }
    }

    #[test]
    fn a_retry_delay_may_be_absent() {
        let error = ErrorEnvelope {
            protocol_version: 1,
            code: ErrorCode::StaleGeneration,
            stage: None,
            message: "A newer context was accepted.".to_owned(),
            retryable: false,
            retry_after_ms: None,
        };

        let encoded = serde_json::to_string(&error).expect("serialises");

        assert!(encoded.contains("\"retry_after_ms\":null"));
        assert!(encoded.contains("\"stage\":null"));
    }
}
