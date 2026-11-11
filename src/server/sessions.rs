//! The two session routes.
//!
//! Creation is idempotent on identity: a repeat with the same vault answers `200`
//! with the state the session already holds, so a client that lost an
//! acknowledgment recovers without losing a generation. A repeat with a different
//! vault is a `409`: a session never changes vault.

use std::time::{Duration, Instant};

use crate::protocol::{PROTOCOL_VERSION, SessionCreateRequest, SessionCreateResponse, SessionId};
use crate::server::AppState;
use crate::server::error::ApiError;
use crate::session::{Admit, AdmitError};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// How long a client is asked to wait before retrying a full registry.
const CAPACITY_RETRY_AFTER: Duration = Duration::from_secs(5);

/// Creates a session, or recovers the one already bound to this identity.
pub async fn create(State(state): State<AppState>, body: Bytes) -> Response {
    let request: SessionCreateRequest = match crate::protocol::decode_strict(&body) {
        Ok(request) => request,
        Err(_) => {
            return ApiError::invalid_request(
                "the session request is not a valid protocol message",
            )
            .into_response();
        }
    };

    if request.protocol_version != PROTOCOL_VERSION {
        return ApiError::protocol_version_mismatch(request.protocol_version, &[PROTOCOL_VERSION])
            .into_response();
    }

    let now = Instant::now();
    let outcome = state
        .sessions
        .admit(request.session_id, request.vault_id, now);

    match outcome {
        Ok(Admit::Created(session)) => {
            respond(StatusCode::CREATED, &state, &session, request.vault_id)
        }
        Ok(Admit::Existing(session)) => respond(StatusCode::OK, &state, &session, request.vault_id),
        Err(AdmitError::VaultConflict { .. }) => {
            ApiError::session_conflict("this session id is already bound to a different vault")
                .into_response()
        }
        Err(AdmitError::Capacity) => {
            ApiError::capacity_limited(CAPACITY_RETRY_AFTER).into_response()
        }
    }
}

fn respond(
    status: StatusCode,
    state: &AppState,
    session: &crate::session::Session,
    vault_id: crate::protocol::VaultId,
) -> Response {
    let body = SessionCreateResponse {
        protocol_version: PROTOCOL_VERSION,
        session_id: session.id(),
        vault_id,
        server_instance_id: state.instance_id,
        latest_generation: session.latest_generation(),
        idle_timeout_seconds: state.sessions.limits().idle_timeout.as_secs(),
    };

    (status, axum::Json(body)).into_response()
}

/// Removes a session, cancelling its work and dropping its volatile state.
pub async fn delete(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    let Ok(session_id) = session_id.parse::<SessionId>() else {
        return ApiError::invalid_request("the session id is not a UUID").into_response();
    };

    match state.sessions.remove(session_id) {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => ApiError::session_not_found().into_response(),
    }
}
