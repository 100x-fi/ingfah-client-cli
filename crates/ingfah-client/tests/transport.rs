#![expect(clippy::expect_used, reason = "test helpers panic on broken fixtures")]

use std::time::Duration;

use ingfah_client::{ApiKey, Body, Client, ClientConfig, Error, Method, Request};
use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request as SeenRequest, ResponseTemplate};

const KEY: &str = "sk-ing-v1-0000000000000000000000000000000000000000000000000000000000000abc";

fn client(server: &MockServer) -> Client {
    let mut config = ClientConfig::new(server.uri(), ApiKey::new(KEY));
    config.retry_backoff = Duration::from_millis(1);
    Client::new(config).expect("client")
}

fn ok_json() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": []}))
}

#[tokio::test]
async fn get_is_retried_after_503_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/voices"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/voices"))
        .respond_with(ok_json())
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server).send(&Request::get("/client/voices")).await;

    assert!(matches!(resp.map(|r| r.body), Ok(Body::Json(_))));
}

#[tokio::test]
async fn get_gives_up_after_three_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(502))
        .expect(3)
        .mount(&server)
        .await;

    let resp = client(&server).send(&Request::get("/client/voices")).await;

    assert!(matches!(resp, Err(Error::Api(e)) if e.status == 502));
}

#[tokio::test]
async fn post_is_not_retried_on_503() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let req = Request::post("/client/outbound/batches/7/pause");
    let resp = client(&server).send(&req).await;

    assert!(matches!(resp, Err(Error::Api(e)) if e.status == 503));
}

#[tokio::test]
async fn get_is_not_retried_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            json!({"status": "error", "error": "resource not found", "error_detail": "agent not found"}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server)
        .send(&Request::get("/client/agents/x"))
        .await;

    assert!(matches!(resp, Err(Error::Api(e)) if e.message == "agent not found"));
}

#[tokio::test]
async fn success_status_with_error_envelope_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "error", "error": "bad request", "error_detail": "per_page too large"}),
        ))
        .mount(&server)
        .await;

    let resp = client(&server).send(&Request::get("/client/voices")).await;

    assert!(matches!(resp, Err(Error::Api(e)) if e.message == "per_page too large"));
}

#[tokio::test]
async fn sends_api_key_accept_and_user_agent_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("X-Api-Key", KEY))
        .and(header("Accept", "application/json"))
        .and(header_exists("User-Agent"))
        .respond_with(ok_json())
        .expect(1)
        .mount(&server)
        .await;

    let resp = client(&server).send(&Request::get("/client/voices")).await;

    assert!(resp.is_ok());
}

#[tokio::test]
async fn user_agent_names_the_cli() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let _ = client(&server).send(&Request::get("/client/voices")).await;

    let seen: Vec<SeenRequest> = server.received_requests().await.unwrap_or_default();
    let ua = seen[0]
        .headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok());
    assert!(ua.is_some_and(|ua| ua.starts_with("ingfah-cli/")));
}

#[tokio::test]
async fn never_sends_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok_json())
        .mount(&server)
        .await;

    let _ = client(&server).send(&Request::get("/client/voices")).await;

    let seen = server.received_requests().await.unwrap_or_default();
    assert!(seen[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn unknown_route_is_refused_without_a_request() {
    let server = MockServer::start().await;

    let resp = client(&server)
        .send(&Request::new(Method::Post, "/get-call-token"))
        .await;

    let seen = server.received_requests().await.unwrap_or_default();
    assert!(matches!(resp, Err(e) if e.is_refusal()) && seen.is_empty());
}

#[tokio::test]
async fn binary_body_is_kept_as_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![0x4f, 0x67, 0x67, 0x00], "audio/ogg"),
        )
        .mount(&server)
        .await;

    let resp = client(&server)
        .send(&Request::get("/client/chat-sessions/u-1/record/download"))
        .await;

    assert_eq!(
        resp.map(|r| r.body).ok(),
        Some(Body::Bytes(vec![0x4f, 0x67, 0x67, 0x00]))
    );
}
