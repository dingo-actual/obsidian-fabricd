//! Command-line surface.
//!
//! Today the binary exposes the configuration commands; the listener and the
//! diagnostics commands are added as the service below them exists.
//!
//! Exit status zero means the requested work completed. Every failure mode has a
//! distinct documented code so a scheduled task or installer can react.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};

use crate::config::{Config, ConfigError};
use crate::telemetry;

/// Exit status codes. Zero means success and one means an unexpected failure.
pub mod status {
    /// Everything the operator asked for happened.
    pub const SUCCESS: u8 = 0;
    /// The process failed for a reason with no dedicated code.
    pub const UNEXPECTED: u8 = 1;
    /// The configuration file is missing, unreadable, or invalid.
    pub const INVALID_CONFIGURATION: u8 = 2;
    /// A required credential is not present in the environment.
    pub const MISSING_CREDENTIAL: u8 = 3;
    /// The configured listener port is already in use.
    pub const PORT_CONFLICT: u8 = 4;
    /// A live provider diagnostic failed.
    pub const PROVIDER_DIAGNOSTIC: u8 = 5;
    /// The cache database could not be opened or migrated.
    pub const CACHE_FAILURE: u8 = 6;
}

/// The command line.
#[derive(Debug, Parser)]
#[command(
    name = "obsidian-fabricd",
    version,
    about = "Loopback recommendation service for Obsidian note context",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect and check the configuration file.
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Check the file against the schema and every cross-section rule.
    Validate {
        /// Path to the configuration file.
        #[arg(long, value_name = "PATH")]
        config: PathBuf,
    },
    /// Write the effective configuration to standard output. Secrets never
    /// appear: the file holds environment variable names only.
    Print {
        /// Path to the configuration file.
        #[arg(long, value_name = "PATH")]
        config: PathBuf,
        /// Accepted for compatibility; output is always secret-free.
        #[arg(long)]
        redacted: bool,
    },
}

/// Runs the command line and returns the process exit status.
pub fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(exit_code_for(&error))
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Config { action } => run_config(action),
    }
}

fn run_config(action: ConfigCommand) -> anyhow::Result<()> {
    let path = match &action {
        ConfigCommand::Validate { config } | ConfigCommand::Print { config, .. } => config,
    };

    let config = load_validated(path)?;
    telemetry::init(&crate::config::telemetry_settings(&config)?)
        .context("cannot start logging")?;

    match action {
        ConfigCommand::Validate { .. } => Ok(()),
        ConfigCommand::Print { .. } => {
            let rendered = crate::config::render_redacted(&config)?;
            print!("{rendered}");
            Ok(())
        }
    }
}

/// Loads the file, then applies every schema rule.
fn load_validated(path: &Path) -> Result<Config, ConfigError> {
    let config = crate::config::load(path)?;
    crate::config::validate(&config)?;

    Ok(config)
}

fn exit_code_for(error: &anyhow::Error) -> u8 {
    match error.downcast_ref::<ConfigError>() {
        Some(ConfigError::MissingSecret { .. }) => status::MISSING_CREDENTIAL,
        Some(_) => status::INVALID_CONFIGURATION,
        None => status::UNEXPECTED,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use clap::{CommandFactory, Parser};

    use super::{Cli, Command, ConfigCommand, exit_code_for, load_validated, status};
    use crate::config::{ConfigError, parse};

    const BASELINE: &str = include_str!("../tests/fixtures/config/baseline.toml");
    const INVALID_VERSION: &str = include_str!("../tests/fixtures/config/invalid-version.toml");

    fn write_config(directory: &tempfile::TempDir, contents: &str) -> std::path::PathBuf {
        let path = directory.path().join("config.toml");
        let mut file = std::fs::File::create(&path).expect("the temporary file is created");
        file.write_all(contents.as_bytes())
            .expect("the fixture is written");

        path
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn validate_requires_a_configuration_path() {
        let error = Cli::try_parse_from(["obsidian-fabricd", "config", "validate"])
            .expect_err("--config is required");

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn print_accepts_the_redacted_flag() {
        let cli = Cli::try_parse_from([
            "obsidian-fabricd",
            "config",
            "print",
            "--redacted",
            "--config",
            "settings.toml",
        ])
        .expect("the documented command line parses");

        let Command::Config {
            action: ConfigCommand::Print { config, redacted },
        } = cli.command
        else {
            panic!("expected the print command");
        };
        assert!(redacted);
        assert_eq!(config, std::path::Path::new("settings.toml"));
    }

    #[test]
    fn a_valid_file_loads_and_validates() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = write_config(&directory, BASELINE);

        let config = load_validated(&path).expect("the baseline validates");

        assert_eq!(config, parse(BASELINE).expect("the baseline parses"));
    }

    #[test]
    fn an_invalid_file_reports_the_validation_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = write_config(&directory, INVALID_VERSION);

        let error = load_validated(&path).expect_err("version 2 is rejected");

        assert!(matches!(
            error,
            ConfigError::UnsupportedVersion { found: 2, .. }
        ));
    }

    #[test]
    fn a_missing_file_reports_a_read_error() {
        let error = load_validated(std::path::Path::new("/nonexistent/config.toml"))
            .expect_err("a missing file is an error");

        assert!(matches!(error, ConfigError::Read { .. }));
        assert_eq!(
            exit_code_for(&anyhow::Error::from(error)),
            status::INVALID_CONFIGURATION
        );
    }

    #[test]
    fn a_missing_credential_has_its_own_exit_code() {
        let error = ConfigError::MissingSecret {
            env: "OBSIDIAN_FABRICD_TOKEN".to_owned(),
        };

        assert_eq!(
            exit_code_for(&anyhow::Error::from(error)),
            status::MISSING_CREDENTIAL
        );
    }

    #[test]
    fn a_malformed_file_has_the_invalid_configuration_code() {
        let error = parse("version = 1\n[privacy]\nunknown = 1\n").expect_err("unknown key");

        assert_eq!(
            exit_code_for(&anyhow::Error::from(error)),
            status::INVALID_CONFIGURATION
        );
    }
}
