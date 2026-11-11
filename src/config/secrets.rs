//! Resolution of `*_env` names into secret values.
//!
//! [`Config`] only ever holds environment variable *names*. Values are resolved
//! once at startup into [`SecretString`] wrappers: they cannot be formatted by
//! accident, and their `Debug` output is redacted.

use secrecy::SecretString;

use crate::config::{Config, ConfigError};

/// A lookup from an environment variable name to its value.
type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Credentials resolved from the environment for this process.
///
/// Every field is optional except the local bearer token. A missing Fabric or
/// inference credential is a readiness observation, not a fatal error.
#[derive(Debug)]
pub struct Secrets {
    /// Bearer token every client request must present.
    pub local_token: SecretString,
    /// Fabric API key; absent means the service starts in `not_ready` mode.
    pub fabric_api_key: Option<SecretString>,
    /// llama-swap key, when the local server has authentication enabled.
    pub llama_swap_api_key: Option<SecretString>,
    /// Fireworks key, shared by its reranker and contextualizer adapters.
    pub fireworks_api_key: Option<SecretString>,
    /// OpenRouter key, shared by its reranker and contextualizer adapters.
    pub openrouter_api_key: Option<SecretString>,
}

/// Resolves every credential named by the configuration from the environment.
///
/// # Errors
///
/// Returns [`ConfigError::MissingSecret`] when the local token variable is unset
/// or empty. Missing provider credentials are reported as `None`.
pub fn resolve(config: &Config) -> Result<Secrets, ConfigError> {
    resolve_with(config, &|name| std::env::var(name).ok())
}

/// Resolves credentials through an explicit lookup, for tests and fixtures.
///
/// # Errors
///
/// Returns [`ConfigError::MissingSecret`] when the local token lookup yields
/// nothing or an empty value.
pub fn resolve_with(config: &Config, lookup: Lookup<'_>) -> Result<Secrets, ConfigError> {
    let local_token = required(lookup, &config.server.token_env)?;

    Ok(Secrets {
        local_token,
        fabric_api_key: optional(lookup, &config.fabric.api_key_env),
        llama_swap_api_key: config
            .reranker
            .llama_swap
            .as_ref()
            .and_then(|adapter| adapter.api_key_env.as_deref())
            .and_then(|name| optional(lookup, name)),
        fireworks_api_key: provider_key(config, "fireworks", lookup),
        openrouter_api_key: provider_key(config, "openrouter", lookup),
    })
}

fn required(lookup: Lookup<'_>, name: &str) -> Result<SecretString, ConfigError> {
    optional(lookup, name).ok_or_else(|| ConfigError::MissingSecret {
        env: name.to_owned(),
    })
}

fn optional(lookup: Lookup<'_>, name: &str) -> Option<SecretString> {
    lookup(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}

/// A hosted key is shared by whichever stage names it first; reranker and
/// contextualizer adapter tables for the same provider must agree.
fn provider_key(config: &Config, provider: &str, lookup: Lookup<'_>) -> Option<SecretString> {
    let reranker_name = match provider {
        "fireworks" => config
            .reranker
            .fireworks
            .as_ref()
            .map(|adapter| adapter.api_key_env.as_str()),
        _ => config
            .reranker
            .openrouter
            .as_ref()
            .map(|adapter| adapter.api_key_env.as_str()),
    };
    let contextualizer_name = match provider {
        "fireworks" => config
            .contextualizer
            .fireworks
            .as_ref()
            .map(|adapter| adapter.api_key_env.as_str()),
        _ => config
            .contextualizer
            .openrouter
            .as_ref()
            .map(|adapter| adapter.api_key_env.as_str()),
    };

    reranker_name
        .or(contextualizer_name)
        .and_then(|name| optional(lookup, name))
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::resolve_with;
    use crate::config::{ConfigError, parse};

    const BASELINE: &str = include_str!("../../tests/fixtures/config/baseline.toml");

    fn baseline() -> super::Config {
        parse(BASELINE).expect("the baseline fixture parses")
    }

    #[test]
    fn a_missing_local_token_is_fatal_and_names_the_variable() {
        let config = baseline();

        let error = resolve_with(&config, &|_| None).expect_err("the token is required");

        let ConfigError::MissingSecret { env } = error else {
            panic!("expected a missing-secret error, got {error:?}");
        };
        assert_eq!(env, "OBSIDIAN_FABRICD_TOKEN");
    }

    #[test]
    fn an_empty_local_token_counts_as_missing() {
        let config = baseline();

        let error = resolve_with(&config, &|_name| Some(String::new()))
            .expect_err("an empty token is not a credential");

        assert!(matches!(error, ConfigError::MissingSecret { .. }));
    }

    #[test]
    fn a_missing_fabric_key_is_not_fatal() {
        let config = baseline();

        let secrets = resolve_with(&config, &|name| {
            (name == "OBSIDIAN_FABRICD_TOKEN").then(|| "local-token".to_owned())
        })
        .expect("only the local token is required");

        assert!(secrets.fabric_api_key.is_none());
        assert!(secrets.fireworks_api_key.is_none());
        assert_eq!(secrets.local_token.expose_secret(), "local-token");
    }

    #[test]
    fn every_configured_provider_key_is_resolved() {
        let config = parse(include_str!(
            "../../tests/fixtures/config/hosted-fallback.toml"
        ))
        .expect("the hosted fixture parses");

        let secrets = resolve_with(&config, &|name| Some(format!("value-of-{name}")))
            .expect("the local token resolves");

        assert_eq!(
            secrets
                .fireworks_api_key
                .as_ref()
                .map(|s| s.expose_secret()),
            Some("value-of-FIREWORKS_API_KEY")
        );
        assert_eq!(
            secrets
                .openrouter_api_key
                .as_ref()
                .map(|s| s.expose_secret()),
            Some("value-of-OPENROUTER_API_KEY")
        );
        assert!(secrets.fabric_api_key.is_some());
    }

    #[test]
    fn debug_output_does_not_reveal_a_secret() {
        let config = baseline();

        let secrets = resolve_with(&config, &|name| {
            (name == "OBSIDIAN_FABRICD_TOKEN").then(|| "super-secret-token".to_owned())
        })
        .expect("the local token resolves");

        let rendered = format!("{secrets:?}");
        assert!(!rendered.contains("super-secret-token"));
    }
}
