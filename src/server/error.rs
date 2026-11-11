//! The single mapping from a failure to the wire error envelope.
//!
//! Every failure that reaches a client goes through [`ApiError`], so the error
//! shape cannot drift between routes. Messages are built from constants and a
//! bounded number of values, and never include request text, provider output, or a
//! header value.

use std::borrow::Cow;
use std::time::Duration;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::protocol::{ErrorCode, ErrorEnvelope, PROTOCOL_VERSION};
use crate::server::AppState;
use crate::server::limits::describe_bytes;

/// Longest message this crate will put on the wire.
const MAX_MESSAGE_SCALARS: usize = 200;

/// One client-visible failure.
#[derive(Clone, Debug)]
pub struct ApiError {
    status: StatusCode,
    code: ErrorCode,
    stage: Option<&'static str>,
    message: Cow<'static, str>,
    retryable: bool,
    retry_after_ms: Option<u64>,
    supported_protocol_versions: Option<Vec<u32>>,
}

impl ApiError {
    /// The request could not be read as a protocol message.
    pub fn invalid_request(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest, message)
    }

    /// The bearer token is missing, malformed, or wrong.
    pub fn unauthorized(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized, message)
    }

    /// A local request control refused the request.
    pub fn forbidden(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, message)
    }

    /// An interaction referenced a result set this session never delivered.
    pub fn invalid_feedback(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidFeedback, message)
    }

    /// The session identity is unknown, normally because the process restarted.
    pub fn session_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::SessionNotFound,
            "the session is unknown; create it again",
        )
    }

    /// The session identity is already bound to a different vault.
    ///
    /// No canonical code covers a rebind, so this service uses `session_conflict`
    /// with a `409`, matching the generation-conflict shape.
    pub fn session_conflict(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::SessionConflict, message)
    }

    /// The body is larger than the configured limit.
    pub fn payload_too_large(limit: u64) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::PayloadTooLarge,
            describe_bytes(limit),
        )
    }

    /// The client and this build do not share a protocol version.
    pub fn protocol_version_mismatch(found: u32, supported: &[u32]) -> Self {
        let mut error = Self::new(
            StatusCode::CONFLICT,
            ErrorCode::ProtocolVersionMismatch,
            format!("protocol version {found} is not supported"),
        );
        error.supported_protocol_versions = Some(supported.to_vec());

        error
    }

    /// The connector is at its concurrency limit.
    pub fn capacity_limited(retry_after: Duration) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::CapacityLimited,
            "too many requests or failed attempts; retry shortly",
        );
        error.retryable = true;
        error.retry_after_ms = Some(u64::try_from(retry_after.as_millis()).unwrap_or(u64::MAX));

        error
    }

    /// Retrieval cannot run because configuration or a credential is missing.
    pub fn not_ready(message: impl Into<Cow<'static, str>>) -> Self {
        let mut error = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NotReady,
            message,
        );
        error.retryable = true;

        error
    }

    /// The connector is stopping.
    pub fn shutting_down() -> Self {
        let mut error = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::ShuttingDown,
            "the connector is shutting down",
        );
        error.retryable = true;

        error
    }

    /// An unexpected internal failure.
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "the connector could not complete the request",
        )
    }

    /// Names the pipeline component a failure belongs to.
    pub fn at_stage(mut self, stage: &'static str) -> Self {
        self.stage = Some(stage);

        self
    }

    /// Marks a failure that may succeed if retried.
    pub fn retryable(mut self) -> Self {
        self.retryable = true;

        self
    }

    /// The status this failure is sent with.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The envelope this failure becomes.
    pub fn envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope {
            protocol_version: PROTOCOL_VERSION,
            code: self.code,
            stage: self.stage.map(str::to_owned),
            message: self.message.chars().take(MAX_MESSAGE_SCALARS).collect(),
            retryable: self.retryable,
            retry_after_ms: self.retry_after_ms,
        }
    }

    fn new(status: StatusCode, code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            code,
            stage: None,
            message: message.into(),
            retryable: false,
            retry_after_ms: None,
            supported_protocol_versions: None,
        }
    }
}

/// The body actually written: the shared envelope plus the one field only a
/// protocol mismatch carries.
#[derive(Serialize)]
struct ErrorBody<'a> {
    #[serde(flatten)]
    envelope: &'a ErrorEnvelope,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported_protocol_versions: Option<&'a [u32]>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let envelope = self.envelope();
        let body = ErrorBody {
            envelope: &envelope,
            supported_protocol_versions: self.supported_protocol_versions.as_deref(),
        };

        let mut response = (self.status, Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if self.status == StatusCode::UNAUTHORIZED {
            headers.insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }

        response
    }
}

/// Ensures every response is uncacheable, and turns a body-limit rejection from
/// the transport layer into the canonical envelope.
pub async fn normalise(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;

    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::payload_too_large(state.request_limit_bytes()).into_response();
    }

    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));

    response
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::StatusCode;

    use super::{ApiError, MAX_MESSAGE_SCALARS};
    use crate::protocol::ErrorCode;

    #[test]
    fn each_constructor_uses_the_canonical_status_and_code() {
        let cases = [
            (
                ApiError::invalid_request("bad"),
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidRequest,
            ),
            (
                ApiError::unauthorized("no token"),
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
            ),
            (
                ApiError::forbidden("bad host"),
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
            ),
            (
                ApiError::internal(),
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
            ),
            (
                ApiError::shutting_down(),
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::ShuttingDown,
            ),
            (
                ApiError::payload_too_large(131_072),
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::PayloadTooLarge,
            ),
            (
                ApiError::session_not_found(),
                StatusCode::NOT_FOUND,
                ErrorCode::SessionNotFound,
            ),
            (
                ApiError::session_conflict("already bound"),
                StatusCode::CONFLICT,
                ErrorCode::SessionConflict,
            ),
        ];

        for (error, status, code) in cases {
            assert_eq!(error.status(), status);
            assert_eq!(error.envelope().code, code);
            assert_eq!(error.envelope().protocol_version, 1);
        }
    }

    #[test]
    fn a_protocol_mismatch_carries_the_supported_versions() {
        let error = ApiError::protocol_version_mismatch(9, &[1]);

        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.envelope().code, ErrorCode::ProtocolVersionMismatch);
        assert_eq!(
            error.supported_protocol_versions.as_deref(),
            Some(&[1u32][..])
        );
        assert!(error.envelope().message.contains('9'));
    }

    #[test]
    fn capacity_limiting_is_retryable_and_states_a_delay() {
        let error = ApiError::capacity_limited(Duration::from_millis(1_500));

        assert_eq!(error.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(error.envelope().retryable);
        assert_eq!(error.envelope().retry_after_ms, Some(1_500));
    }

    #[test]
    fn a_message_is_bounded_before_it_reaches_the_wire() {
        let long = "x".repeat(MAX_MESSAGE_SCALARS * 3);

        let error = ApiError::invalid_request(long);

        assert_eq!(
            error.envelope().message.chars().count(),
            MAX_MESSAGE_SCALARS
        );
    }

    #[test]
    fn a_stage_can_be_named_without_changing_the_code() {
        let error = ApiError::not_ready("Fabric credentials are missing").at_stage("fabric");

        assert_eq!(error.envelope().stage.as_deref(), Some("fabric"));
        assert!(error.envelope().retryable);
    }

    #[test]
    fn every_built_in_message_is_well_under_the_bound() {
        let errors = [
            ApiError::invalid_request("the request body is not a valid protocol message"),
            ApiError::unauthorized("the bearer token is missing or wrong"),
            ApiError::forbidden("the request did not address the configured loopback listener"),
            ApiError::payload_too_large(u64::MAX),
            ApiError::protocol_version_mismatch(u32::MAX, &[1]),
            ApiError::capacity_limited(Duration::from_secs(10)),
            ApiError::not_ready("the connector has no retrieval credential"),
            ApiError::shutting_down(),
            ApiError::internal(),
            ApiError::invalid_feedback("the result set is unknown or expired"),
            ApiError::session_not_found(),
            ApiError::session_conflict("the session is already bound to another vault"),
        ];

        for error in errors {
            assert!(error.envelope().message.chars().count() <= MAX_MESSAGE_SCALARS);
        }
    }
}
