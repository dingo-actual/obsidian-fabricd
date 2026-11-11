//! Reranker and contextualizer sections plus the adapter tables they select.
//!
//! Adapter tables carry no defaults: a selected provider must name its endpoint,
//! model, and credential variable, so a half-written provider section fails at
//! parse time rather than at the first request.

use serde::{Deserialize, Serialize};

/// A provider that sends data off the machine, so it needs explicit egress permission.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostedProvider {
    /// Fireworks AI.
    Fireworks,
    /// OpenRouter.
    OpenRouter,
}

impl HostedProvider {
    /// The configuration spelling of this provider.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fireworks => "fireworks",
            Self::OpenRouter => "openrouter",
        }
    }
}

/// A reranking provider, in chain order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RerankerProvider {
    /// Identity ordering with no inference and no scores.
    #[serde(rename = "none")]
    None,
    /// A separately managed llama-swap server on loopback.
    #[serde(rename = "llama-swap")]
    LlamaSwap,
    /// Fireworks AI.
    #[serde(rename = "fireworks")]
    Fireworks,
    /// OpenRouter.
    #[serde(rename = "openrouter")]
    OpenRouter,
}

impl RerankerProvider {
    /// The configuration spelling of this provider.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LlamaSwap => "llama-swap",
            Self::Fireworks => "fireworks",
            Self::OpenRouter => "openrouter",
        }
    }

    /// The egress-permission identity, or `None` when the provider runs locally.
    pub fn hosted(self) -> Option<HostedProvider> {
        match self {
            Self::None | Self::LlamaSwap => None,
            Self::Fireworks => Some(HostedProvider::Fireworks),
            Self::OpenRouter => Some(HostedProvider::OpenRouter),
        }
    }
}

/// The contextual-prefix provider, in chain order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextualizerProvider {
    /// The stage is not available.
    Disabled,
    /// Fireworks AI.
    Fireworks,
    /// OpenRouter.
    OpenRouter,
}

impl ContextualizerProvider {
    /// The configuration spelling of this provider.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Fireworks => "fireworks",
            Self::OpenRouter => "openrouter",
        }
    }

    /// The egress-permission identity, or `None` when the stage is disabled.
    pub fn hosted(self) -> Option<HostedProvider> {
        match self {
            Self::Disabled => None,
            Self::Fireworks => Some(HostedProvider::Fireworks),
            Self::OpenRouter => Some(HostedProvider::OpenRouter),
        }
    }
}

/// A llama-swap adapter table (`[reranker.llama_swap]`).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LlamaSwapAdapter {
    /// Loopback origin of the operator-managed server.
    pub base_url: String,
    /// Rerank path below the base URL.
    pub endpoint: String,
    /// Model alias configured in llama-swap.
    pub model: String,
    /// Credential variable, omitted when the server has authentication disabled.
    pub api_key_env: Option<String>,
}

/// A hosted adapter table (`[reranker.fireworks]`, `[contextualizer.openrouter]`, …).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostedAdapter {
    /// Provider origin, which must be HTTPS.
    pub base_url: String,
    /// Endpoint path below the base URL.
    pub endpoint: String,
    /// Credential variable for this destination.
    pub api_key_env: String,
    /// Provider model identifier.
    pub model: String,
}

/// `[reranker]` — the chain that orders the candidate pool.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reranker {
    /// The configured primary provider.
    pub provider: RerankerProvider,
    /// Providers that may be tried, in order, when the primary fails.
    pub fallback: Vec<RerankerProvider>,
    /// Request the whole bounded pool instead of only the displayed count.
    pub request_full_pool: bool,
    /// llama-swap adapter table.
    pub llama_swap: Option<LlamaSwapAdapter>,
    /// Fireworks adapter table.
    pub fireworks: Option<HostedAdapter>,
    /// OpenRouter adapter table.
    pub openrouter: Option<HostedAdapter>,
}

impl Default for Reranker {
    fn default() -> Self {
        Self {
            provider: RerankerProvider::None,
            fallback: Vec::new(),
            request_full_pool: true,
            llama_swap: None,
            fireworks: None,
            openrouter: None,
        }
    }
}

impl Reranker {
    /// The adapter table for a provider, if the operator configured one.
    pub fn adapter_for(&self, provider: RerankerProvider) -> AdapterPresence {
        let present = match provider {
            RerankerProvider::None => return AdapterPresence::NotRequired,
            RerankerProvider::LlamaSwap => self.llama_swap.is_some(),
            RerankerProvider::Fireworks => self.fireworks.is_some(),
            RerankerProvider::OpenRouter => self.openrouter.is_some(),
        };

        if present {
            AdapterPresence::Configured
        } else {
            AdapterPresence::Missing
        }
    }
}

/// `[contextualizer]` — the optional retrieval-prefix stage.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Contextualizer {
    /// Whether the stage may run at all.
    pub enabled: bool,
    /// The configured primary provider.
    pub provider: ContextualizerProvider,
    /// Providers that may be tried, in order, when the primary fails.
    pub fallback: Vec<ContextualizerProvider>,
    /// Output budget for one prefix completion, including reasoning tokens.
    ///
    /// Hosted chat completions are always streamed, so the budget may exceed a
    /// provider's non-streaming ceiling.
    pub max_output_tokens: u32,
    /// Application-owned instruction template version.
    pub prompt_version: String,
    /// Persist accepted prefixes in the cache.
    pub persist_prefixes: bool,
    /// Fireworks adapter table.
    pub fireworks: Option<HostedAdapter>,
    /// OpenRouter adapter table.
    pub openrouter: Option<HostedAdapter>,
}

impl Default for Contextualizer {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: ContextualizerProvider::Disabled,
            fallback: Vec::new(),
            max_output_tokens: 4096,
            prompt_version: "context-prefix-1".to_owned(),
            persist_prefixes: false,
            fireworks: None,
            openrouter: None,
        }
    }
}

impl Contextualizer {
    /// The adapter table for a provider, if the operator configured one.
    pub fn adapter_for(&self, provider: ContextualizerProvider) -> AdapterPresence {
        let present = match provider {
            ContextualizerProvider::Disabled => return AdapterPresence::NotRequired,
            ContextualizerProvider::Fireworks => self.fireworks.is_some(),
            ContextualizerProvider::OpenRouter => self.openrouter.is_some(),
        };

        if present {
            AdapterPresence::Configured
        } else {
            AdapterPresence::Missing
        }
    }
}

/// Whether a provider's adapter table is present, and whether it has to be.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterPresence {
    /// The provider needs no adapter table.
    NotRequired,
    /// The operator supplied the table.
    Configured,
    /// The provider was selected without a table; validation rejects this.
    Missing,
}

#[cfg(test)]
mod tests {
    use super::{AdapterPresence, Contextualizer, Reranker, RerankerProvider};

    #[test]
    fn defaults_disable_reranking_and_contextualisation() {
        let reranker = Reranker::default();
        let contextualizer = Contextualizer::default();

        assert_eq!(reranker.provider, RerankerProvider::None);
        assert!(reranker.fallback.is_empty());
        assert!(reranker.request_full_pool);
        assert!(!contextualizer.enabled);
        assert_eq!(contextualizer.max_output_tokens, 4096);
        assert!(!contextualizer.persist_prefixes);
    }

    #[test]
    fn a_selected_provider_without_an_adapter_table_is_missing() {
        let reranker = Reranker::default();

        assert_eq!(
            reranker.adapter_for(RerankerProvider::None),
            AdapterPresence::NotRequired
        );
        assert_eq!(
            reranker.adapter_for(RerankerProvider::LlamaSwap),
            AdapterPresence::Missing
        );
    }

    #[test]
    fn every_provider_has_a_configuration_spelling() {
        for provider in [
            RerankerProvider::None,
            RerankerProvider::LlamaSwap,
            RerankerProvider::Fireworks,
            RerankerProvider::OpenRouter,
        ] {
            assert!(!provider.as_str().is_empty());
        }
        assert_eq!(RerankerProvider::LlamaSwap.as_str(), "llama-swap");
        assert_eq!(
            RerankerProvider::OpenRouter.hosted(),
            Some(super::HostedProvider::OpenRouter)
        );
        assert_eq!(RerankerProvider::LlamaSwap.hosted(), None);
    }
}
