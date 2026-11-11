//! Bearer-token checking and authentication-failure limiting.
//!
//! The token is accepted in exactly one place: the `Authorization` header, as
//! `Bearer <token>`. A token in a query string, a cookie, or a body is not a
//! credential this service recognises. Comparison is constant time and works on
//! fixed-length digests, so neither the token nor its length leaks through timing.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::server::AppState;
use crate::server::error::ApiError;

/// Longest token a client may present. Anything longer is a wrong token and is
/// refused without hashing.
pub const MAX_TOKEN_BYTES: usize = 512;
/// Failed attempts allowed per peer inside one window.
pub const MAX_FAILURES: u32 = 5;
/// Length of the failure window.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(10);
/// Peers tracked at once; a loopback listener sees one, so this is generous.
const MAX_TRACKED_PEERS: usize = 128;

/// Query-string keys that clients sometimes use to smuggle a token.
const TOKEN_QUERY_KEYS: [&str; 5] = ["token", "access_token", "auth", "api_key", "key"];

/// The digest of the configured token, plus the failure limiter.
#[derive(Debug)]
pub struct AuthState {
    digest: [u8; 32],
    limiter: FailureLimiter,
}

impl AuthState {
    /// Hashes the configured token once, so a request never touches the secret.
    pub fn new(token: &SecretString) -> Self {
        Self {
            digest: digest(token.expose_secret().as_bytes()),
            limiter: FailureLimiter::new(MAX_FAILURES, FAILURE_WINDOW),
        }
    }

    /// Reports whether the presented token matches, in constant time.
    pub fn verify(&self, presented: &[u8]) -> bool {
        if presented.is_empty() || presented.len() > MAX_TOKEN_BYTES {
            return false;
        }

        digest(presented).ct_eq(&self.digest).into()
    }

    /// The failure limiter, exposed so the middleware and tests share one policy.
    pub fn limiter(&self) -> &FailureLimiter {
        &self.limiter
    }
}

fn digest(token: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token);
    hasher.finalize().into()
}

/// Counts failed attempts per peer in a fixed window.
#[derive(Debug)]
pub struct FailureLimiter {
    max_failures: u32,
    window: Duration,
    peers: Mutex<HashMap<IpAddr, Attempts>>,
}

#[derive(Clone, Copy, Debug)]
struct Attempts {
    count: u32,
    started: Instant,
}

impl FailureLimiter {
    /// Creates a limiter with an explicit policy, so tests need no sleeps.
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures,
            window,
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the remaining wait when this peer has spent its attempts.
    pub fn blocked_for(&self, peer: IpAddr, now: Instant) -> Option<Duration> {
        let peers = self.peers.lock().unwrap_or_else(|error| error.into_inner());
        let attempts = peers.get(&peer)?;

        if attempts.count < self.max_failures {
            return None;
        }
        let elapsed = now.saturating_duration_since(attempts.started);
        if elapsed >= self.window {
            return None;
        }

        Some(self.window - elapsed)
    }

    /// Records one failure, starting or refreshing the window.
    pub fn record_failure(&self, peer: IpAddr, now: Instant) {
        let mut peers = self.peers.lock().unwrap_or_else(|error| error.into_inner());

        if peers.len() >= MAX_TRACKED_PEERS && !peers.contains_key(&peer) {
            peers.clear();
        }

        let entry = peers.entry(peer).or_insert(Attempts {
            count: 0,
            started: now,
        });
        if now.saturating_duration_since(entry.started) >= self.window {
            *entry = Attempts {
                count: 0,
                started: now,
            };
        }
        entry.count = entry.count.saturating_add(1);
    }

    /// Clears this peer's failures after a success.
    pub fn record_success(&self, peer: IpAddr) {
        let mut peers = self.peers.lock().unwrap_or_else(|error| error.into_inner());
        peers.remove(&peer);
    }
}

/// Requires a valid bearer token on every request, including health and events.
pub async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let peer_ip = peer_of(&request);
    let now = Instant::now();

    if let Some(ip) = peer_ip
        && let Some(wait) = state.auth.limiter().blocked_for(ip, now)
    {
        return ApiError::capacity_limited(wait).into_response();
    }

    let refusal = match presented_token(request.headers(), request.uri().query()) {
        Ok(presented) if state.auth.verify(presented) => {
            if let Some(ip) = peer_ip {
                state.auth.limiter().record_success(ip);
            }
            return next.run(request).await;
        }
        Ok(_) => ApiError::unauthorized("the bearer token is not valid"),
        Err(reason) => ApiError::unauthorized(reason),
    };

    if let Some(ip) = peer_ip {
        state.auth.limiter().record_failure(ip, now);
    }

    refusal.into_response()
}

/// The peer address, when the transport reported one.
fn peer_of(request: &Request) -> Option<std::net::IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|connect| connect.0.ip())
}

/// Extracts the presented token, or explains why there is none to check.
fn presented_token<'a>(
    headers: &'a axum::http::HeaderMap,
    query: Option<&str>,
) -> Result<&'a [u8], &'static str> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let Some(value) = values.next() else {
        return Err(if carries_query_token(query) {
            "a token is never accepted in a query string; send Authorization: Bearer"
        } else {
            "the request has no authorization header"
        });
    };
    if values.next().is_some() {
        return Err("the request has more than one authorization header");
    }

    let bytes = value.as_bytes();
    let Some(space) = bytes.iter().position(|byte| *byte == b' ') else {
        return Err("the authorization header is not a bearer credential");
    };
    let (scheme, rest) = bytes.split_at(space);
    let token = &rest[1..];

    if !scheme.eq_ignore_ascii_case(b"bearer") || token.is_empty() {
        return Err("the authorization header is not a bearer credential");
    }
    if token.len() > MAX_TOKEN_BYTES {
        return Err("the presented token is longer than any valid token");
    }

    Ok(token)
}

fn carries_query_token(query: Option<&str>) -> bool {
    query.is_some_and(|query| {
        query.split('&').any(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            TOKEN_QUERY_KEYS
                .iter()
                .any(|candidate| key.eq_ignore_ascii_case(candidate))
        })
    })
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, Instant};

    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use secrecy::SecretString;

    use super::{
        AuthState, FAILURE_WINDOW, FailureLimiter, MAX_FAILURES, MAX_TOKEN_BYTES, presented_token,
    };

    fn peer() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    }

    fn headers_with(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn a_matching_token_is_accepted() {
        let state = AuthState::new(&SecretString::from("token-value-for-tests"));

        assert!(state.verify(b"token-value-for-tests"));
    }

    #[test]
    fn a_wrong_prefix_boundary_or_length_is_rejected() {
        let state = AuthState::new(&SecretString::from("token-value-for-tests"));

        assert!(!state.verify(b"token-value-for-test"));
        assert!(!state.verify(b"token-value-for-tests "));
        assert!(!state.verify(b""));
        assert!(!state.verify(b"TOKEN-VALUE-FOR-TESTS"));
    }

    #[test]
    fn an_over_long_token_is_rejected_without_hashing() {
        let state = AuthState::new(&SecretString::from("token-value-for-tests"));
        let long = vec![b'a'; MAX_TOKEN_BYTES + 1];

        assert!(!state.verify(&long));
    }

    #[test]
    fn the_header_must_be_a_bearer_credential() {
        assert_eq!(
            presented_token(&headers_with("Bearer abc"), None),
            Ok(&b"abc"[..])
        );
        assert_eq!(
            presented_token(&headers_with("bearer abc"), None),
            Ok(&b"abc"[..])
        );
        assert!(presented_token(&headers_with("Basic abc"), None).is_err());
        assert!(presented_token(&headers_with("Bearer"), None).is_err());
        assert!(presented_token(&headers_with("Bearer "), None).is_err());
    }

    #[test]
    fn a_missing_header_is_refused_and_a_query_token_is_named() {
        let empty = HeaderMap::new();

        assert_eq!(
            presented_token(&empty, None),
            Err("the request has no authorization header")
        );
        assert_eq!(
            presented_token(&empty, Some("token=abc&other=1")),
            Err("a token is never accepted in a query string; send Authorization: Bearer")
        );
        assert_eq!(
            presented_token(&empty, Some("Access_Token=abc")),
            Err("a token is never accepted in a query string; send Authorization: Bearer")
        );
        assert_eq!(
            presented_token(&empty, Some("page=2")),
            Err("the request has no authorization header")
        );
    }

    #[test]
    fn two_authorization_headers_are_refused() {
        let mut headers = HeaderMap::new();
        headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer a"));
        headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer b"));

        assert_eq!(
            presented_token(&headers, None),
            Err("the request has more than one authorization header")
        );
    }

    #[test]
    fn failures_accumulate_until_the_limit_and_then_block() {
        let limiter = FailureLimiter::new(MAX_FAILURES, FAILURE_WINDOW);
        let start = Instant::now();

        for attempt in 0..MAX_FAILURES {
            assert!(limiter.blocked_for(peer(), start).is_none());
            limiter.record_failure(peer(), start);
            assert_eq!(attempt + 1, MAX_FAILURES.min(attempt + 1));
        }

        let wait = limiter
            .blocked_for(peer(), start)
            .expect("the peer is blocked");

        assert_eq!(wait, FAILURE_WINDOW);
    }

    #[test]
    fn the_window_expires_without_sleeping() {
        let limiter = FailureLimiter::new(1, FAILURE_WINDOW);
        let start = Instant::now();
        limiter.record_failure(peer(), start);

        assert!(limiter.blocked_for(peer(), start).is_some());
        assert!(
            limiter
                .blocked_for(peer(), start + FAILURE_WINDOW)
                .is_none()
        );
    }

    #[test]
    fn a_success_clears_the_count() {
        let limiter = FailureLimiter::new(1, FAILURE_WINDOW);
        let start = Instant::now();
        limiter.record_failure(peer(), start);
        limiter.record_success(peer());

        assert!(limiter.blocked_for(peer(), start).is_none());
    }

    #[test]
    fn peers_are_counted_separately() {
        let limiter = FailureLimiter::new(1, FAILURE_WINDOW);
        let start = Instant::now();
        limiter.record_failure(peer(), start);

        assert!(
            limiter
                .blocked_for(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), start)
                .is_none()
        );
    }

    #[test]
    fn a_partially_elapsed_window_reports_the_remainder() {
        let limiter = FailureLimiter::new(1, Duration::from_secs(10));
        let start = Instant::now();
        limiter.record_failure(peer(), start);

        assert_eq!(
            limiter.blocked_for(peer(), start + Duration::from_secs(4)),
            Some(Duration::from_secs(6))
        );
    }
}
