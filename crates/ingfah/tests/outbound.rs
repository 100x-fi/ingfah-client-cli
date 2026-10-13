#![expect(
    clippy::expect_used,
    reason = "test fixtures panic when the temp dir is unusable"
)]

mod common;

use common::{ingfah, request_count};
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn batch_file(dir: &tempfile::TempDir, with_slots: bool) -> std::path::PathBuf {
    let mut schedule = json!({
        "client_product_id": 3,
        "start_time": "2026-10-14 09:00:00",
        "end_time": "2026-10-14 17:00:00",
        "day_slot": "0111110",
        "timezone": "Asia/Bangkok",
        "records": [
            {"row_number": 1, "phone_number": "+66812345678"},
            {"row_number": 2, "phone_number": "+66812345679"}
        ]
    });
    if with_slots {
        schedule["time_slots"] = json!([{"from": "09:00", "to": "17:00"}]);
    }
    let body = json!({"client_outbound_option_id": 1, "name": "October", "est_duration_minutes": 3, "schedules": [schedule]});
    let file = dir.path().join("batch.json");
    std::fs::write(&file, body.to_string()).expect("write batch file");
    file
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_create_without_yes_exits_3_and_sends_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("tempdir");

    ingfah(&server)
        .args(["batches", "create", "--file"])
        .arg(batch_file(&dir, true))
        .assert()
        .code(3);

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_create_refuses_schedule_without_time_slots() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("tempdir");

    ingfah(&server)
        .args(["--yes", "batches", "create", "--file"])
        .arg(batch_file(&dir, false))
        .assert()
        .code(3)
        .stderr(contains("time_slots"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_create_with_invalid_records_exits_1_and_lists_them() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/client/outbound/batches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": {"invalids": [{"error": "DUPLICATE_PHONE_NUMBER", "column": "phone_number", "row_numbers": [2]}]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");

    ingfah(&server)
        .args(["--yes", "batches", "create", "--file"])
        .arg(batch_file(&dir, true))
        .assert()
        .code(1)
        .stderr(contains("DUPLICATE_PHONE_NUMBER").and(contains("not created")));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_create_422_shows_invalids_from_error_data() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "status": "error", "error": "unprocessable entity", "error_detail": "invalid records",
            "data": {"invalids": [{"error": "INVALID_PHONE_NUMBER", "column": "phone_number", "row_numbers": [1]}]}
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");

    ingfah(&server)
        .args(["--yes", "batches", "create", "--file"])
        .arg(batch_file(&dir, true))
        .assert()
        .code(1)
        .stderr(contains("INVALID_PHONE_NUMBER"));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_cancel_dry_run_sends_nothing() {
    let server = MockServer::start().await;

    ingfah(&server)
        .args(["--dry-run", "batches", "cancel", "7"])
        .assert()
        .success()
        .stdout(contains("destructive"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_list_sends_repeated_array_filters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/chat-sessions-list"))
        .and(query_param("product_id[]", "3"))
        .and(query_param("outcome[]", "interested"))
        .and(query_param("start_time", "2026-10-01T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": {"rows": [{"id": "u-1", "title": "Policy"}], "filters": {}},
            "pagination": {"page": 1, "total_pages": 1, "has_next": false}
        })))
        .expect(1)
        .mount(&server)
        .await;

    ingfah(&server)
        .args([
            "calls",
            "list",
            "--team",
            "3",
            "--outcome",
            "interested",
            "--from",
            "2026-10-01T00:00:00Z",
        ])
        .assert()
        .success()
        .stdout(contains("u-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn recording_hides_presigned_url_unless_asked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/chat-sessions/u-1/record"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": {"uuid": "u-1", "audio_record_url": "https://storage.example/secret-sig", "checksum": "abc123"}
        })))
        .mount(&server)
        .await;

    ingfah(&server)
        .args(["calls", "recording", "u-1"])
        .assert()
        .success()
        .stdout(contains("abc123").and(contains("secret-sig").not()));
}

#[tokio::test(flavor = "multi_thread")]
async fn recording_out_saves_audio_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/client/chat-sessions/u-1/record/download"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![0x4f, 0x67, 0x67, 0x53], "audio/ogg"),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("call.ogg");

    ingfah(&server)
        .args(["calls", "recording", "u-1", "--out"])
        .arg(&file)
        .assert()
        .success();

    assert_eq!(
        std::fs::read(&file).ok(),
        Some(vec![0x4f, 0x67, 0x67, 0x53])
    );
}
