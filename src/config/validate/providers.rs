//! Provider egress rules for the reranker and contextualizer stages.
//!
//! A provider may only be used when the egress policy names it, and it must have
//! the adapter table that carries its destination, model, and credential variable.

use crate::config::ConfigError;
use crate::config::schema::{
    AdapterPresence, Config, ContextualizerProvider, HostedAdapter, HostedProvider,
};

use crate::config::origin::{require_https, require_loopback, require_path};

use super::{non_empty, within};

/// Maximum contextualizer output budget, including reasoning tokens.
const MAX_PREFIX_TOKENS: u64 = 16_384;

pub(super) fn validate_reranker(config: &Config) -> Result<(), ConfigError> {
    let reranker = &config.reranker;
    let allowed = &config.privacy.allowed_hosted_providers;

    check_provider(
        "reranker",
        "reranker.provider",
        reranker.provider.as_str(),
        reranker.provider.hosted(),
        reranker.adapter_for(reranker.provider),
        allowed,
    )?;

    for provider in &reranker.fallback {
        if *provider == reranker.provider {
            return Err(ConfigError::Invalid {
                field: "reranker.fallback".to_owned(),
                detail: format!("`{}` already is the primary provider", provider.as_str()),
            });
        }
        check_provider(
            "reranker",
            "reranker.fallback",
            provider.as_str(),
            provider.hosted(),
            reranker.adapter_for(*provider),
            allowed,
        )?;
    }

    if let Some(adapter) = &reranker.llama_swap {
        require_loopback("reranker.llama_swap.base_url", &adapter.base_url)?;
        require_path("reranker.llama_swap.endpoint", &adapter.endpoint)?;
        non_empty("reranker.llama_swap.model", &adapter.model)?;
    }

    validate_hosted_adapters(
        "reranker",
        reranker.fireworks.as_ref(),
        reranker.openrouter.as_ref(),
        &config.contextualizer.fireworks,
        &config.contextualizer.openrouter,
    )
}

pub(super) fn validate_contextualizer(config: &Config) -> Result<(), ConfigError> {
    let contextualizer = &config.contextualizer;
    let allowed = &config.privacy.allowed_hosted_providers;

    match (contextualizer.enabled, contextualizer.provider) {
        (false, ContextualizerProvider::Disabled) => {}
        (true, ContextualizerProvider::Disabled) => {
            return Err(ConfigError::Invalid {
                field: "contextualizer.enabled".to_owned(),
                detail: "the stage is enabled but `contextualizer.provider` is `disabled`"
                    .to_owned(),
            });
        }
        (false, provider) => {
            return Err(ConfigError::Invalid {
                field: "contextualizer.provider".to_owned(),
                detail: format!(
                    "`{}` is configured but `contextualizer.enabled` is false",
                    provider.as_str()
                ),
            });
        }
        (true, provider) => check_provider(
            "contextualizer",
            "contextualizer.provider",
            provider.as_str(),
            provider.hosted(),
            contextualizer.adapter_for(provider),
            allowed,
        )?,
    }

    for provider in &contextualizer.fallback {
        if *provider == contextualizer.provider {
            return Err(ConfigError::Invalid {
                field: "contextualizer.fallback".to_owned(),
                detail: format!("`{}` already is the primary provider", provider.as_str()),
            });
        }
        check_provider(
            "contextualizer",
            "contextualizer.fallback",
            provider.as_str(),
            provider.hosted(),
            contextualizer.adapter_for(*provider),
            allowed,
        )?;
    }

    within(
        "contextualizer.max_output_tokens",
        u64::from(contextualizer.max_output_tokens),
        1,
        MAX_PREFIX_TOKENS,
    )?;
    non_empty(
        "contextualizer.prompt_version",
        &contextualizer.prompt_version,
    )?;

    validate_hosted_adapters(
        "contextualizer",
        contextualizer.fireworks.as_ref(),
        contextualizer.openrouter.as_ref(),
        &config.reranker.fireworks,
        &config.reranker.openrouter,
    )
}

/// Checks one provider of one stage: egress permission, then its adapter table.
fn check_provider(
    stage: &str,
    field: &str,
    provider: &str,
    hosted: Option<HostedProvider>,
    presence: AdapterPresence,
    allowed: &[HostedProvider],
) -> Result<(), ConfigError> {
    if let Some(hosted) = hosted
        && !allowed.contains(&hosted)
    {
        return Err(ConfigError::Invalid {
            field: field.to_owned(),
            detail: format!(
                "hosted provider `{}` is not listed in `privacy.allowed_hosted_providers`",
                hosted.as_str()
            ),
        });
    }

    if presence == AdapterPresence::Missing {
        return Err(ConfigError::Invalid {
            field: field.to_owned(),
            detail: format!(
                "provider `{provider}` needs a `[{stage}.{}]` table",
                provider.replace('-', "_")
            ),
        });
    }

    Ok(())
}

/// Validates the adapter tables of one stage, including the credential variable
/// that two stages sharing a provider must agree on.
fn validate_hosted_adapters(
    stage: &str,
    fireworks: Option<&HostedAdapter>,
    openrouter: Option<&HostedAdapter>,
    other_fireworks: &Option<HostedAdapter>,
    other_openrouter: &Option<HostedAdapter>,
) -> Result<(), ConfigError> {
    for (provider, adapter, other) in [
        ("fireworks", fireworks, other_fireworks),
        ("openrouter", openrouter, other_openrouter),
    ] {
        let Some(adapter) = adapter else {
            continue;
        };

        let prefix = format!("{stage}.{provider}");
        require_https(&format!("{prefix}.base_url"), &adapter.base_url)?;
        require_path(&format!("{prefix}.endpoint"), &adapter.endpoint)?;
        non_empty(&format!("{prefix}.api_key_env"), &adapter.api_key_env)?;
        non_empty(&format!("{prefix}.model"), &adapter.model)?;

        if let Some(other) = other
            && other.api_key_env != adapter.api_key_env
        {
            return Err(ConfigError::Invalid {
                field: format!("{prefix}.api_key_env"),
                detail: format!(
                    "`{}` must be the same variable in every adapter table for this provider",
                    adapter.api_key_env
                ),
            });
        }
    }

    Ok(())
}
