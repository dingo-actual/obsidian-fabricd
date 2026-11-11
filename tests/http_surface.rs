//! The loopback HTTP surface, exercised over a real socket.
//!
//! Real sockets matter here: `Host` validation, the peer address, and the body
//! limit are transport-level controls, and an in-process router call would skip
//! them.

use std::net::SocketAddr;

use obsidian_fabricd::config::{self, Secrets};
use obsidian_fabricd::protocol::{
    HealthResponse, ProviderObservation, ServerInstanceId, ServerStatus,
};
use obsidian_fabricd::server::{self, AppState};
use tokio::net::TcpListener;
use uuid::Uuid;

const TOKEN: &str = "local-token-that-is-long-enough-to-matter";
const BASELINE: &str = include_str!("fixtures/config/baseline.toml");

struct Running {
    address: SocketAddr,
    client: reqwest::Client,
}

fn config(port: u16) -> config::Config {
    let mut config = config::parse(BASELINE).expect("the baseline fixture parses");
    config.server.port = port;

    config
}

fn secrets(fabric_key: bool) -> Secrets {
    config::resolve_with(&config(0), &|name| match name {
        "OBSIDIAN_FABRICD_TOKEN" => Some(TOKEN.to_owned()),
        "FABRIC_API_KEY" if fabric_key => Some("fabric-key-for-tests".to_owned()),
        _ => None,
    })
    .expect("the local token resolves")
}

/// Binds an ephemeral loopback port and serves the router on it.
async fn start(fabric_key: bool) -> Running {
    obsidian_fabricd::tls::install_crypto_provider();

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("an ephemeral loopback port");
    let address = listener.local_addr().expect("a local address");

    let state = AppState::new(
        &config(address.port()),
        &secrets(fabric_key),
        ServerInstanceId::from_uuid(Uuid::new_v4()),
    );

    tokio::spawn(async move {
        let _ = server::serve(listener, state).await;
    });

    Running {
        address,
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("a client"),
    }
}

impl Running {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("the request completes")
    }
}

fn header_value(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[tokio::test]
async fn health_requires_a_bearer_token() {
    let running = start(true).await;

    let response = running
        .client
        .get(running.url("/v1/health"))
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 401);
    assert_eq!(
        header_value(&response, "cache-control").as_deref(),
        Some("no-store")
    );
    assert_eq!(
        header_value(&response, "www-authenticate").as_deref(),
        Some("Bearer")
    );

    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert_eq!(body["code"], "unauthorized");
    assert_eq!(body["protocol_version"], 1);
}

#[tokio::test]
async fn a_wrong_token_is_refused() {
    let running = start(true).await;

    let response = running
        .client
        .get(running.url("/v1/health"))
        .bearer_auth("not-the-token")
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn a_malformed_or_oversized_token_is_refused() {
    let running = start(true).await;

    for header in [
        "Basic abc".to_owned(),
        format!("Bearer {}", "a".repeat(600)),
        "Bearer".to_owned(),
    ] {
        let response = running
            .client
            .get(running.url("/v1/health"))
            .header(reqwest::header::AUTHORIZATION, header)
            .send()
            .await
            .expect("the request completes");

        assert_eq!(response.status(), 401);
    }
}

#[tokio::test]
async fn a_token_in_the_query_string_is_not_a_credential() {
    let running = start(true).await;

    let response = running
        .client
        .get(running.url(&format!("/v1/health?token={TOKEN}")))
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 401);
    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert!(
        body["message"]
            .as_str()
            .expect("a message")
            .contains("query string"),
        "message was {}",
        body["message"]
    );
}

#[tokio::test]
async fn health_reports_degraded_when_fabric_is_configured_but_unobserved() {
    let running = start(true).await;

    let response = running.get("/v1/health").await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        header_value(&response, "cache-control").as_deref(),
        Some("no-store")
    );

    let health: HealthResponse = response.json().await.expect("the health payload");
    assert_eq!(health.protocol_version, 1);
    assert_eq!(health.supported_protocol_versions, vec![1]);
    assert_eq!(health.server_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(health.status, ServerStatus::Degraded);
    assert!(health.fabric.configured);
    assert_eq!(health.fabric.last_status, ProviderObservation::Unknown);
    assert_eq!(health.limits.request_bytes, 131_072);
    assert_eq!(health.limits.event_bytes, 262_144);
}

#[tokio::test]
async fn health_reports_not_ready_without_a_fabric_credential() {
    let running = start(false).await;

    let response = running.get("/v1/health").await;

    assert_eq!(response.status(), 200);
    let health: HealthResponse = response.json().await.expect("the health payload");
    assert_eq!(health.status, ServerStatus::NotReady);
    assert!(!health.fabric.configured);
    assert_eq!(
        health.fabric.last_status,
        ProviderObservation::Misconfigured
    );
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_with_the_canonical_error() {
    let running = start(true).await;
    let over_limit = "x".repeat(131_073);

    let response = running
        .client
        .get(running.url("/v1/health"))
        .bearer_auth(TOKEN)
        .body(over_limit)
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 413);
    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert_eq!(body["code"], "payload_too_large");
    assert_eq!(body["protocol_version"], 1);
    assert!(
        body["message"]
            .as_str()
            .expect("a message")
            .contains("131072")
    );
}

#[tokio::test]
async fn repeated_authentication_failures_are_limited() {
    let running = start(true).await;

    let mut statuses = Vec::new();
    for _ in 0..6 {
        let response = running
            .client
            .get(running.url("/v1/health"))
            .bearer_auth("wrong")
            .send()
            .await
            .expect("the request completes");
        statuses.push(response.status().as_u16());
    }

    assert_eq!(&statuses[..5], &[401, 401, 401, 401, 401]);
    assert_eq!(statuses[5], 429);

    let response = running.get("/v1/health").await;
    assert_eq!(
        response.status(),
        429,
        "a blocked peer stays blocked until the window ends"
    );
}

#[tokio::test]
async fn a_request_for_another_authority_is_refused_before_routing() {
    let running = start(true).await;

    let (status, body) = raw_request(
        running.address,
        "GET /v1/health HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert_eq!(status, 403, "body: {body}");
    assert!(body.contains("forbidden"), "body: {body}");
}

#[tokio::test]
async fn a_request_carrying_an_origin_is_refused_before_routing() {
    let running = start(true).await;
    let request = format!(
        "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: https://evil.example\r\n\
         Authorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n",
        running.address.port()
    );

    let (status, body) = raw_request(running.address, &request).await;

    assert_eq!(status, 403, "body: {body}");
    assert!(body.contains("origin"), "body: {body}");
}

#[tokio::test]
async fn a_request_with_the_right_authority_and_no_origin_is_served() {
    let running = start(true).await;
    let request = format!(
        "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\n\
         Connection: close\r\n\r\n",
        running.address.port()
    );

    let (status, body) = raw_request(running.address, &request).await;

    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"protocol_version\":1"));
}

#[tokio::test]
async fn the_bearer_token_is_never_echoed() {
    let running = start(true).await;

    let response = running
        .client
        .get(running.url("/v1/health?token=leaked"))
        .bearer_auth("wrong-token-value")
        .send()
        .await
        .expect("the request completes");

    let text = response.text().await.expect("the body");
    assert!(!text.contains("wrong-token-value"));
    assert!(!text.contains("leaked"));
}

#[tokio::test]
async fn an_unrouted_path_is_not_served_as_health() {
    let running = start(true).await;

    let response = running.get("/v1/unknown").await;

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn a_wrong_method_on_a_known_path_is_rejected() {
    let running = start(true).await;

    let response = running.get("/v1/sessions").await;

    assert_eq!(response.status(), 405);
}

/// Sends one HTTP/1.1 request by hand, so the client cannot rewrite the headers.
async fn raw_request(address: SocketAddr, request: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("a connection");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the request is written");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("the response is read");

    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status line");

    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();

    (status, body)
}
