mod common;

use common::ingfah;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": data}))
}

fn flow() -> Value {
    json!([{
        "id": "greet",
        "instructions": ["Greet the caller"],
        "transitions": [{"condition": "", "next_step": "lookup"}],
        "guidelines": {"end_call_keyword": false, "plugin_functions": [{"id": 12, "instruction": "Look it up"}]}
    }])
}

fn agent_detail(phone_tools: Value) -> Value {
    json!({
        "slug": "mali", "type": "audio", "name": "Mali",
        "voice": {"id": 3, "name": "Nara"},
        "ai_plugin_functions": [{"id": 12, "signature": "lookup_policy"}],
        "phone_tools": phone_tools
    })
}

fn published_revision() -> Value {
    json!({
        "id": 17, "prompt_engine_version": "v3", "name": "Mali", "vocal_name": "มะลิ",
        "greeting_message": "สวัสดีค่ะ",
        "ai_instruction_identity": "You are Mali",
        "ai_instruction_task": "Old task",
        "ai_instruction_flow": flow(),
        "ai_instructions": "Be brief",
        "ai_instruction_variables": {"company": "Ingfah"},
        "voice": {"id": 3, "name": "Nara"}, "voice_speed": 1,
        "created_at": "2026-10-01T00:00:00Z", "published_at": "2026-10-01T01:00:00Z"
    })
}

fn stored_revision(task: &str) -> Value {
    let mut rev = published_revision();
    rev["id"] = json!(18);
    rev["ai_instruction_task"] = json!(task);
    rev["published_at"] = Value::Null;
    rev
}

async fn mount_agent(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/client/agents/mali"))
        .respond_with(ok(agent_detail(json!([{"id": 5, "name": "transfer"}]))))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/agents/mali/revisions"))
        .respond_with(ok(
            json!({"revisions": [{"id": 17, "status": "published"}]}),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/agents/mali/revisions/17"))
        .respond_with(ok(published_revision()))
        .mount(server)
        .await;
}

async fn mount_create(server: &MockServer, stored_task: &str) {
    Mock::given(method("POST"))
        .and(path("/client/agents/mali/revisions"))
        .respond_with(ok(json!({
            "id": 18,
            "ai_plugin_functions": [{"id": 12}],
            "phone_tools": [{"id": 5}],
            "text_chat_tools": []
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/agents/mali/revisions/18"))
        .respond_with(ok(stored_revision(stored_task)))
        .mount(server)
        .await;
}

async fn posted_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST")
        .filter_map(|r| serde_json::from_slice(&r.body).ok())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_posts_full_snapshot_with_preserved_tools_and_mapped_fields() {
    let server = MockServer::start().await;
    mount_agent(&server).await;
    mount_create(&server, "New task").await;

    ingfah(&server)
        .args([
            "--yes",
            "agents",
            "revise",
            "mali",
            "--set",
            "ai_instruction_task=New task",
        ])
        .assert()
        .success()
        .stdout(contains("ingfah agents publish mali --revision 18"));

    let expected = json!({
        "prompt_engine_version": "v3",
        "name": "Mali",
        "vocal_name": "มะลิ",
        "ai_greeting_message": "สวัสดีค่ะ",
        "ai_instruction_identity": "You are Mali",
        "ai_instruction_task": "New task",
        "ai_instruction_flow": flow(),
        "ai_instructions": "Be brief",
        "ai_instruction_variables": {"company": "Ingfah"},
        "voice_id": 3,
        "voice_speed": 1,
        "ai_plugin_function_ids": [12],
        "phone_tools": [5]
    });
    assert_eq!(posted_bodies(&server).await, vec![expected]);
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_add_tool_extends_the_list() {
    let server = MockServer::start().await;
    mount_agent(&server).await;
    mount_create(&server, "Old task").await;

    let _ = ingfah(&server)
        .args([
            "--yes",
            "agents",
            "revise",
            "mali",
            "--add-tool",
            "plugin:40",
        ])
        .output();

    let bodies = posted_bodies(&server).await;
    assert_eq!(
        bodies.first().map(|b| b["ai_plugin_function_ids"].clone()),
        Some(json!([12, 40]))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_read_back_mismatch_exits_non_zero() {
    let server = MockServer::start().await;
    mount_agent(&server).await;
    mount_create(&server, "Something else").await;

    ingfah(&server)
        .args([
            "--yes",
            "agents",
            "revise",
            "mali",
            "--set",
            "ai_instruction_task=New task",
        ])
        .assert()
        .code(1)
        .stderr(contains("ai_instruction_task").and(contains("Do not publish")));
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_dry_run_sends_no_write() {
    let server = MockServer::start().await;
    mount_agent(&server).await;

    ingfah(&server)
        .args([
            "--dry-run",
            "agents",
            "revise",
            "mali",
            "--set",
            "name=Malee",
        ])
        .assert()
        .success()
        .stdout(contains("\"dry_run\": true").and(contains("\"phone_tools\"")));

    assert!(posted_bodies(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_without_yes_on_non_tty_exits_3_without_a_write() {
    let server = MockServer::start().await;
    mount_agent(&server).await;

    ingfah(&server)
        .args(["agents", "revise", "mali", "--set", "name=Malee"])
        .assert()
        .code(3);

    assert!(posted_bodies(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_refuses_flow_that_references_an_unlinked_tool() {
    let server = MockServer::start().await;
    mount_agent(&server).await;
    let new_flow = json!([{"id": "greet", "transitions": [],
        "guidelines": {"plugin_functions": [{"id": 99, "instruction": "x"}]}}]);

    ingfah(&server)
        .args(["--yes", "agents", "revise", "mali", "--set"])
        .arg(format!("ai_instruction_flow={new_flow}"))
        .assert()
        .code(3)
        .stderr(contains("plugin:99"));

    assert!(posted_bodies(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn revise_refuses_setting_a_tool_list_directly() {
    let server = MockServer::start().await;
    mount_agent(&server).await;

    ingfah(&server)
        .args([
            "--yes",
            "agents",
            "revise",
            "mali",
            "--set",
            "phone_tools=[]",
        ])
        .assert()
        .failure()
        .stderr(contains("--add-tool"));

    assert!(posted_bodies(&server).await.is_empty());
}

async fn mount_publish_state(server: &MockServer, draft_phone: Value, phone_after: Value) {
    Mock::given(method("GET"))
        .and(path("/client/agents/mali/revisions"))
        .respond_with(ok(json!({"revisions": [
            {"id": 18, "status": "draft"}, {"id": 17, "status": "published"}
        ]})))
        .mount(server)
        .await;
    let mut before = agent_detail(json!([{"id": 5}]));
    before["ai_agent_instruction_revision"] =
        json!({"id": 18, "ai_plugin_functions": [{"id": 12}], "phone_tools": draft_phone});
    Mock::given(method("GET"))
        .and(path("/client/agents/mali"))
        .respond_with(ok(before))
        .up_to_n_times(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/agents/mali"))
        .respond_with(ok(agent_detail(phone_after)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/client/agents/mali/revisions/18"))
        .respond_with(ok(stored_revision("New task")))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/client/agents/mali/revisions/18/publish"))
        .respond_with(ok(
            json!({"revision_id": 18, "published_at": "2026-10-13T00:00:00Z"}),
        ))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_succeeds_when_tools_and_voice_match_the_revision() {
    let server = MockServer::start().await;
    mount_publish_state(&server, json!([{"id": 5}]), json!([{"id": 5}])).await;

    ingfah(&server)
        .args(["--yes", "agents", "publish", "mali", "--revision", "18"])
        .assert()
        .success()
        .stdout(contains("phone:5"));
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_fails_loudly_when_a_tool_comes_unbound() {
    let server = MockServer::start().await;
    mount_publish_state(&server, json!([{"id": 5}]), json!([])).await;

    ingfah(&server)
        .args(["--yes", "agents", "publish", "mali", "--revision", "18"])
        .assert()
        .code(1)
        .stderr(contains("-phone:5"));
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_refuses_a_revision_that_is_not_the_newest() {
    let server = MockServer::start().await;
    mount_publish_state(&server, json!([{"id": 5}]), json!([{"id": 5}])).await;

    ingfah(&server)
        .args(["--yes", "agents", "publish", "mali", "--revision", "17"])
        .assert()
        .code(3)
        .stderr(contains("#18"));

    assert!(posted_bodies(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_refuses_a_draft_that_unbinds_a_tool() {
    let server = MockServer::start().await;
    mount_publish_state(&server, json!([]), json!([])).await;

    ingfah(&server)
        .args(["--yes", "agents", "publish", "mali", "--revision", "18"])
        .assert()
        .code(3)
        .stderr(contains("-phone:5"));

    assert!(posted_bodies(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_unbinds_a_tool_with_allow_tool_drop() {
    let server = MockServer::start().await;
    mount_publish_state(&server, json!([]), json!([])).await;

    ingfah(&server)
        .args([
            "--yes",
            "agents",
            "publish",
            "mali",
            "--revision",
            "18",
            "--allow-tool-drop",
        ])
        .assert()
        .success();
}
