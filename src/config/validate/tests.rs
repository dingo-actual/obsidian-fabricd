//! Table-driven checks for each cross-section rule.

use super::validate;
use crate::config::schema::{
    ContextualizerProvider, HostedAdapter, HostedProvider, LlamaSwapAdapter, RerankerProvider,
};
use crate::config::{Config, ConfigError, parse};

const BASELINE: &str = include_str!("../../../tests/fixtures/config/baseline.toml");

fn baseline() -> Config {
    parse(BASELINE).expect("the baseline fixture parses")
}

fn hosted_adapter(model: &str) -> HostedAdapter {
    HostedAdapter {
        base_url: "https://api.fireworks.ai/inference/v1".to_owned(),
        endpoint: "/rerank".to_owned(),
        api_key_env: "FIREWORKS_API_KEY".to_owned(),
        model: model.to_owned(),
    }
}

fn llama_swap_adapter(base_url: &str) -> LlamaSwapAdapter {
    LlamaSwapAdapter {
        base_url: base_url.to_owned(),
        endpoint: "/v1/rerank".to_owned(),
        model: "qwen3-reranker-8b".to_owned(),
        api_key_env: None,
    }
}

/// Applies `mutate` to the baseline and asserts that validation fails on `field`.
#[track_caller]
fn assert_rejected(field: &str, mutate: impl FnOnce(&mut Config)) {
    let mut config = baseline();
    mutate(&mut config);

    let error = validate(&config).expect_err("the configuration must be rejected");

    let ConfigError::Invalid { field: actual, .. } = error else {
        panic!("expected an invalid-field error, got {error:?}");
    };
    assert_eq!(actual, field);
}

#[test]
fn the_baseline_is_valid() {
    validate(&baseline()).expect("the baseline validates");
}

#[test]
fn the_schema_version_must_be_one() {
    let mut config = baseline();
    config.version = 2;

    assert!(matches!(
        validate(&config),
        Err(ConfigError::UnsupportedVersion {
            found: 2,
            supported: 1
        })
    ));
}

#[test]
fn the_listener_must_be_a_literal_loopback_address() {
    for bind in ["0.0.0.0", "localhost", "192.168.1.10", "[::1]"] {
        assert_rejected("server.bind", |config| config.server.bind = bind.to_owned());
    }

    let mut config = baseline();
    config.server.bind = "::1".to_owned();
    validate(&config).expect("the IPv6 loopback is accepted");
}

#[test]
fn server_numbers_must_be_in_range() {
    assert_rejected("server.port", |config| config.server.port = 0);
    assert_rejected("server.max_sessions", |config| {
        config.server.max_sessions = 0;
    });
    assert_rejected("server.max_sessions", |config| {
        config.server.max_sessions = 65;
    });
    assert_rejected("server.session_idle_seconds", |config| {
        config.server.session_idle_seconds = 0;
    });
    assert_rejected("server.request_limit_bytes", |config| {
        config.server.request_limit_bytes = 0;
    });
    assert_rejected("server.event_limit_bytes", |config| {
        config.server.event_limit_bytes = 0;
    });
    assert_rejected("server.token_env", |config| {
        config.server.token_env.clear();
    });
}

#[test]
fn fabric_destinations_must_be_secure_and_complete() {
    assert_rejected("fabric.base_url", |config| {
        config.fabric.base_url = "http://api.fabric.so".to_owned();
    });
    assert_rejected("fabric.base_url", |config| {
        config.fabric.base_url = "https://user:secret@api.fabric.so".to_owned();
    });
    assert_rejected("fabric.base_url", |config| {
        config.fabric.base_url = "not a url".to_owned();
    });
    assert_rejected("fabric.search_path", |config| {
        config.fabric.search_path = "v2/search".to_owned();
    });
    assert_rejected("fabric.api_key_env", |config| {
        config.fabric.api_key_env.clear();
    });
    assert_rejected("fabric.account_namespace", |config| {
        config.fabric.account_namespace.clear();
    });
    assert_rejected("fabric.credential_epoch", |config| {
        config.fabric.credential_epoch.clear();
    });
}

#[test]
fn the_pipeline_builds_at_most_three_queries_into_a_bounded_pool() {
    assert_rejected("fabric.query_count", |config| {
        config.fabric.query_count = 0;
    });
    assert_rejected("fabric.query_count", |config| {
        config.fabric.query_count = 4;
    });
    assert_rejected("fabric.candidate_count", |config| {
        config.fabric.candidate_count = 41;
    });
    assert_rejected("fabric.max_response_bytes", |config| {
        config.fabric.max_response_bytes = 0;
    });
}

#[test]
fn semantic_budgets_cannot_exceed_their_ceilings() {
    assert_rejected("context.max_semantic_chars", |config| {
        config.context.max_semantic_chars = 12_001;
    });
    assert_rejected("context.max_query_chars", |config| {
        config.context.max_query_chars = 1_001;
    });
    assert_rejected("context.max_candidate_chars", |config| {
        config.context.max_candidate_chars = 8_001;
    });
    assert_rejected("context.max_query_chars", |config| {
        config.context.max_query_chars = 0;
    });
}

#[test]
fn custom_redaction_patterns_are_bounded_and_must_compile() {
    assert_rejected("privacy.custom_redaction_patterns", |config| {
        config.privacy.custom_redaction_patterns = vec!["(unclosed".to_owned()];
    });
    assert_rejected("privacy.custom_redaction_patterns", |config| {
        config.privacy.custom_redaction_patterns =
            (0..33).map(|index| format!("secret-{index}")).collect();
    });
    assert_rejected("privacy.custom_redaction_patterns", |config| {
        config.privacy.custom_redaction_patterns = vec!["x".repeat(513)];
    });

    let mut config = baseline();
    config.privacy.custom_redaction_patterns = vec!["OPENAI_SK_[A-Za-z0-9]+".to_owned()];
    validate(&config).expect("a valid pattern is accepted");
}

#[test]
fn a_hosted_reranker_needs_explicit_egress_permission() {
    assert_rejected("reranker.provider", |config| {
        config.reranker.provider = RerankerProvider::Fireworks;
        config.reranker.fireworks = Some(hosted_adapter("qwen3-reranker-8b"));
    });
    assert_rejected("reranker.fallback", |config| {
        config.reranker.fallback = vec![RerankerProvider::OpenRouter];
        config.reranker.openrouter = Some(hosted_adapter("qwen3-reranker-8b"));
    });

    let mut config = baseline();
    config.privacy.allowed_hosted_providers = vec![HostedProvider::Fireworks];
    config.reranker.provider = RerankerProvider::Fireworks;
    config.reranker.fireworks = Some(hosted_adapter("qwen3-reranker-8b"));
    validate(&config).expect("an approved hosted provider is accepted");
}

#[test]
fn a_selected_provider_needs_its_adapter_table() {
    assert_rejected("reranker.provider", |config| {
        config.reranker.provider = RerankerProvider::LlamaSwap;
    });

    let mut config = baseline();
    config.privacy.allowed_hosted_providers = vec![HostedProvider::Fireworks];
    config.reranker.provider = RerankerProvider::Fireworks;
    let error = validate(&config).expect_err("a hosted provider without a table is rejected");
    assert!(error.to_string().contains("reranker.fireworks"));
}

#[test]
fn a_provider_may_not_repeat_as_its_own_fallback() {
    assert_rejected("reranker.fallback", |config| {
        config.reranker.fallback = vec![RerankerProvider::None];
    });
}

#[test]
fn a_local_reranker_must_stay_on_loopback() {
    for base_url in [
        "http://192.168.0.10:8080",
        "http://localhost:8080",
        "https://rerank.example.com",
    ] {
        assert_rejected("reranker.llama_swap.base_url", |config| {
            config.reranker.provider = RerankerProvider::LlamaSwap;
            config.reranker.llama_swap = Some(llama_swap_adapter(base_url));
        });
    }

    let mut config = baseline();
    config.reranker.provider = RerankerProvider::LlamaSwap;
    config.reranker.llama_swap = Some(llama_swap_adapter("http://127.0.0.1:8080"));
    validate(&config).expect("a loopback llama-swap origin is accepted");
}

#[test]
fn the_contextualizer_switch_and_provider_must_agree() {
    assert_rejected("contextualizer.enabled", |config| {
        config.contextualizer.enabled = true;
    });
    assert_rejected("contextualizer.provider", |config| {
        config.contextualizer.provider = ContextualizerProvider::Fireworks;
    });
}

#[test]
fn the_contextualizer_budget_covers_reasoning_tokens() {
    assert_rejected("contextualizer.max_output_tokens", |config| {
        config.contextualizer.max_output_tokens = 0;
    });
    assert_rejected("contextualizer.max_output_tokens", |config| {
        config.contextualizer.max_output_tokens = 16_385;
    });

    // A reasoning model spends hundreds of tokens before it emits any text, so a
    // streamed budget well above the non-streaming ceiling must be accepted.
    let mut config = baseline();
    config.contextualizer.max_output_tokens = 8192;
    validate(&config).expect("8192 tokens is accepted");
}

#[test]
fn both_stages_must_name_the_same_credential_variable_for_one_provider() {
    let mut config = baseline();
    config.privacy.allowed_hosted_providers = vec![HostedProvider::Fireworks];
    config.reranker.provider = RerankerProvider::Fireworks;
    config.reranker.fireworks = Some(hosted_adapter("qwen3-reranker-8b"));
    config.contextualizer.enabled = true;
    config.contextualizer.provider = ContextualizerProvider::Fireworks;
    config.contextualizer.fireworks = Some(HostedAdapter {
        api_key_env: "ANOTHER_FIREWORKS_KEY".to_owned(),
        ..hosted_adapter("deepseek-v4p1-flash")
    });

    let error = validate(&config).expect_err("the two tables disagree");
    assert!(error.to_string().contains("reranker.fireworks.api_key_env"));
}

#[test]
fn results_are_bounded() {
    assert_rejected("results.display_count", |config| {
        config.results.display_count = 0;
    });
    assert_rejected("results.display_count", |config| {
        config.results.display_count = 11;
    });
    assert_rejected("results.dismissal_ttl_hours", |config| {
        config.results.dismissal_ttl_hours = 721;
    });
}

#[test]
fn every_stage_needs_a_positive_budget() {
    assert_rejected("timeouts.fabric_ms", |config| {
        config.timeouts.fabric_ms = 0;
    });
    assert_rejected("timeouts.overall_pipeline_ms", |config| {
        config.timeouts.overall_pipeline_ms = 0;
    });
    assert_rejected("concurrency.pipelines", |config| {
        config.concurrency.pipelines = 0;
    });
    assert_rejected("concurrency.local_reranker", |config| {
        config.concurrency.local_reranker = 0;
    });
    assert_rejected("budgets.hosted_requests_per_minute", |config| {
        config.budgets.hosted_requests_per_minute = 0;
    });
    assert_rejected("budgets.hosted_burst", |config| {
        config.budgets.hosted_burst = 0;
    });
}

#[test]
fn cache_retention_is_bounded() {
    assert_rejected("cache.final_ttl_hours", |config| {
        config.cache.final_ttl_hours = 0;
    });
    assert_rejected("cache.max_entries", |config| {
        config.cache.max_entries = 0;
    });
    assert_rejected("cache.feedback_retention_days", |config| {
        config.cache.feedback_retention_days = 31;
    });
}

#[test]
fn a_relative_or_placeholder_cache_path_is_rejected() {
    for path in [
        "cache.sqlite3",
        "$HOME/cache.sqlite3",
        r"%LOCALAPPDATA%\cache",
    ] {
        let mut config = baseline();
        config.cache.path = path.to_owned();

        assert!(
            matches!(validate(&config), Err(ConfigError::UnresolvedPath { .. })),
            "path {path} was accepted"
        );
    }

    let mut config = baseline();
    config.cache.path = "/var/lib/obsidian-fabricd/cache.sqlite3".to_owned();
    validate(&config).expect("an absolute path is accepted");
}

#[test]
fn payload_capture_cannot_be_enabled() {
    assert_rejected("logging.log_payloads", |config| {
        config.logging.log_payloads = true;
    });
    assert_rejected("logging.level", |config| {
        config.logging.level = "loud".to_owned();
    });
}
