mod common;

use common::{ingfah, request_count};
use predicates::str::contains;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": data}))
}

async fn mount_agent(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/client/agents/mali"))
        .respond_with(ok(json!({"id": 8, "slug": "mali", "name": "Mali"})))
        .mount(server)
        .await;
}

async fn methods_sent(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_without_yes_on_non_tty_exits_3_and_deletes_nothing() {
    let server = MockServer::start().await;
    mount_agent(&server).await;

    ingfah(&server)
        .args(["agents", "delete", "mali"])
        .assert()
        .code(3);

    assert!(
        !methods_sent(&server)
            .await
            .iter()
            .any(|m| m.starts_with("DELETE"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_with_yes_reads_the_agent_then_deletes_it() {
    let server = MockServer::start().await;
    mount_agent(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/client/agents/mali"))
        .respond_with(ok(json!({"id": 8, "slug": "mali", "status": "deleted"})))
        .mount(&server)
        .await;

    let out = ingfah(&server)
        .args(["--yes", "agents", "delete", "mali"])
        .output()
        .unwrap_or_else(|e| panic!("run: {e}"));

    let printed: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    assert_eq!(
        (methods_sent(&server).await, printed),
        (
            vec![
                "GET /client/agents/mali".to_owned(),
                "DELETE /client/agents/mali".to_owned()
            ],
            json!({"id": 8, "slug": "mali", "status": "deleted"})
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_refused_by_the_server_shows_error_detail() {
    let server = MockServer::start().await;
    mount_agent(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/client/agents/mali"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "status": "error", "error": "bad request",
            "error_detail": "agent is used by an active team"
        })))
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["--yes", "agents", "delete", "mali"])
        .assert()
        .code(1)
        .stderr(contains("agent is used by an active team"));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_dry_run_sends_only_the_read() {
    let server = MockServer::start().await;
    mount_agent(&server).await;

    ingfah(&server)
        .args(["--dry-run", "agents", "delete", "mali"])
        .assert()
        .success()
        .stdout(contains("\"dry_run\": true"));

    assert_eq!(request_count(&server).await, 1);
}
