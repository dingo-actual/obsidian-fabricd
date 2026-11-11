//! Configuration contract.
//!
//! Holds the versioned TOML schema, secret-name resolution, platform path
//! resolution, cross-section validation, and the pipeline fingerprint that
//! partitions the cache. Unknown keys are rejected: a misspelled privacy or
//! fallback setting must fail validation rather than take effect silently.
//!
//! The effective configuration never holds a secret value. Secret fields appear
//! here only as environment variable *names*; [`secrets::resolve`] turns them into
//! values at startup.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sha2::{Digest, Sha256};
use tracing::level_filters::LevelFilter;

mod origin;
mod paths;
mod schema;
mod secrets;
mod validate;

pub use paths::cache_file;
pub use schema::*;
pub use secrets::{Secrets, resolve, resolve_with};
pub use validate::validate;

use crate::telemetry;

/// Everything that can go wrong while loading, validating, or resolving configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read configuration {path}")]
    Read {
        /// Configured file path.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid TOML or does not match the schema.
    #[error("configuration is not valid TOML")]
    Parse {
        /// Parser failure, with a line and column.
        #[source]
        source: toml::de::Error,
    },
    /// The schema version is not one this build understands.
    #[error("unsupported configuration version {found}; this build supports version {supported}")]
    UnsupportedVersion {
        /// Version found in the file.
        found: u32,
        /// Version this build supports.
        supported: u32,
    },
    /// A value is well-formed TOML but contradicts another setting or a bound.
    #[error("invalid `{field}`: {detail}")]
    Invalid {
        /// Dotted configuration path.
        field: String,
        /// What is wrong and what the operator can do.
        detail: String,
    },
    /// A required credential is not present in the environment.
    #[error("missing required credential in environment variable `{env}`")]
    MissingSecret {
        /// Variable name from the configuration.
        env: String,
    },
    /// A configured path cannot be resolved.
    #[error("cannot resolve `{field}`: {detail}")]
    UnresolvedPath {
        /// Dotted configuration path.
        field: &'static str,
        /// Why resolution failed.
        detail: String,
    },
    /// The effective configuration cannot be written back as TOML.
    #[error("cannot render the configuration")]
    Render {
        /// Serialiser failure.
        #[source]
        source: toml::ser::Error,
    },
}

/// Parses configuration text. Does not run cross-section validation.
///
/// # Errors
///
/// Returns [`ConfigError::Parse`] for malformed TOML, unknown keys, unknown
/// section names, or unknown enum values.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    toml::from_str(text).map_err(|source| ConfigError::Parse { source })
}

/// Reads and parses a configuration file. Does not run cross-section validation.
///
/// # Errors
///
/// Returns [`ConfigError::Read`] when the file cannot be read and
/// [`ConfigError::Parse`] when its contents are invalid.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    parse(&text)
}

/// Renders the effective configuration as TOML.
///
/// Only non-secret values exist in [`Config`], so this output is safe to paste
/// into an issue or compare between machines. It re-parses into an equal
/// configuration.
///
/// # Errors
///
/// Returns [`ConfigError::Render`] if the schema cannot be serialised.
pub fn render_redacted(config: &Config) -> Result<String, ConfigError> {
    toml::to_string_pretty(config).map_err(|source| ConfigError::Render { source })
}

/// Stable fingerprint of everything that changes what a pipeline produces.
///
/// The listener and logging sections are excluded: they never alter a query,
/// a provider call, or a candidate. The fingerprint partitions the cache, so a
/// change to any remaining field must invalidate old entries.
pub fn fingerprint(config: &Config) -> String {
    let mut value = serde_json::to_value(config).expect("the configuration schema serialises");
    if let Some(fields) = value.as_object_mut() {
        fields.remove("server");
        fields.remove("logging");
    }
    let canonical = serde_json::to_string(&value).expect("a JSON value serialises");

    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Logs settings derived from the configuration.
///
/// # Errors
///
/// Returns [`ConfigError::Invalid`] when `logging.level` is not a level directive.
pub fn telemetry_settings(config: &Config) -> Result<telemetry::Settings, ConfigError> {
    let level = LevelFilter::from_str(&config.logging.level).map_err(|_| ConfigError::Invalid {
        field: "logging.level".to_owned(),
        detail: format!(
            "`{}` is not one of off, error, warn, info, debug, trace",
            config.logging.level
        ),
    })?;

    Ok(telemetry::Settings {
        level,
        format: telemetry::LogFormat::Text,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ConfigError, cache_file, fingerprint, load, parse, render_redacted};

    const BASELINE: &str = include_str!("../../tests/fixtures/config/baseline.toml");

    fn baseline() -> super::Config {
        parse(BASELINE).expect("the baseline fixture parses")
    }

    #[test]
    fn loading_a_missing_file_names_the_path() {
        let error = load(Path::new("/nonexistent/obsidian-fabricd.toml"))
            .expect_err("a missing file must fail");

        let ConfigError::Read { path, .. } = error else {
            panic!("expected a read error, got {error:?}");
        };
        assert!(path.ends_with("obsidian-fabricd.toml"));
    }

    #[test]
    fn rendering_drops_no_field() {
        let config = baseline();

        let printed = render_redacted(&config).expect("the config renders");
        let reparsed = parse(&printed).expect("the printed config re-parses");

        assert_eq!(reparsed, config);
    }

    #[test]
    fn the_fingerprint_ignores_the_listener_and_logging() {
        let mut config = baseline();
        let original = fingerprint(&config);

        config.server.port = 1;
        config.logging.level = "trace".to_owned();

        assert_eq!(fingerprint(&config), original);
    }

    #[test]
    fn the_fingerprint_tracks_the_egress_policy() {
        let mut config = baseline();
        let original = fingerprint(&config);

        config.privacy.include_document_title_remote = true;

        assert_ne!(fingerprint(&config), original);
    }

    #[test]
    fn the_cache_file_is_absolute_for_auto_and_rejects_relative_paths() {
        let config = baseline();

        let auto = cache_file(&config).expect("auto resolves");
        assert!(auto.is_absolute());
        assert!(auto.ends_with("cache.sqlite3"));

        let mut relative = baseline();
        relative.cache.path = "relative/cache.sqlite3".to_owned();
        assert!(matches!(
            cache_file(&relative),
            Err(ConfigError::UnresolvedPath { .. })
        ));
    }
}
