//! Process-wide `tracing` subscriber setup.
//!
//! Payload capture is **off** and there is no code path that turns it on. Normal
//! operation logs lengths, counts, stages, and durations only: never document
//! paths, titles, anchor text, queries, excerpts, resource URLs, tokens, or
//! content fingerprints. A development payload dump, if one is ever added, needs
//! its own explicit flag and synthetic data.

use tracing::Subscriber;
use tracing::level_filters::LevelFilter;

/// How each log record is encoded on the writer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LogFormat {
    /// Human-readable single-line records, for a terminal.
    #[default]
    Text,
    /// One JSON object per record, for a log collector.
    Json,
}

/// Subscriber settings resolved from configuration before startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    /// Highest level that is logged. Configuration is authoritative, so an
    /// environment variable cannot silently widen or narrow it.
    pub level: LevelFilter,
    /// Record encoding.
    pub format: LogFormat,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            level: LevelFilter::INFO,
            format: LogFormat::default(),
        }
    }
}

/// Why the process-wide subscriber could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// A subscriber is already installed for this process.
    #[error("a tracing subscriber is already installed")]
    AlreadyInstalled,
}

/// Installs the global subscriber, writing records to standard error.
///
/// Call this once during startup. There is no payload capture and no way to
/// enable one from here.
///
/// # Errors
///
/// Returns [`InitError::AlreadyInstalled`] when a subscriber is already set for
/// this process.
pub fn init(settings: &Settings) -> Result<(), InitError> {
    tracing::subscriber::set_global_default(subscriber(settings))
        .map_err(|_| InitError::AlreadyInstalled)
}

fn subscriber(settings: &Settings) -> Box<dyn Subscriber + Send + Sync> {
    let builder = tracing_subscriber::fmt()
        .with_max_level(settings.level)
        .with_writer(std::io::stderr);

    match settings.format {
        LogFormat::Text => Box::new(builder.finish()),
        LogFormat::Json => Box::new(builder.json().finish()),
    }
}

#[cfg(test)]
mod tests {
    use super::{InitError, LogFormat, Settings, init, subscriber};
    use tracing::level_filters::LevelFilter;

    #[test]
    fn defaults_to_info_level_and_text_output() {
        let settings = Settings::default();

        assert_eq!(settings.level, LevelFilter::INFO);
        assert_eq!(settings.format, LogFormat::Text);
    }

    #[test]
    fn emits_a_record_in_each_format_without_panicking() {
        for format in [LogFormat::Text, LogFormat::Json] {
            let settings = Settings {
                level: LevelFilter::INFO,
                format,
            };

            // A scrubbed event: only a stage name and a count, never content.
            tracing::subscriber::with_default(subscriber(&settings), || {
                tracing::info!(stage = "smoke", count = 1, "record emitted");
            });
        }
    }

    #[test]
    fn installs_the_subscriber_exactly_once() {
        let json = Settings {
            level: LevelFilter::DEBUG,
            format: LogFormat::Json,
        };

        assert!(init(&json).is_ok());

        assert!(matches!(init(&json), Err(InitError::AlreadyInstalled)));
    }
}
