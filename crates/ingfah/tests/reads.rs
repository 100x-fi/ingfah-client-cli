mod common;

use common::{ingfah, request_count};
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn agents_page(page: u32, slug: &str, has_next: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "status": "success",
        "data": [{"slug": slug, "name": slug, "type": "audio"}],
        "pagination": {"page": page, "per_page": 1, "total": 2, "total_pages": 2, "has_next": has_next, "has_prev": page > 1}
    }))
}

async fn two_pages_of_agents() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/ai-agents"))
        .and(query_param("page", "1"))
        .respond_with(agents_page(1, "mali", true))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/ai-agents"))
        .and(query_param("page", "2"))
        .respond_with(agents_page(2, "nara", false))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn list_all_walks_every_page() {
    let server = two_pages_of_agents().await;

    let out = ingfah(&server)
        .args(["agents", "list", "--all"])
        .output()
        .expect("run");

    let rows: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let slugs: Vec<&str> = rows
        .as_array()
        .map(|a| a.iter().filter_map(|r| r["slug"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(slugs, ["mali", "nara"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_without_all_fetches_one_page_and_says_there_is_more() {
    let server = two_pages_of_agents().await;

    ingfah(&server)
        .args(["agents", "list"])
        .assert()
        .success()
        .stdout(contains("mali").and(contains("nara").not()))
        .stderr(contains("--all"));

    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_all_stops_on_an_empty_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success", "data": [],
            "pagination": {"page": 1, "total_pages": 9, "has_next": true}
        })))
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["phone-tools", "list", "--all"])
        .assert()
        .success();

    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn table_format_renders_declared_columns() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/voices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": [{"id": 3, "name": "Nara", "gender": "female", "description": "Warm"}]
        })))
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["--format", "table", "voices", "list"])
        .assert()
        .success()
        .stdout(contains("Gender").and(contains("Nara")));
}

#[tokio::test(flavor = "multi_thread")]
async fn analytics_summary_sends_window_and_joined_teams() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/analytics/summary"))
        .and(query_param("from", "2026-10-01T00:00:00Z"))
        .and(query_param("to", "2026-10-08T00:00:00Z"))
        .and(query_param("client_product_ids", "3,4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "success", "data": {"total_calls": 12}})),
        )
        .expect(1)
        .mount(&server)
        .await;

    ingfah(&server)
        .args([
            "analytics",
            "summary",
            "--from",
            "2026-10-01T00:00:00Z",
            "--to",
            "2026-10-08T00:00:00Z",
            "--team",
            "3",
            "--team",
            "4",
        ])
        .assert()
        .success()
        .stdout(contains("total_calls"));
}
