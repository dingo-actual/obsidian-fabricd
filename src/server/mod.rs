//! The authenticated loopback HTTP surface.
//!
//! Scope: one route, `GET /v1/health`, behind the full set of local request
//! controls. Everything here is about making the door trustworthy:
//!
//! - [`guard`] checks the `Host` authority, refuses any browser `Origin`, and
//!   refuses a non-loopback peer when the address is available.
//! - [`auth`] accepts a bearer token only in the `Authorization` header, compares
//!   it in constant time, and limits repeated failures.
//! - [`limits`] caps request bodies before they are buffered.
//! - [`error`] is the single mapping from a failure to the wire error envelope.
//! - [`access`] logs method, path, status, and duration, never a header or a body.
//!
//! Every response carries `Cache-Control: no-store`, including failures produced
//! by the controls themselves.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tower_http::limit::RequestBodyLimitLayer;

use crate::config::Config;
use crate::config::Secrets;
use crate::protocol::{Limits, ServerInstanceId};
use crate::server::auth::{AuthState, authenticate};
use crate::server::guard::{Authority, GuardState, check_request};
use crate::server::health::{ProviderStatus, health};

pub mod access;
pub mod auth;
pub mod error;
pub mod guard;
pub mod health;
pub mod limits;

pub use error::ApiError;

/// Everything the HTTP surface needs, shared by every connection.
///
/// Deleting the derived `Debug` on purpose: this holds the bearer token and the
/// token digest, and a stray `{:?}` must not print either.
#[derive(Clone)]
pub struct AppState {
    /// Identity of this process lifetime.
    pub instance_id: ServerInstanceId,
    /// Declared byte limits, echoed by the health response.
    pub limits: Limits,
    /// Cached provider observations; health never probes a provider.
    pub providers: ProviderStatus,
    /// Bearer-token checking and failure limiting.
    pub auth: Arc<AuthState>,
    /// Accepted authority and origin policy.
    pub guard: Arc<GuardState>,
}

impl AppState {
    /// Builds the state from validated configuration, resolved secrets, and the
    /// process identity.
    pub fn new(config: &Config, secrets: &Secrets, instance_id: ServerInstanceId) -> Self {
        let limits = Limits {
            request_bytes: config.server.request_limit_bytes,
            event_bytes: config.server.event_limit_bytes,
        };

        Self {
            instance_id,
            limits,
            providers: ProviderStatus::from_config(config, secrets),
            auth: Arc::new(AuthState::new(&secrets.local_token)),
            guard: Arc::new(GuardState::new(Authority::new(
                &config.server.bind,
                config.server.port,
            ))),
        }
    }

    /// Replaces the credential checking, for tests that need a known token.
    pub fn with_token(mut self, token: &SecretString) -> Self {
        self.auth = Arc::new(AuthState::new(token));

        self
    }

    /// Replaces the cached provider observations, for tests and for later phases.
    pub fn with_providers(mut self, providers: ProviderStatus) -> Self {
        self.providers = providers;

        self
    }

    /// The largest accepted request body.
    pub fn request_limit_bytes(&self) -> u64 {
        self.limits.request_bytes
    }
}

/// Builds the router: local request controls outside, routes inside.
///
/// Layer order matters. The outermost layer normalises every response, so a
/// rejection from a control still carries the canonical error envelope and
/// `Cache-Control: no-store`.
pub fn router(state: AppState) -> Router {
    let limit = state.request_limit_bytes().min(usize::MAX as u64) as usize;

    Router::new()
        .route("/v1/health", get(health))
        .layer(RequestBodyLimitLayer::new(limit))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            authenticate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            check_request,
        ))
        .layer(axum::middleware::from_fn(access::log))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            error::normalise,
        ))
        .with_state(state)
}

/// Binds the configured loopback listener.
///
/// The port is never silently replaced: an occupied port is a startup failure the
/// operator must resolve.
///
/// # Errors
///
/// Returns [`ServerError::InvalidAddress`] when the configured bind address is not
/// a literal address, and [`ServerError::Bind`] when the socket cannot be bound.
pub async fn bind(config: &Config) -> Result<TcpListener, ServerError> {
    let address: SocketAddr = format!("{}:{}", config.server.bind, config.server.port)
        .parse()
        .map_err(|_| ServerError::InvalidAddress {
            bind: config.server.bind.clone(),
            port: config.server.port,
        })?;

    TcpListener::bind(address)
        .await
        .map_err(|source| ServerError::Bind { address, source })
}

/// Serves the router until the listener fails.
///
/// # Errors
///
/// Returns the underlying I/O failure when the listener stops accepting.
pub async fn serve(listener: TcpListener, app: Router) -> std::io::Result<()> {
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

/// Why the listener could not start.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The configured bind value is not a literal socket address.
    #[error("`{bind}:{port}` is not a literal socket address")]
    InvalidAddress {
        /// Configured bind address.
        bind: String,
        /// Configured port.
        port: u16,
    },
    /// The socket could not be bound, usually because the port is in use.
    #[error("cannot bind {address}")]
    Bind {
        /// The address that was tried.
        address: SocketAddr,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::{ServerError, bind};
    use crate::config::parse;

    const BASELINE: &str = include_str!("../../tests/fixtures/config/baseline.toml");

    #[tokio::test]
    async fn an_unparseable_bind_address_is_reported_without_binding() {
        let mut config = parse(BASELINE).expect("the fixture parses");
        config.server.bind = "127.0.0.1 ".to_owned();

        let error = bind(&config).await;

        assert!(matches!(error, Err(ServerError::InvalidAddress { .. })));
    }

    #[tokio::test]
    async fn an_occupied_port_is_a_startup_failure_not_a_new_port() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("an ephemeral port");
        let port = listener.local_addr().expect("an address").port();

        let mut config = parse(BASELINE).expect("the fixture parses");
        config.server.port = port;

        let error = bind(&config).await;

        assert!(matches!(error, Err(ServerError::Bind { address, .. }) if address.port() == port));
    }
}
