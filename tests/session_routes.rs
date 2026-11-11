//! Session creation and deletion over the real HTTP surface.
//!
//! These assert the guarantees a plugin depends on: a repeat is safe, a rebind is
//! refused, the cap refuses instead of evicting, and two windows never interfere.

use std::net::SocketAddr;

use obsidian_fabricd::config::{self, Secrets};
use obsidian_fabricd::protocol::{ServerInstanceId, SessionCreateResponse};
use obsidian_fabricd::server::{self, AppState};
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

const TOKEN: &str = "local-token-that-is-long-enough-to-matter";
const BASELINE: &str = include_str!("fixtures/config/baseline.toml");

struct Running {
    address: SocketAddr,
    client: reqwest::Client,
}

fn config(port: u16, max_sessions: usize) -> config::Config {
    let mut config = config::parse(BASELINE).expect("the baseline fixture parses");
    config.server.port = port;
    config.server.max_sessions = max_sessions;

    config
}

fn secrets() -> Secrets {
    config::resolve_with(&config(0, 16), &|name| match name {
        "OBSIDIAN_FABRICD_TOKEN" => Some(TOKEN.to_owned()),
        "FABRIC_API_KEY" => Some("fabric-key-for-tests".to_owned()),
        _ => None,
    })
    .expect("the local token resolves")
}

async fn start_with(max_sessions: usize) -> Running {
    obsidian_fabricd::tls::install_crypto_provider();

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("an ephemeral loopback port");
    let address = listener.local_addr().expect("a local address");
    let state = AppState::new(
        &config(address.port(), max_sessions),
        &secrets(),
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

async fn start() -> Running {
    start_with(16).await
}

impl Running {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    async fn create_session(&self, session_id: Uuid, vault_id: Uuid) -> reqwest::Response {
        self.client
            .post(self.url("/v1/sessions"))
            .bearer_auth(TOKEN)
            .json(&json!({
                "protocol_version": 1,
                "session_id": session_id,
                "vault_id": vault_id,
            }))
            .send()
            .await
            .expect("the request completes")
    }

    async fn delete_session(&self, session_id: Uuid) -> reqwest::Response {
        self.client
            .delete(self.url(&format!("/v1/sessions/{session_id}")))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("the request completes")
    }
}

#[tokio::test]
async fn a_new_session_is_created_with_generation_zero() {
    let running = start().await;
    let (session_id, vault_id) = (Uuid::new_v4(), Uuid::new_v4());

    let response = running.create_session(session_id, vault_id).await;

    assert_eq!(response.status(), 201);
    let body: SessionCreateResponse = response.json().await.expect("the session payload");
    assert_eq!(body.protocol_version, 1);
    assert_eq!(body.session_id.to_string(), session_id.to_string());
    assert_eq!(body.vault_id.to_string(), vault_id.to_string());
    assert_eq!(body.latest_generation.get(), 0);
    assert_eq!(body.idle_timeout_seconds, 900);
}

#[tokio::test]
async fn an_identical_retry_returns_200_with_the_same_instance() {
    let running = start().await;
    let (session_id, vault_id) = (Uuid::new_v4(), Uuid::new_v4());

    let first: SessionCreateResponse = running
        .create_session(session_id, vault_id)
        .await
        .json()
        .await
        .expect("the first payload");
    let response = running.create_session(session_id, vault_id).await;

    assert_eq!(response.status(), 200);
    let second: SessionCreateResponse = response.json().await.expect("the second payload");
    assert_eq!(second.server_instance_id, first.server_instance_id);
    assert_eq!(second.latest_generation, first.latest_generation);
}

#[tokio::test]
async fn rebinding_a_session_to_another_vault_is_a_conflict() {
    let running = start().await;
    let session_id = Uuid::new_v4();

    let first = running.create_session(session_id, Uuid::new_v4()).await;
    assert_eq!(first.status(), 201);
    let response = running.create_session(session_id, Uuid::new_v4()).await;

    assert_eq!(response.status(), 409);
    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert_eq!(body["code"], "session_conflict");
    assert_eq!(body["protocol_version"], 1);
}

#[tokio::test]
async fn two_sessions_on_one_vault_are_independent() {
    let running = start().await;
    let vault_id = Uuid::new_v4();
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());

    assert_eq!(running.create_session(first, vault_id).await.status(), 201);
    assert_eq!(running.create_session(second, vault_id).await.status(), 201);

    assert_eq!(running.delete_session(first).await.status(), 204);

    let response = running.create_session(second, vault_id).await;
    assert_eq!(response.status(), 200, "the other window is untouched");
}

#[tokio::test]
async fn deleting_a_session_twice_reports_it_as_gone() {
    let running = start().await;
    let session_id = Uuid::new_v4();
    assert_eq!(
        running
            .create_session(session_id, Uuid::new_v4())
            .await
            .status(),
        201
    );

    assert_eq!(running.delete_session(session_id).await.status(), 204);

    let response = running.delete_session(session_id).await;
    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert_eq!(body["code"], "session_not_found");
}

#[tokio::test]
async fn the_seventeenth_session_is_refused_and_none_is_dropped() {
    let running = start_with(16).await;
    let vault_id = Uuid::new_v4();
    let mut admitted = Vec::new();

    for _ in 0..16 {
        let session_id = Uuid::new_v4();
        let response = running.create_session(session_id, vault_id).await;
        assert_eq!(response.status(), 201);
        admitted.push(session_id);
    }

    let response = running.create_session(Uuid::new_v4(), vault_id).await;
    assert_eq!(response.status(), 429);
    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert_eq!(body["code"], "capacity_limited");
    assert!(body["retry_after_ms"].as_u64().expect("a retry delay") > 0);

    for session_id in admitted {
        let response = running.create_session(session_id, vault_id).await;
        assert_eq!(response.status(), 200, "a live session must still be there");
    }
}

#[tokio::test]
async fn an_unsupported_protocol_version_names_the_supported_ones() {
    let running = start().await;

    let response = running
        .client
        .post(running.url("/v1/sessions"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "protocol_version": 2,
            "session_id": Uuid::new_v4(),
            "vault_id": Uuid::new_v4(),
        }))
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 409);
    let body: serde_json::Value = response.json().await.expect("a JSON error");
    assert_eq!(body["code"], "protocol_version_mismatch");
    assert_eq!(body["supported_protocol_versions"], json!([1]));
}

#[tokio::test]
async fn a_malformed_session_request_is_refused() {
    let running = start().await;

    for body in [
        json!({"protocol_version": 1, "session_id": "not-a-uuid", "vault_id": Uuid::new_v4()}),
        json!({"protocol_version": 1}),
    ] {
        let response = running
            .client
            .post(running.url("/v1/sessions"))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .expect("the request completes");

        assert_eq!(response.status(), 400, "body {body}");
        let payload: serde_json::Value = response.json().await.expect("a JSON error");
        assert_eq!(payload["code"], "invalid_request");
    }
}

#[tokio::test]
async fn a_duplicate_key_in_a_session_request_is_refused() {
    let running = start().await;
    let session_id = Uuid::new_v4();
    let raw = format!(
        r#"{{"protocol_version":1,"session_id":"{session_id}","vault_id":"{}","vault_id":"{}"}}"#,
        Uuid::new_v4(),
        Uuid::new_v4()
    );

    let response = running
        .client
        .post(running.url("/v1/sessions"))
        .bearer_auth(TOKEN)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(raw)
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn an_unknown_additive_field_is_tolerated() {
    let running = start().await;
    let (session_id, vault_id) = (Uuid::new_v4(), Uuid::new_v4());

    let response = running
        .client
        .post(running.url("/v1/sessions"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "protocol_version": 1,
            "session_id": session_id,
            "vault_id": vault_id,
            "future_field": "ignored",
        }))
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 201);
}

#[tokio::test]
async fn a_session_route_still_requires_authentication() {
    let running = start().await;

    let create = running
        .client
        .post(running.url("/v1/sessions"))
        .json(&json!({
            "protocol_version": 1,
            "session_id": Uuid::new_v4(),
            "vault_id": Uuid::new_v4(),
        }))
        .send()
        .await
        .expect("the request completes");
    let delete = running
        .client
        .delete(running.url(&format!("/v1/sessions/{}", Uuid::new_v4())))
        .send()
        .await
        .expect("the request completes");

    assert_eq!(create.status(), 401);
    assert_eq!(delete.status(), 401);
}

#[tokio::test]
async fn a_session_id_that_is_not_a_uuid_is_a_bad_request() {
    let running = start().await;

    let response = running
        .client
        .delete(running.url("/v1/sessions/not-a-uuid"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the request completes");

    assert_eq!(response.status(), 400);
}
