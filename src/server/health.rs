//! The health route.
//!
//! The response reports cached observations and configuration readiness only. It
//! never contacts a provider, so a liveness check stays cheap and cannot trigger a
//! paid request. A live process answers `200` even when retrieval is not ready.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use crate::config::{Config, ContextualizerProvider, RerankerProvider, Secrets};
use crate::protocol::{
    ContextualizerHealth, ContextualizerMode, FabricHealth, HealthResponse, Limits,
    PROTOCOL_VERSION, ProviderObservation, RerankMode, RerankerHealth, ServerStatus,
};

use crate::server::AppState;

/// Cached provider observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderStatus {
    /// Retrieval service state.
    pub fabric: FabricHealth,
    /// Prefix stage state.
    pub contextualizer: ContextualizerHealth,
    /// Reranking stage state.
    pub reranker: RerankerHealth,
}

impl ProviderStatus {
    /// Derives observations from configuration and resolved secrets.
    ///
    /// Nothing has been called yet, so a configured stage is `unknown` and an
    /// unconfigured one is `disabled`. A missing Fabric credential is reported as
    /// `misconfigured`, which is what makes the service `not_ready`.
    pub fn from_config(config: &Config, secrets: &Secrets) -> Self {
        let fabric_configured = secrets.fabric_api_key.is_some();

        Self {
            fabric: FabricHealth {
                configured: fabric_configured,
                last_status: if fabric_configured {
                    ProviderObservation::Unknown
                } else {
                    ProviderObservation::Misconfigured
                },
            },
            contextualizer: ContextualizerHealth {
                mode: contextualizer_mode(config),
                last_status: match config.contextualizer.provider {
                    ContextualizerProvider::Disabled => ProviderObservation::Disabled,
                    _ => ProviderObservation::Unknown,
                },
            },
            reranker: RerankerHealth {
                mode: reranker_mode(config),
                last_status: match config.reranker.provider {
                    RerankerProvider::None => ProviderObservation::Disabled,
                    _ => ProviderObservation::Unknown,
                },
            },
        }
    }

    /// Overall readiness.
    ///
    /// Retrieval that cannot run at all is `not_ready`; a configured stage whose
    /// state is unproven is `degraded`; only a proven-good Fabric observation with
    /// no misconfigured stage is `ok`. A disabled optional stage never degrades.
    pub fn status(&self) -> ServerStatus {
        if !self.fabric.configured
            || matches!(
                self.fabric.last_status,
                ProviderObservation::Misconfigured | ProviderObservation::Unavailable
            )
        {
            return ServerStatus::NotReady;
        }

        let misconfigured = [self.contextualizer.last_status, self.reranker.last_status]
            .contains(&ProviderObservation::Misconfigured);
        if misconfigured {
            return ServerStatus::Degraded;
        }

        if self.fabric.last_status == ProviderObservation::Ok {
            return ServerStatus::Ok;
        }

        ServerStatus::Degraded
    }
}

/// Reports readiness without touching a provider.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let providers = state.providers;

    Json(HealthResponse {
        protocol_version: PROTOCOL_VERSION,
        supported_protocol_versions: vec![PROTOCOL_VERSION],
        server_version: env!("CARGO_PKG_VERSION").to_owned(),
        server_instance_id: state.instance_id,
        status: providers.status(),
        limits: Limits {
            request_bytes: state.limits.request_bytes,
            event_bytes: state.limits.event_bytes,
        },
        fabric: providers.fabric,
        contextualizer: providers.contextualizer,
        reranker: providers.reranker,
    })
}

fn contextualizer_mode(config: &Config) -> ContextualizerMode {
    match config.contextualizer.provider {
        ContextualizerProvider::Disabled => ContextualizerMode::Disabled,
        ContextualizerProvider::Fireworks => ContextualizerMode::Fireworks,
        ContextualizerProvider::OpenRouter => ContextualizerMode::OpenRouter,
    }
}

fn reranker_mode(config: &Config) -> RerankMode {
    match config.reranker.provider {
        RerankerProvider::None => RerankMode::None,
        RerankerProvider::LlamaSwap => RerankMode::LlamaSwap,
        RerankerProvider::Fireworks => RerankMode::Fireworks,
        RerankerProvider::OpenRouter => RerankMode::OpenRouter,
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderStatus;
    use crate::config::{Config, ContextualizerProvider, RerankerProvider, parse};
    use crate::protocol::{
        ContextualizerHealth, ContextualizerMode, FabricHealth, ProviderObservation, RerankMode,
        RerankerHealth, ServerStatus,
    };

    const BASELINE: &str = include_str!("../../tests/fixtures/config/baseline.toml");

    fn config() -> Config {
        parse(BASELINE).expect("the baseline fixture parses")
    }

    fn status(fabric: ProviderObservation, configured: bool) -> ProviderStatus {
        ProviderStatus {
            fabric: FabricHealth {
                configured,
                last_status: fabric,
            },
            contextualizer: ContextualizerHealth {
                mode: ContextualizerMode::Disabled,
                last_status: ProviderObservation::Disabled,
            },
            reranker: RerankerHealth {
                mode: RerankMode::None,
                last_status: ProviderObservation::Disabled,
            },
        }
    }

    #[test]
    fn a_missing_fabric_credential_is_not_ready() {
        let mut config = config();
        config.privacy.allowed_hosted_providers = vec![];
        let secrets = crate::config::resolve_with(&config, &|name| {
            (name == "OBSIDIAN_FABRICD_TOKEN").then(|| "local".to_owned())
        })
        .expect("the local token resolves");

        let providers = ProviderStatus::from_config(&config, &secrets);

        assert!(!providers.fabric.configured);
        assert_eq!(
            providers.fabric.last_status,
            ProviderObservation::Misconfigured
        );
        assert_eq!(providers.status(), ServerStatus::NotReady);
    }

    #[test]
    fn a_configured_but_unobserved_fabric_is_degraded() {
        let providers = status(ProviderObservation::Unknown, true);

        assert_eq!(providers.status(), ServerStatus::Degraded);
    }

    #[test]
    fn a_proven_fabric_is_ok() {
        assert_eq!(
            status(ProviderObservation::Ok, true).status(),
            ServerStatus::Ok
        );
    }

    #[test]
    fn an_unavailable_fabric_is_not_ready() {
        assert_eq!(
            status(ProviderObservation::Unavailable, true).status(),
            ServerStatus::NotReady
        );
    }

    #[test]
    fn a_misconfigured_optional_stage_only_degrades() {
        let mut providers = status(ProviderObservation::Ok, true);
        providers.reranker.last_status = ProviderObservation::Misconfigured;

        assert_eq!(providers.status(), ServerStatus::Degraded);
    }

    #[test]
    fn disabled_optional_stages_never_degrade() {
        let providers = status(ProviderObservation::Ok, true);

        assert_eq!(
            providers.contextualizer.last_status,
            ProviderObservation::Disabled
        );
        assert_eq!(providers.status(), ServerStatus::Ok);
    }

    #[test]
    fn configured_modes_are_reported_from_configuration() {
        let mut config = config();
        config.privacy.allowed_hosted_providers = vec![crate::config::HostedProvider::Fireworks];
        config.reranker.provider = RerankerProvider::Fireworks;
        config.reranker.fireworks = Some(crate::config::HostedAdapter {
            base_url: "https://api.fireworks.ai/inference/v1".to_owned(),
            endpoint: "/rerank".to_owned(),
            api_key_env: "FIREWORKS_API_KEY".to_owned(),
            model: "qwen3-reranker-8b".to_owned(),
        });
        config.contextualizer.enabled = true;
        config.contextualizer.provider = ContextualizerProvider::Fireworks;
        config.contextualizer.fireworks = Some(crate::config::HostedAdapter {
            base_url: "https://api.fireworks.ai/inference/v1".to_owned(),
            endpoint: "/chat/completions".to_owned(),
            api_key_env: "FIREWORKS_API_KEY".to_owned(),
            model: "deepseek-v4p1-flash".to_owned(),
        });
        let secrets = crate::config::resolve_with(&config, &|name| Some(name.to_owned()))
            .expect("the local token resolves");

        let providers = ProviderStatus::from_config(&config, &secrets);

        assert_eq!(providers.reranker.mode, RerankMode::Fireworks);
        assert_eq!(providers.contextualizer.mode, ContextualizerMode::Fireworks);
        assert_eq!(providers.reranker.last_status, ProviderObservation::Unknown);
        assert_eq!(
            providers.contextualizer.last_status,
            ProviderObservation::Unknown
        );
    }

    #[test]
    fn an_llama_swap_reranker_is_reported_by_its_wire_name() {
        let mut config = config();
        config.reranker.provider = RerankerProvider::LlamaSwap;
        config.reranker.llama_swap = Some(crate::config::LlamaSwapAdapter {
            base_url: "http://127.0.0.1:8080".to_owned(),
            endpoint: "/v1/rerank".to_owned(),
            model: "qwen3-reranker-8b".to_owned(),
            api_key_env: None,
        });
        let secrets = crate::config::resolve_with(&config, &|name| Some(name.to_owned()))
            .expect("the local token resolves");

        assert_eq!(
            ProviderStatus::from_config(&config, &secrets).reranker.mode,
            RerankMode::LlamaSwap
        );
    }
}
