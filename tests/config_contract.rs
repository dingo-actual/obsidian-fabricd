//! Contract tests for the configuration surface.
//!
//! These exercise only the public API: parse, validate, render, fingerprint.

use obsidian_fabricd::config::{self, Config, ConfigError};

const BASELINE: &str = include_str!("fixtures/config/baseline.toml");
const LLAMA_SWAP: &str = include_str!("fixtures/config/reranker-llama-swap.toml");
const HOSTED_FALLBACK: &str = include_str!("fixtures/config/hosted-fallback.toml");
const INVALID_UNKNOWN_KEY: &str = include_str!("fixtures/config/invalid-unknown-key.toml");
const INVALID_VERSION: &str = include_str!("fixtures/config/invalid-version.toml");
const INVALID_FALLBACK: &str = include_str!("fixtures/config/invalid-fallback.toml");
const INVALID_PATTERN: &str = include_str!("fixtures/config/invalid-pattern.toml");

fn baseline() -> Config {
    config::parse(BASELINE).expect("the baseline fixture parses")
}

#[test]
fn baseline_parses_with_the_documented_defaults() {
    let config = baseline();

    assert_eq!(config.version, 1);
    assert_eq!(config.server.bind, "127.0.0.1");
    assert_eq!(config.server.port, 47_322);
    assert_eq!(config.server.token_env, "OBSIDIAN_FABRICD_TOKEN");
    assert_eq!(config.server.max_sessions, 16);
    assert_eq!(config.server.session_idle_seconds, 900);
    assert_eq!(config.server.request_limit_bytes, 131_072);
    assert_eq!(config.server.event_limit_bytes, 262_144);

    assert_eq!(config.fabric.base_url, "https://api.fabric.so");
    assert_eq!(config.fabric.search_path, "/v2/search");
    assert_eq!(config.fabric.api_key_env, "FABRIC_API_KEY");
    assert_eq!(config.fabric.account_namespace, "personal-fabric");
    assert_eq!(config.fabric.credential_epoch, "initial-setup");
    assert_eq!(config.fabric.base_id, "");
    assert_eq!(config.fabric.query_count, 2);
    assert!(!config.fabric.multi_query);
    assert_eq!(config.fabric.candidate_count, 25);
    assert_eq!(config.fabric.max_response_bytes, 4_194_304);

    assert!(config.privacy.redact_secrets);
    assert!(config.privacy.allowed_hosted_providers.is_empty());
    assert!(config.privacy.property_allowlist.is_empty());
    assert!(!config.privacy.include_document_title_remote);
    assert!(config.privacy.include_headings_remote);
    assert!(config.privacy.custom_redaction_patterns.is_empty());

    assert_eq!(config.context.max_semantic_chars, 12_000);
    assert_eq!(config.context.max_query_chars, 1_000);
    assert_eq!(config.context.max_candidate_chars, 8_000);

    assert!(!config.contextualizer.enabled);
    assert_eq!(config.contextualizer.max_output_tokens, 4096);
    assert_eq!(config.contextualizer.prompt_version, "context-prefix-1");
    assert!(!config.contextualizer.persist_prefixes);

    assert!(config.reranker.request_full_pool);

    assert_eq!(config.results.display_count, 5);
    assert_eq!(config.results.dismissal_ttl_hours, 24);
    assert!(!config.results.recent_tie_break);

    assert_eq!(config.timeouts.connect_ms, 3_000);
    assert_eq!(config.timeouts.fabric_ms, 10_000);
    assert_eq!(config.timeouts.contextualizer_ms, 10_000);
    assert_eq!(config.timeouts.local_reranker_ms, 30_000);
    assert_eq!(config.timeouts.hosted_reranker_ms, 15_000);
    assert_eq!(config.timeouts.overall_pipeline_ms, 45_000);

    assert_eq!(config.concurrency.pipelines, 2);
    assert_eq!(config.concurrency.fabric, 3);
    assert_eq!(config.concurrency.contextualizer, 2);
    assert_eq!(config.concurrency.local_reranker, 1);
    assert_eq!(config.concurrency.hosted_reranker, 4);

    assert_eq!(config.budgets.hosted_requests_per_minute, 30);
    assert_eq!(config.budgets.hosted_burst, 5);

    assert!(config.cache.enabled);
    assert_eq!(config.cache.path, "auto");
    assert_eq!(config.cache.final_ttl_hours, 6);
    assert_eq!(config.cache.prefix_ttl_hours, 168);
    assert_eq!(config.cache.degraded_ttl_seconds, 300);
    assert_eq!(config.cache.max_bytes, 104_857_600);
    assert_eq!(config.cache.max_entries, 5_000);
    assert_eq!(config.cache.feedback_retention_days, 30);

    assert_eq!(config.logging.level, "info");
    assert!(!config.logging.log_payloads);
}

#[test]
fn a_version_only_file_resolves_to_the_same_effective_configuration() {
    let minimal = config::parse("version = 1\n").expect("a version-only file parses");

    assert_eq!(minimal, baseline());
}

#[test]
fn every_fixture_profile_validates() {
    for text in [BASELINE, LLAMA_SWAP, HOSTED_FALLBACK] {
        let config = config::parse(text).expect("fixture parses");
        config::validate(&config).expect("fixture validates");
    }
}

#[test]
fn rejects_an_unknown_key() {
    let error = config::parse(INVALID_UNKNOWN_KEY).expect_err("an unknown key must be rejected");

    let ConfigError::Parse { source } = error else {
        panic!("expected a parse error, got {error:?}");
    };
    assert!(source.to_string().contains("redact_everything"));
}

#[test]
fn rejects_an_unsupported_schema_version() {
    let config = config::parse(INVALID_VERSION).expect("the fixture still parses");

    let error = config::validate(&config).expect_err("version 2 must be rejected");

    assert!(matches!(
        error,
        ConfigError::UnsupportedVersion {
            found: 2,
            supported: 1
        }
    ));
}

#[test]
fn rejects_a_fallback_provider_that_privacy_does_not_allow() {
    let config = config::parse(INVALID_FALLBACK).expect("the fixture still parses");

    let error = config::validate(&config).expect_err("an unapproved fallback must be rejected");

    let ConfigError::Invalid { field, detail } = error else {
        panic!("expected a validation error, got {error:?}");
    };
    assert_eq!(field, "reranker.fallback");
    assert!(detail.contains("fireworks"), "detail was: {detail}");
}

#[test]
fn rejects_a_malformed_custom_redaction_pattern() {
    let config = config::parse(INVALID_PATTERN).expect("the fixture still parses");

    let error = config::validate(&config).expect_err("a malformed pattern must be rejected");

    let ConfigError::Invalid { field, .. } = error else {
        panic!("expected a validation error, got {error:?}");
    };
    assert_eq!(field, "privacy.custom_redaction_patterns");
}

#[test]
fn the_printed_configuration_round_trips() {
    let config = baseline();

    let printed = config::render_redacted(&config).expect("the config renders");
    let reparsed = config::parse(&printed).expect("the printed config re-parses");

    config::validate(&reparsed).expect("the printed config validates");
    assert_eq!(reparsed, config);
    assert_eq!(config::fingerprint(&reparsed), config::fingerprint(&config));
}

#[test]
fn the_printed_configuration_holds_no_secret_values() {
    let config = baseline();

    let printed = config::render_redacted(&config).expect("the config renders");

    // Only environment variable names appear; values never enter the struct.
    assert!(printed.contains(r#"token_env = "OBSIDIAN_FABRICD_TOKEN""#));
    assert!(printed.contains(r#"api_key_env = "FABRIC_API_KEY""#));
}

#[test]
fn the_fingerprint_is_stable_and_tracks_pipeline_fields() {
    let config = baseline();

    assert_eq!(
        config::fingerprint(&config),
        config::fingerprint(&baseline())
    );
    assert_eq!(config::fingerprint(&config).len(), 64);

    let mut pipeline_change = baseline();
    pipeline_change.fabric.query_count = 3;
    assert_ne!(
        config::fingerprint(&config),
        config::fingerprint(&pipeline_change)
    );

    // Listener and logging settings do not change what a pipeline would produce.
    let mut listener_change = baseline();
    listener_change.server.port = 47_323;
    listener_change.logging.level = "debug".to_owned();
    assert_eq!(
        config::fingerprint(&config),
        config::fingerprint(&listener_change)
    );
}
