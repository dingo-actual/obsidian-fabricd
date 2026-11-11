//! Golden wire fixtures.
//!
//! Each fixture is decoded through the strict entry point, checked field by
//! field, and re-encoded. Re-encoding must equal the fixture value, which pins
//! every wire name: a renamed field shows up here immediately.

use obsidian_fabricd::protocol::{
    CachePolicy, CancelReason, CancelRequest, ContextAcceptance, ContextualizerMode, ErrorCode,
    ErrorEnvelope, FeedbackEvent, FeedbackRequest, HealthResponse, ProviderObservation, QueryMode,
    RelevancePolicy, RerankMode, ResultsEvent, ScoreKind, ServerStatus, SessionCreateRequest,
    SessionCreateResponse, Stage, StatusEvent, Trigger, decode_envelope, decode_strict,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

const ENVELOPE: &str = include_str!("fixtures/protocol/context-envelope.json");
const RESULTS: &str = include_str!("fixtures/protocol/results-event.json");
const SESSION_REQUEST: &str = include_str!("fixtures/protocol/session-create-request.json");
const SESSION_RESPONSE: &str = include_str!("fixtures/protocol/session-create-response.json");
const HEALTH: &str = include_str!("fixtures/protocol/health.json");
const ERROR: &str = include_str!("fixtures/protocol/error.json");
const CANCEL: &str = include_str!("fixtures/protocol/cancel-request.json");
const ACCEPTANCE: &str = include_str!("fixtures/protocol/context-acceptance.json");
const STATUS: &str = include_str!("fixtures/protocol/status-event.json");
const FEEDBACK: &str = include_str!("fixtures/protocol/feedback-request.json");

/// Asserts that a typed value re-encodes to exactly the fixture's JSON value.
#[track_caller]
fn assert_round_trips<T>(fixture: &str, typed: &T)
where
    T: Serialize,
{
    let expected: Value = serde_json::from_str(fixture).expect("the fixture is JSON");
    let actual = serde_json::to_value(typed).expect("the typed value serialises");

    assert_eq!(
        actual, expected,
        "re-encoded value differs from the fixture"
    );
}

#[track_caller]
fn decode<T: DeserializeOwned>(fixture: &str) -> T {
    decode_strict(fixture.as_bytes()).expect("the fixture decodes")
}

#[test]
fn the_worked_context_envelope_decodes_field_by_field() {
    let envelope = decode_envelope(ENVELOPE.as_bytes()).expect("the worked exchange decodes");

    assert_eq!(envelope.protocol_version, 1);
    assert_eq!(
        envelope.session_id.to_string(),
        "5b6f4992-9158-4b22-aa77-601f771b6907"
    );
    assert_eq!(
        envelope.vault_id.to_string(),
        "b1bda11a-5c6c-43ab-8153-82497d538969"
    );
    assert_eq!(envelope.generation.get(), 12);
    assert_eq!(envelope.document_path, "Projects/Associative Memory/E17.md");
    assert_eq!(envelope.document_title, "E17");
    assert_eq!(
        envelope.current_text,
        "The learned keys become nearly collinear during training."
    );
    assert_eq!(envelope.selected_text, None);
    assert_eq!(envelope.section_title.as_deref(), Some("Failure analysis"));
    assert_eq!(
        envelope.heading_path,
        ["Experiment E17", "Failure analysis"]
    );
    assert_eq!(envelope.preceding_text.len(), 1);
    assert!(envelope.following_text.is_empty());
    assert!(envelope.properties.is_empty());
    assert_eq!(envelope.timestamp_ms.get(), 1_791_290_000_000);
    assert_eq!(envelope.trigger, Trigger::Automatic);
    assert_eq!(envelope.cache_policy, CachePolicy::PreferCache);
}

#[test]
fn the_worked_context_envelope_round_trips() {
    let envelope = decode_envelope(ENVELOPE.as_bytes()).expect("the worked exchange decodes");

    assert_round_trips(ENVELOPE, &envelope);
}

#[test]
fn the_synthetic_results_event_round_trips() {
    let event: ResultsEvent = decode(RESULTS);

    assert_eq!(event.generation.get(), 12);
    assert_eq!(event.query_mode, QueryMode::Plain);
    assert_eq!(event.rerank_mode, RerankMode::None);
    assert_eq!(event.score_kind, ScoreKind::Unscored);
    assert_eq!(event.model, None);
    assert_eq!(event.relevance_policy, RelevancePolicy::RankOnly);
    assert_eq!(event.empty_reason, None);
    assert!(!event.degraded);
    assert!(event.degradation_codes.is_empty());
    assert!(!event.cache.hit);
    assert_eq!(event.cache.age_ms, None);

    assert_eq!(event.results.len(), 1);
    let recommendation = &event.results[0];
    assert_eq!(recommendation.resource_id, "fixture-resource-1");
    assert_eq!(recommendation.title, "Associative memory experiment notes");
    assert_eq!(recommendation.resource_type.as_deref(), Some("note"));
    assert_eq!(
        recommendation.original_url.as_deref(),
        Some("https://example.org/associative-memory")
    );
    assert_eq!(recommendation.source_rank, 1);
    assert_eq!(recommendation.score, None);
    assert_eq!(recommendation.page, None);

    assert_round_trips(RESULTS, &event);
}

#[test]
fn session_creation_round_trips() {
    let request: SessionCreateRequest = decode(SESSION_REQUEST);
    assert_eq!(request.protocol_version, 1);
    assert_round_trips(SESSION_REQUEST, &request);

    let response: SessionCreateResponse = decode(SESSION_RESPONSE);
    assert_eq!(response.latest_generation.get(), 0);
    assert_eq!(response.idle_timeout_seconds, 900);
    assert_round_trips(SESSION_RESPONSE, &response);
}

#[test]
fn the_health_response_round_trips() {
    let health: HealthResponse = decode(HEALTH);

    assert_eq!(health.supported_protocol_versions, vec![1]);
    assert_eq!(health.server_version, "0.1.0");
    assert_eq!(health.status, ServerStatus::Degraded);
    assert_eq!(health.limits.request_bytes, 131_072);
    assert_eq!(health.limits.event_bytes, 262_144);
    assert!(health.fabric.configured);
    assert_eq!(health.fabric.last_status, ProviderObservation::Unknown);
    assert_eq!(health.contextualizer.mode, ContextualizerMode::Disabled);
    assert_eq!(
        health.contextualizer.last_status,
        ProviderObservation::Disabled
    );
    assert_eq!(health.reranker.mode, RerankMode::None);
    assert_eq!(health.reranker.last_status, ProviderObservation::Disabled);

    assert_round_trips(HEALTH, &health);
}

#[test]
fn the_error_envelope_round_trips() {
    let error: ErrorEnvelope = decode(ERROR);

    assert_eq!(error.code, ErrorCode::FabricUnavailable);
    assert_eq!(error.stage.as_deref(), Some("fabric"));
    assert!(error.retryable);
    assert_eq!(error.retry_after_ms, Some(30_000));

    assert_round_trips(ERROR, &error);
}

#[test]
fn a_cancel_request_round_trips() {
    let cancel: CancelRequest = decode(CANCEL);

    assert_eq!(cancel.generation.get(), 13);
    assert_eq!(cancel.reason, CancelReason::ContextChanged);

    assert_round_trips(CANCEL, &cancel);
}

#[test]
fn an_acceptance_round_trips() {
    let acceptance: ContextAcceptance = decode(ACCEPTANCE);

    assert!(acceptance.accepted);
    assert!(!acceptance.duplicate);

    assert_round_trips(ACCEPTANCE, &acceptance);
}

#[test]
fn a_status_event_round_trips() {
    let status: StatusEvent = decode(STATUS);

    assert_eq!(status.stage, Stage::Searching);
    assert_eq!(status.message.as_deref(), Some("Searching Fabric."));

    assert_round_trips(STATUS, &status);
}

#[test]
fn a_feedback_request_round_trips() {
    let feedback: FeedbackRequest = decode(FEEDBACK);

    assert_eq!(feedback.event, FeedbackEvent::Opened);
    assert_eq!(feedback.resource_id, "fixture-resource-1");
    assert_eq!(feedback.generation.get(), 12);

    assert_round_trips(FEEDBACK, &feedback);
}

#[test]
fn an_unknown_additive_field_is_tolerated_and_not_retained() {
    let mut value: Value = serde_json::from_str(ENVELOPE).expect("the fixture is JSON");
    value["experimental_field"] = Value::from("ignored");
    value["selection"] = serde_json::json!({"offset": 3});
    let text = serde_json::to_string(&value).expect("the value serialises");

    let envelope = decode_envelope(text.as_bytes()).expect("unknown fields are ignored");
    let re_encoded = serde_json::to_string(&envelope).expect("the envelope serialises");

    assert!(!re_encoded.contains("experimental_field"));
    assert!(!re_encoded.contains("selection"));
    assert_eq!(envelope.heading_path.len(), 2);
}

#[test]
fn every_fixture_uses_snake_case_keys() {
    for fixture in [
        ENVELOPE,
        RESULTS,
        SESSION_REQUEST,
        SESSION_RESPONSE,
        HEALTH,
        ERROR,
        CANCEL,
        ACCEPTANCE,
        STATUS,
        FEEDBACK,
    ] {
        for key in keys_of(fixture) {
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "key `{key}` is not snake_case"
            );
        }
    }
}

fn keys_of(fixture: &str) -> Vec<String> {
    fn walk(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    out.push(key.clone());
                    walk(child, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }

    let mut keys = Vec::new();
    walk(
        &serde_json::from_str(fixture).expect("the fixture is JSON"),
        &mut keys,
    );
    keys
}
