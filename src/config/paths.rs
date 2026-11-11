//! Platform path resolution for configured paths.
//!
//! TOML does not expand environment variables, so an explicit path must be
//! absolute and must not contain a placeholder. `"auto"` resolves through the
//! platform data directory: `%LOCALAPPDATA%` on Windows, `Application Support` on
//! macOS, and the XDG data directory on other targets.

use std::path::{Path, PathBuf};

use crate::config::ConfigError;

/// The literal value that selects the platform data directory.
const AUTO: &str = "auto";
/// Directory and file name used for the cache database.
const APP_DIR: &str = "obsidian-fabricd";
const DATABASE_FILE: &str = "cache.sqlite3";

/// Resolves `cache.path` into an absolute database file path.
///
/// # Errors
///
/// Returns [`ConfigError::UnresolvedPath`] when an explicit path is relative or
/// contains an unexpanded placeholder, or when no platform data directory is
/// available.
pub fn cache_file(config: &crate::config::Config) -> Result<PathBuf, ConfigError> {
    let configured = config.cache.path.trim();

    if configured == AUTO {
        return Ok(data_dir()?.join(APP_DIR).join(DATABASE_FILE));
    }

    let path = Path::new(configured);
    if has_placeholder(configured) {
        return Err(ConfigError::UnresolvedPath {
            field: "cache.path",
            detail: "environment variable placeholders are not expanded; use an absolute path or \"auto\""
                .to_owned(),
        });
    }
    if !path.is_absolute() {
        return Err(ConfigError::UnresolvedPath {
            field: "cache.path",
            detail: "path must be absolute or \"auto\"".to_owned(),
        });
    }

    Ok(path.to_path_buf())
}

fn has_placeholder(value: &str) -> bool {
    value.contains('$') || value.contains('%')
}

fn data_dir() -> Result<PathBuf, ConfigError> {
    #[cfg(target_os = "windows")]
    let directory = non_empty("LOCALAPPDATA").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    let directory =
        non_empty("HOME").map(|home| Path::new(&home).join("Library").join("Application Support"));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let directory = xdg_data_dir();

    directory.ok_or_else(|| ConfigError::UnresolvedPath {
        field: "cache.path",
        detail: "no platform data directory is available; set `cache.path` to an absolute path"
            .to_owned(),
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn xdg_data_dir() -> Option<PathBuf> {
    // A relative XDG_DATA_HOME is invalid, so it is ignored.
    if let Some(dir) = non_empty("XDG_DATA_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }

    non_empty("HOME").map(|home| Path::new(&home).join(".local").join("share"))
}

fn non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::has_placeholder;

    #[test]
    fn placeholders_are_detected() {
        assert!(has_placeholder(r"%LOCALAPPDATA\obsidian-fabricd"));
        assert!(has_placeholder("$HOME/.cache/x"));
        assert!(!has_placeholder("/var/lib/obsidian-fabricd/cache.sqlite3"));
    }
}
