//! Destination rules for configured origins.
//!
//! A remote processor must be reached over HTTPS without embedded credentials. A
//! local processor must be a literal loopback address: hostname resolution is
//! outside the deployment model, so `localhost` is rejected.

use std::net::IpAddr;
use std::str::FromStr;

use crate::config::ConfigError;

/// Requires an absolute HTTPS URL with a host and no embedded credentials.
pub(super) fn require_https(field: &str, value: &str) -> Result<(), ConfigError> {
    let url = parse(field, value)?;

    if url.scheme() != "https" {
        return Err(invalid(field, format!("`{value}` must use https")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            field,
            format!("`{value}` must not embed credentials"),
        ));
    }

    Ok(())
}

/// Requires an absolute HTTP or HTTPS URL whose host is a literal loopback address.
pub(super) fn require_loopback(field: &str, value: &str) -> Result<(), ConfigError> {
    let url = parse(field, value)?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid(field, format!("`{value}` must use http or https")));
    }

    let host = url
        .host_str()
        // IPv6 hosts are serialised with brackets.
        .map(|host| host.trim_matches(['[', ']']))
        .ok_or_else(|| invalid(field, format!("`{value}` has no host")))?;
    let address = IpAddr::from_str(host).map_err(|_| {
        invalid(
            field,
            format!("`{value}` must use a literal loopback address such as 127.0.0.1 or [::1]"),
        )
    })?;
    if !address.is_loopback() {
        return Err(invalid(
            field,
            format!("`{value}` is not a loopback address"),
        ));
    }

    Ok(())
}

/// Requires a path that starts at the origin root and contains no whitespace.
pub(super) fn require_path(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.starts_with('/') && !value.contains(char::is_whitespace) {
        return Ok(());
    }

    Err(invalid(
        field,
        format!("`{value}` must start with `/` and contain no whitespace"),
    ))
}

fn parse(field: &str, value: &str) -> Result<reqwest::Url, ConfigError> {
    let url = reqwest::Url::parse(value)
        .map_err(|source| invalid(field, format!("`{value}` is not an absolute URL: {source}")))?;

    if url.host_str().is_none() {
        return Err(invalid(field, format!("`{value}` has no host")));
    }

    Ok(url)
}

fn invalid(field: &str, detail: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        field: field.to_owned(),
        detail: detail.into(),
    }
}
