//! Cross-section configuration rules.
//!
//! These are fatal errors, not warnings: a contradiction between the egress policy
//! and a configured provider, an unsafe destination, or an out-of-range bound must
//! stop the process before it accepts a request.

use std::net::IpAddr;
use std::str::FromStr;

use regex::RegexBuilder;
use tracing::level_filters::LevelFilter;

use crate::config::schema::{Config, SUPPORTED_VERSION};
use crate::config::{ConfigError, cache_file};

mod providers;

use crate::config::origin::{require_https, require_path};
use providers::{validate_contextualizer, validate_reranker};

/// Maximum number of operator-supplied redaction patterns.
const MAX_CUSTOM_PATTERNS: usize = 32;
/// Maximum length of one operator-supplied redaction pattern.
const MAX_PATTERN_CHARS: usize = 512;
/// Regex engine size limit, so one pattern cannot stall startup.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;
/// Upper bound for the session registry, well below a runaway client.
const MAX_SESSIONS: u64 = 64;
/// Longest session idle timeout.
const MAX_IDLE_SECONDS: u64 = 86_400;
/// Upper bound for a displayed result set.
const MAX_DISPLAY_COUNT: u64 = 10;
/// Longest retention of interaction feedback.
const MAX_FEEDBACK_RETENTION_DAYS: u64 = 30;

/// Validates every cross-section rule.
///
/// # Errors
///
/// Returns the first violation as [`ConfigError::UnsupportedVersion`],
/// [`ConfigError::Invalid`], or [`ConfigError::UnresolvedPath`].
pub fn validate(config: &Config) -> Result<(), ConfigError> {
    validate_version(config)?;
    validate_server(config)?;
    validate_fabric(config)?;
    validate_privacy(config)?;
    validate_context(config)?;
    validate_reranker(config)?;
    validate_contextualizer(config)?;
    validate_results(config)?;
    validate_timeouts(config)?;
    validate_concurrency(config)?;
    validate_budgets(config)?;
    validate_cache(config)?;
    validate_logging(config)
}

fn validate_version(config: &Config) -> Result<(), ConfigError> {
    if config.version == SUPPORTED_VERSION {
        return Ok(());
    }

    Err(ConfigError::UnsupportedVersion {
        found: config.version,
        supported: SUPPORTED_VERSION,
    })
}

fn validate_server(config: &Config) -> Result<(), ConfigError> {
    let server = &config.server;

    let address = IpAddr::from_str(&server.bind).map_err(|_| {
        invalid(
            "server.bind",
            format!(
                "`{}` is not a literal address; use 127.0.0.1 or ::1",
                server.bind
            ),
        )
    })?;
    if !address.is_loopback() {
        return Err(invalid(
            "server.bind",
            format!("`{}` is not a loopback address", server.bind),
        ));
    }

    at_least("server.port", u64::from(server.port), 1)?;
    non_empty("server.token_env", &server.token_env)?;
    within(
        "server.max_sessions",
        server.max_sessions as u64,
        1,
        MAX_SESSIONS,
    )?;
    within(
        "server.session_idle_seconds",
        server.session_idle_seconds,
        1,
        MAX_IDLE_SECONDS,
    )?;
    at_least(
        "server.request_limit_bytes",
        server.request_limit_bytes,
        1_024,
    )?;
    at_least("server.event_limit_bytes", server.event_limit_bytes, 1_024)
}

fn validate_fabric(config: &Config) -> Result<(), ConfigError> {
    let fabric = &config.fabric;

    require_https("fabric.base_url", &fabric.base_url)?;
    require_path("fabric.search_path", &fabric.search_path)?;
    non_empty("fabric.api_key_env", &fabric.api_key_env)?;
    non_empty("fabric.account_namespace", &fabric.account_namespace)?;
    non_empty("fabric.credential_epoch", &fabric.credential_epoch)?;
    within("fabric.query_count", u64::from(fabric.query_count), 1, 3)?;
    within(
        "fabric.candidate_count",
        u64::from(fabric.candidate_count),
        1,
        40,
    )?;
    at_least(
        "fabric.max_response_bytes",
        fabric.max_response_bytes,
        1_024,
    )
}

fn validate_privacy(config: &Config) -> Result<(), ConfigError> {
    let patterns = &config.privacy.custom_redaction_patterns;

    if patterns.len() > MAX_CUSTOM_PATTERNS {
        return Err(invalid(
            "privacy.custom_redaction_patterns",
            format!(
                "at most {MAX_CUSTOM_PATTERNS} patterns are allowed, found {}",
                patterns.len()
            ),
        ));
    }

    for (index, pattern) in patterns.iter().enumerate() {
        let position = index + 1;
        if pattern.chars().count() > MAX_PATTERN_CHARS {
            return Err(invalid(
                "privacy.custom_redaction_patterns",
                format!("pattern {position} is longer than {MAX_PATTERN_CHARS} characters"),
            ));
        }

        // The pattern text is not echoed: it is operator input that may itself
        // contain a secret.
        RegexBuilder::new(pattern)
            .size_limit(PATTERN_SIZE_LIMIT)
            .build()
            .map_err(|source| {
                invalid(
                    "privacy.custom_redaction_patterns",
                    format!("pattern {position} does not compile: {source}"),
                )
            })?;
    }

    Ok(())
}

fn validate_context(config: &Config) -> Result<(), ConfigError> {
    let context = &config.context;

    within(
        "context.max_semantic_chars",
        u64::from(context.max_semantic_chars),
        1,
        12_000,
    )?;
    within(
        "context.max_query_chars",
        u64::from(context.max_query_chars),
        1,
        1_000,
    )?;
    within(
        "context.max_candidate_chars",
        u64::from(context.max_candidate_chars),
        1,
        8_000,
    )
}

fn validate_results(config: &Config) -> Result<(), ConfigError> {
    let results = &config.results;

    within(
        "results.display_count",
        u64::from(results.display_count),
        1,
        MAX_DISPLAY_COUNT,
    )?;
    within(
        "results.dismissal_ttl_hours",
        u64::from(results.dismissal_ttl_hours),
        1,
        24 * MAX_FEEDBACK_RETENTION_DAYS,
    )
}

fn validate_timeouts(config: &Config) -> Result<(), ConfigError> {
    let timeouts = &config.timeouts;

    for (field, value) in [
        ("timeouts.connect_ms", timeouts.connect_ms),
        ("timeouts.fabric_ms", timeouts.fabric_ms),
        ("timeouts.contextualizer_ms", timeouts.contextualizer_ms),
        ("timeouts.local_reranker_ms", timeouts.local_reranker_ms),
        ("timeouts.hosted_reranker_ms", timeouts.hosted_reranker_ms),
        ("timeouts.overall_pipeline_ms", timeouts.overall_pipeline_ms),
    ] {
        at_least(field, value, 1)?;
    }

    Ok(())
}

fn validate_concurrency(config: &Config) -> Result<(), ConfigError> {
    let concurrency = &config.concurrency;

    for (field, value) in [
        ("concurrency.pipelines", concurrency.pipelines),
        ("concurrency.fabric", concurrency.fabric),
        ("concurrency.contextualizer", concurrency.contextualizer),
        ("concurrency.local_reranker", concurrency.local_reranker),
        ("concurrency.hosted_reranker", concurrency.hosted_reranker),
    ] {
        at_least(field, u64::from(value), 1)?;
    }

    Ok(())
}

fn validate_budgets(config: &Config) -> Result<(), ConfigError> {
    let budgets = &config.budgets;

    at_least(
        "budgets.hosted_requests_per_minute",
        u64::from(budgets.hosted_requests_per_minute),
        1,
    )?;
    at_least("budgets.hosted_burst", u64::from(budgets.hosted_burst), 1)
}

fn validate_cache(config: &Config) -> Result<(), ConfigError> {
    let cache = &config.cache;

    at_least("cache.final_ttl_hours", u64::from(cache.final_ttl_hours), 1)?;
    at_least(
        "cache.prefix_ttl_hours",
        u64::from(cache.prefix_ttl_hours),
        1,
    )?;
    at_least(
        "cache.degraded_ttl_seconds",
        u64::from(cache.degraded_ttl_seconds),
        1,
    )?;
    at_least("cache.max_bytes", cache.max_bytes, 1)?;
    at_least("cache.max_entries", cache.max_entries, 1)?;
    within(
        "cache.feedback_retention_days",
        u64::from(cache.feedback_retention_days),
        1,
        MAX_FEEDBACK_RETENTION_DAYS,
    )?;

    cache_file(config).map(|_| ())
}

fn validate_logging(config: &Config) -> Result<(), ConfigError> {
    let logging = &config.logging;

    LevelFilter::from_str(&logging.level).map_err(|_| {
        invalid(
            "logging.level",
            format!(
                "`{}` is not one of off, error, warn, info, debug, trace",
                logging.level
            ),
        )
    })?;

    if logging.log_payloads {
        return Err(invalid(
            "logging.log_payloads",
            "payload capture is not implemented; the service logs lengths, counts, stages, and durations only",
        ));
    }

    Ok(())
}

pub(super) fn non_empty(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(invalid(field, "must not be empty"));
    }

    Ok(())
}

fn at_least(field: &str, value: u64, minimum: u64) -> Result<(), ConfigError> {
    if value >= minimum {
        return Ok(());
    }

    Err(invalid(
        field,
        format!("must be at least {minimum}, found {value}"),
    ))
}

pub(super) fn within(
    field: &str,
    value: u64,
    minimum: u64,
    maximum: u64,
) -> Result<(), ConfigError> {
    if (minimum..=maximum).contains(&value) {
        return Ok(());
    }

    Err(invalid(
        field,
        format!("must be between {minimum} and {maximum}, found {value}"),
    ))
}

fn invalid(field: &str, detail: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        field: field.to_owned(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
