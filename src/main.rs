//! Binary entry point: compose the service from the library modules.
//!
//! Subcommands, configuration loading, and the listener arrive in later phases.
//! For now the binary only installs telemetry so the process has working logging.

use obsidian_fabricd::telemetry;

fn main() -> anyhow::Result<()> {
    telemetry::init(&telemetry::Settings::default())?;
    Ok(())
}
