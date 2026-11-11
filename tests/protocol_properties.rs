//! Property tests for the strict decoder.
//!
//! The decoder is the only door into the wire model, so its rejections matter as
//! much as its acceptances: every case below is a body the protocol forbids.

use obsidian_fabricd::protocol::{
    CachePolicy, ContextEnvelope, ProtocolError, ResultsEvent, Trigger, decode_envelope,
    decode_strict,
};
use proptest::prelude::*;
use uuid::Uuid;

fn envelope_json(generation: u64, timestamp_ms: u64, path: &str, text: &str) -> String {
    serde_json::json!({
        "protocol_version": 1,
        "session_id": "5b6f4992-9158-4b22-aa77-601f771b6907",
        "vault_id": "b1bda11a-5c6c-43ab-8153-82497d538969",
        "generation": generation,
        "document_path": path,
        "document_title": "E17",
        "current_text": text,
        "selected_text": null,
        "section_title": null,
        "heading_path": [],
        "preceding_text": [],
        "following_text": [],
        "properties": {},
        "timestamp_ms": timestamp_ms,
        "trigger": "manual",
        "cache_policy": "refresh"
    })
    .to_string()
}

prop_compose! {
    fn safe_generation()(value in 0u64..=9_007_199_254_740_991u64) -> u64 { value }
}

prop_compose! {
    fn safe_timestamp()(value in 0u64..=9_007_199_254_740_991u64) -> u64 { value }
}

prop_compose! {
    fn document_path()(segment in "[A-Za-z0-9 _-]{1,20}", file in "[A-Za-z0-9_-]{1,12}") -> String {
        format!("Notes/{segment}/{file}.md")
    }
}

prop_compose! {
    fn note_text()(chars in proptest::collection::vec(any::<char>(), 0..200)) -> String {
        chars.into_iter().collect()
    }
}

proptest! {
    #[test]
    fn an_arbitrary_valid_envelope_round_trips(
        generation in safe_generation(),
        timestamp in safe_timestamp(),
        path in document_path(),
        text in note_text(),
    ) {
        let source = envelope_json(generation, timestamp, &path, &text);

        let first = decode_envelope(source.as_bytes()).expect("a valid envelope decodes");
        let encoded = serde_json::to_string(&first).expect("the envelope serialises");
        let second = decode_envelope(encoded.as_bytes()).expect("the encoded envelope decodes");

        prop_assert_eq!(&first, &second);
        prop_assert_eq!(first.generation.get(), generation);
        prop_assert_eq!(first.timestamp_ms.get(), timestamp);
        prop_assert_eq!(&first.document_path, &path);
        prop_assert_eq!(&first.current_text, &text);
        prop_assert_eq!(first.trigger, Trigger::Manual);
        prop_assert_eq!(first.cache_policy, CachePolicy::Refresh);
    }

    #[test]
    fn nesting_deeper_than_four_levels_is_rejected(depth in 5usize..9) {
        let nested = format!("{}{}{}", "[".repeat(depth), "1", "]".repeat(depth));

        let error = decode_strict::<Vec<u8>>(nested.as_bytes())
            .expect_err("deep nesting must be rejected");

        prop_assert!(matches!(error, ProtocolError::TooDeep), "got {error:?}");
    }

    #[test]
    fn a_duplicate_key_is_rejected_at_any_depth(depth in 0usize..3) {
        let mut body = String::new();
        for _ in 0..depth {
            body.push_str(r#"{"nested":"#);
        }
        body.push_str(r#"{"a":1,"a":2}"#);
        for _ in 0..depth {
            body.push('}');
        }

        let error = decode_strict::<serde_json::Value>(body.as_bytes())
            .expect_err("a duplicate key must be rejected");

        prop_assert!(matches!(error, ProtocolError::DuplicateKey { .. }), "got {error:?}");
    }

    #[test]
    fn more_than_thirty_two_keys_are_rejected(count in 33usize..64) {
        let pairs: Vec<String> = (0..count).map(|index| format!(r#""k{index}":{index}"#)).collect();
        let body = format!("{{{}}}", pairs.join(","));

        let error = decode_strict::<serde_json::Value>(body.as_bytes())
            .expect_err("a wide object must be rejected");

        prop_assert!(matches!(error, ProtocolError::TooManyKeys { .. }), "got {error:?}");
    }

    #[test]
    fn an_out_of_range_generation_is_rejected(excess in 1u64..1_000_000) {
        let generation = 9_007_199_254_740_991 + excess;
        let source = envelope_json(generation, 0, "Notes/a.md", "text");

        let error = decode_envelope(source.as_bytes()).expect_err("an unsafe integer must be rejected");

        prop_assert!(matches!(error, ProtocolError::InvalidJson { .. }), "got {error:?}");
    }
}

#[test]
fn a_number_beyond_the_double_range_is_rejected() {
    let source = envelope_json(1, 0, "Notes/a.md", "text")
        .replace("\"timestamp_ms\":0", "\"timestamp_ms\":1e400");

    let error =
        decode_envelope(source.as_bytes()).expect_err("an infinite number must be rejected");

    assert!(
        matches!(error, ProtocolError::InvalidJson { .. }),
        "got {error:?}"
    );
}

#[test]
fn an_unsafe_timestamp_is_rejected() {
    let source = envelope_json(1, 9_007_199_254_740_992, "Notes/a.md", "text");

    let error =
        decode_envelope(source.as_bytes()).expect_err("an unsafe timestamp must be rejected");

    assert!(
        matches!(error, ProtocolError::InvalidJson { .. }),
        "got {error:?}"
    );
}

#[test]
fn invalid_utf8_is_rejected() {
    let error = decode_strict::<serde_json::Value>(&[b'{', 0xff, b'}'])
        .expect_err("invalid UTF-8 must be rejected");

    assert!(matches!(error, ProtocolError::NotUtf8), "got {error:?}");
}

#[test]
fn trailing_data_is_rejected() {
    let mut source = envelope_json(1, 0, "Notes/a.md", "text");
    source.push_str("{}");

    let error = decode_envelope(source.as_bytes()).expect_err("trailing data must be rejected");

    assert!(
        matches!(error, ProtocolError::InvalidJson { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_malformed_body_is_rejected() {
    for body in ["{", "not json", "", "{\"generation\":}"] {
        let error = decode_strict::<serde_json::Value>(body.as_bytes())
            .expect_err("a malformed body must be rejected");

        assert!(
            matches!(error, ProtocolError::InvalidJson { .. }),
            "body {body:?}: {error:?}"
        );
    }
}

#[test]
fn a_json_number_that_is_not_an_integer_is_rejected_for_an_integer_field() {
    let source =
        envelope_json(1, 0, "Notes/a.md", "text").replace("\"generation\":1", "\"generation\":1.5");

    let error =
        decode_envelope(source.as_bytes()).expect_err("a fractional generation must be rejected");

    assert!(
        matches!(error, ProtocolError::InvalidJson { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_missing_required_field_is_rejected() {
    let source =
        envelope_json(1, 0, "Notes/a.md", "text").replace("\"document_title\":\"E17\",", "");

    let error = decode_envelope(source.as_bytes()).expect_err("a missing field must be rejected");

    assert!(
        matches!(error, ProtocolError::InvalidJson { .. }),
        "got {error:?}"
    );
}

#[test]
fn the_wire_model_holds_no_arbitrary_value_for_session_and_server_identity() {
    let event: ResultsEvent =
        decode_strict(include_str!("fixtures/protocol/results-event.json").as_bytes())
            .expect("the results fixture decodes");
    let parsed: Uuid = event.result_set_id.to_string().parse().expect("a UUID");

    assert_eq!(parsed, *event.result_set_id.as_uuid());
    assert_eq!(event.server_instance_id.as_uuid().get_version_num(), 4);
}

#[test]
fn a_duplicate_key_in_an_envelope_is_rejected_at_the_root_and_below() {
    let root = envelope_json(1, 0, "Notes/a.md", "text")
        .replace("\"generation\":1", "\"generation\":1,\"generation\":2");
    let error =
        decode_envelope(root.as_bytes()).expect_err("a repeated generation must be rejected");
    assert!(
        matches!(&error, ProtocolError::DuplicateKey { key } if key == "generation"),
        "got {error:?}"
    );

    let nested = envelope_json(1, 0, "Notes/a.md", "text")
        .replace("\"properties\":{}", "\"properties\":{\"a\":1,\"a\":2}");
    let error =
        decode_envelope(nested.as_bytes()).expect_err("a repeated property must be rejected");
    assert!(
        matches!(&error, ProtocolError::DuplicateKey { key } if key == "a"),
        "got {error:?}"
    );
}

#[test]
fn an_envelope_with_an_unknown_field_still_decodes() {
    // Unknown fields are ignored but never retained, so an additive protocol
    // change cannot smuggle data into a typed value.
    let source = envelope_json(1, 0, "Notes/a.md", "text").replace(
        "\"protocol_version\":1",
        "\"protocol_version\":1,\"future_field\":\"x\"",
    );

    let envelope: ContextEnvelope =
        decode_envelope(source.as_bytes()).expect("unknown fields are tolerated");
    let re_encoded = serde_json::to_string(&envelope).expect("the envelope serialises");

    assert!(!re_encoded.contains("future_field"));
}
