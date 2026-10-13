mod common;

use common::{ingfah, ingfah_without_key, request_count};
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread")]
async fn write_without_yes_on_non_tty_exits_3_and_sends_nothing() {
    let server = MockServer::start().await;

    ingfah(&server)
        .args(["api", "POST", "/client/outbound/batches/7/pause"])
        .assert()
        .code(3)
        .stderr(contains("--yes"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn write_with_yes_is_sent_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/client/outbound/batches/7/pause"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "success", "data": {"id": 7, "status": "paused"}})),
        )
        .expect(1)
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["--yes", "api", "POST", "/client/outbound/batches/7/pause"])
        .assert()
        .success()
        .stdout(contains("paused"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_prints_the_request_and_sends_nothing() {
    let server = MockServer::start().await;

    ingfah(&server)
        .args([
            "--dry-run",
            "api",
            "POST",
            "/client/products",
            "--body",
            r#"{"name":"Support"}"#,
        ])
        .assert()
        .success()
        .stdout(contains("\"dry_run\": true").and(contains("Support")));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_route_exits_3_without_a_request() {
    let server = MockServer::start().await;

    ingfah(&server)
        .args(["--yes", "api", "POST", "/get-call-token"])
        .assert()
        .code(3)
        .stderr(contains("not an allowed API-key route"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn jwt_only_route_exits_3_without_a_request() {
    let server = MockServer::start().await;

    ingfah(&server)
        .args(["api", "GET", "/client/documents"])
        .assert()
        .code(3);

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn out_refuses_to_overwrite_an_existing_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("a,b\n", "text/csv"))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("calls.csv");
    std::fs::write(&file, "keep me").expect("seed file");

    ingfah(&server)
        .args(["api", "GET", "/client/chat-sessions-list/csv", "--out"])
        .arg(&file)
        .assert()
        .code(3)
        .stderr(contains("refusing to overwrite"));

    assert_eq!(
        std::fs::read_to_string(&file).ok().as_deref(),
        Some("keep me")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn out_writes_a_new_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("a,b\n", "text/csv"))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("calls.csv");

    ingfah(&server)
        .args(["api", "GET", "/client/chat-sessions-list/csv", "--out"])
        .arg(&file)
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(&file).ok().as_deref(),
        Some("a,b\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn success_status_with_error_envelope_exits_1_with_error_detail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "error", "error": "bad request", "error_detail": "from and to are required"}),
        ))
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["api", "GET", "/client/analytics/summary"])
        .assert()
        .code(1)
        .stderr(contains("from and to are required"));
}

#[tokio::test(flavor = "multi_thread")]
async fn error_data_is_shown() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "status": "error", "error": "conflict", "error_detail": "tool in use",
            "data": {"agents": [{"slug": "mali"}]}
        })))
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["--yes", "api", "DELETE", "/client/plugin-functions/3"])
        .assert()
        .code(1)
        .stderr(contains("tool in use").and(contains("mali")));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_key_fails_before_any_request() {
    let server = MockServer::start().await;

    ingfah_without_key(&server)
        .args(["api", "GET", "/client/voices"])
        .assert()
        .code(1)
        .stderr(contains("ingfah auth login").and(contains("INGFAH_API_KEY")));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn env_key_is_used_when_no_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(wiremock::matchers::header("X-Api-Key", "from-env"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": []})),
        )
        .expect(1)
        .mount(&server)
        .await;

    ingfah_without_key(&server)
        .env("INGFAH_API_KEY", "from-env")
        .args(["api", "GET", "/client/voices"])
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread")]
async fn key_flag_wins_over_env() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(wiremock::matchers::header("X-Api-Key", common::KEY))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": []})),
        )
        .expect(1)
        .mount(&server)
        .await;

    ingfah(&server)
        .env("INGFAH_API_KEY", "from-env")
        .args(["api", "GET", "/client/voices"])
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_revision_that_drops_a_bound_tool_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/agents/mali"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": {"slug": "mali", "type": "audio", "phone_tools": [{"id": 5}], "ai_plugin_functions": [{"id": 12}]}
        })))
        .mount(&server)
        .await;
    let body =
        json!({"name": "Mali", "ai_plugin_function_ids": [12], "phone_tools": []}).to_string();

    ingfah(&server)
        .args([
            "--yes",
            "api",
            "POST",
            "/client/agents/mali/revisions",
            "--body",
            &body,
        ])
        .assert()
        .code(3)
        .stderr(contains("phone:5"));

    let posts = server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 0);
}
