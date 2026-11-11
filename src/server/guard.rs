//! Local request controls applied before routing.
//!
//! Three checks, in order: the `Host` authority must be the listener this process
//! bound, a browser `Origin` is refused unless it was explicitly approved (none is
//! by default), and the peer must be loopback when its address is known.
//!
//! The desktop transport sends no `Origin`, so the absent-origin path is the normal
//! one. Requiring `Host` costs nothing and blocks a request addressed to another
//! name that happens to reach this port.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{HOST, ORIGIN};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;
use crate::server::error::ApiError;

/// Accepted `Host` spellings for the configured listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authority {
    accepted: Vec<String>,
}

impl Authority {
    /// Builds the accepted spellings for a loopback bind address and port.
    pub fn new(bind: &str, port: u16) -> Self {
        let host = if bind.contains(':') && !bind.starts_with('[') {
            // An IPv6 literal is written with brackets in an authority.
            format!("[{bind}]")
        } else {
            bind.to_owned()
        };

        Self {
            accepted: vec![format!("{host}:{port}")],
        }
    }

    /// Reports whether a `Host` value addresses this listener.
    pub fn accepts(&self, value: &str) -> bool {
        self.accepted
            .iter()
            .any(|accepted| accepted.eq_ignore_ascii_case(value))
    }
}

/// The authority policy plus any explicitly approved origins.
#[derive(Clone, Debug, Default)]
pub struct GuardState {
    /// Accepted `Host` values.
    pub authority: Authority,
    /// Origins that may call this service. Empty: the desktop transport sends none.
    pub approved_origins: Vec<String>,
}

impl GuardState {
    /// Builds the guard for one listener.
    pub fn new(authority: Authority) -> Self {
        Self {
            authority,
            approved_origins: Vec::new(),
        }
    }

    /// Applies every local control. `peer` is absent when the transport does not
    /// report a peer address.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::forbidden`] when the request is not addressed to this
    /// listener, carries an unapproved origin, or arrives from a non-loopback peer.
    pub fn check(
        &self,
        headers: &axum::http::HeaderMap,
        peer: Option<IpAddr>,
    ) -> Result<(), ApiError> {
        let host = headers
            .get(HOST)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(missing_host)?;

        if !self.authority.accepts(host) {
            return Err(missing_host());
        }

        if let Some(origin) = headers.get(ORIGIN) {
            let origin = origin.to_str().unwrap_or_default();
            if !self
                .approved_origins
                .iter()
                .any(|approved| approved.eq_ignore_ascii_case(origin))
            {
                return Err(ApiError::forbidden(
                    "the request carried an origin this connector does not serve",
                ));
            }
        }

        if let Some(peer) = peer
            && !peer.is_loopback()
        {
            return Err(ApiError::forbidden(
                "the request did not arrive from a loopback address",
            ));
        }

        Ok(())
    }
}

impl Default for Authority {
    fn default() -> Self {
        Self::new("127.0.0.1", 47_322)
    }
}

fn missing_host() -> ApiError {
    ApiError::forbidden("the request did not address the configured loopback listener")
}

/// Applies the local controls before any route sees the request.
pub async fn check_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip());

    if let Err(error) = state.guard.check(request.headers(), peer_ip) {
        return error.into_response();
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use axum::http::{HeaderMap, HeaderValue, header::HOST};

    use super::{Authority, GuardState};

    fn guard(bind: &str, port: u16) -> GuardState {
        GuardState::new(Authority::new(bind, port))
    }

    fn headers(host: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static(host));

        headers
    }

    #[test]
    fn an_exact_authority_is_accepted() {
        let guard = guard("127.0.0.1", 47_322);

        assert!(guard.check(&headers("127.0.0.1:47322"), None).is_ok());
        assert!(
            guard
                .check(
                    &headers("127.0.0.1:47322"),
                    Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
                )
                .is_ok()
        );
    }

    #[test]
    fn the_authority_comparison_is_case_insensitive() {
        let guard = GuardState::new(Authority::new("::1", 47_322));

        assert!(guard.check(&headers("[::1]:47322"), None).is_ok());
        assert!(!Authority::new("127.0.0.1", 47_322).accepts("127.0.0.1"));
    }

    #[test]
    fn a_wrong_host_or_port_is_refused() {
        let guard = guard("127.0.0.1", 47_322);

        for host in [
            "127.0.0.1:47323",
            "localhost:47322",
            "evil.example:47322",
            "127.0.0.2:47322",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                HOST,
                HeaderValue::from_bytes(host.as_bytes()).expect("valid"),
            );

            assert!(
                guard.check(&headers, None).is_err(),
                "host {host} was accepted"
            );
        }
    }

    #[test]
    fn a_missing_host_is_refused() {
        let guard = guard("127.0.0.1", 47_322);

        assert!(guard.check(&HeaderMap::new(), None).is_err());
    }

    #[test]
    fn an_absent_origin_is_the_normal_desktop_case() {
        let guard = guard("127.0.0.1", 47_322);

        assert!(guard.check(&headers("127.0.0.1:47322"), None).is_ok());
    }

    #[test]
    fn a_present_origin_is_refused_by_default() {
        let guard = guard("127.0.0.1", 47_322);
        let mut headers = headers("127.0.0.1:47322");
        headers.insert(ORIGIN, HeaderValue::from_static("https://evil.example"));

        assert!(guard.check(&headers, None).is_err());
    }

    #[test]
    fn an_explicitly_approved_origin_is_accepted() {
        let mut state = guard("127.0.0.1", 47_322);
        state.approved_origins = vec!["app://obsidian".to_owned()];
        let mut headers = headers("127.0.0.1:47322");
        headers.insert(ORIGIN, HeaderValue::from_static("app://obsidian"));

        assert!(state.check(&headers, None).is_ok());
    }

    #[test]
    fn a_non_loopback_peer_is_refused() {
        let guard = guard("127.0.0.1", 47_322);

        assert!(
            guard
                .check(
                    &headers("127.0.0.1:47322"),
                    Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
                )
                .is_ok()
        );
        assert!(
            guard
                .check(
                    &headers("127.0.0.1:47322"),
                    Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)))
                )
                .is_err()
        );
    }

    const ORIGIN: axum::http::header::HeaderName = axum::http::header::ORIGIN;
}
