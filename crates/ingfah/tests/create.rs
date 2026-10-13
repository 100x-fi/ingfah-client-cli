mod common;

use common::{ingfah, request_count};
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SLUG: &str = "ingfah-agent-mali";

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": data}))
}

fn draft(task: &str) -> Value {
    json!({
        "id": 31,
        "prompt_engine_version": "v3",
        "name": "Mali",
        "vocal_name": "มะลิ",
        "ai_greeting_message": "สวัสดีค่ะ",
        "ai_instruction_identity": "You are Mali",
        "ai_instruction_task": task,
        "ai_instruction_flow": [],
        "ai_instructions": "",
        "ai_instruction_variables": null,
        "voice": {"id": 3, "name": "Nara"},
        "voice_speed": null,
        "ai_plugin_functions": [{"id": 12}],
        "phone_tools": [{"id": 5}],
        "text_chat_tools": []
    })
}

fn agent(task: &str) -> Value {
    json!({
        "id": 8, "slug": SLUG, "type": "audio", "name": "", "visibility": "public",
        "description": "Sells policies",
        "ai_agent_instruction_revision": draft(task)
    })
}

async fn mount_create(server: &MockServer, stored_task: &str) {
    Mock::given(method("POST"))
        .and(path("/client/agents"))
        .respond_with(ok(agent("Help")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/client/agents/{SLUG}")))
        .respond_with(ok(agent(stored_task)))
        .mount(server)
        .await;
}

fn audio_create_args() -> Vec<&'static str> {
    vec![
        "agents",
        "create",
        "--visibility",
        "public",
        "--set",
        "name=Mali",
        "--set",
        "vocal_name=มะลิ",
        "--set",
        "ai_greeting_message=สวัสดีค่ะ",
        "--set",
        "ai_instruction_task=Help",
        "--set",
        "voice_id=3",
        "--set",
        "description=Sells policies",
        "--add-tool",
        "plugin:12",
        "--add-tool",
        "phone:5",
    ]
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
async fn create_sends_the_exact_body() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;
    let dir = tempfile::tempdir().expect("temp dir");
    let identity = dir.path().join("identity.md");
    std::fs::write(&identity, "You are Mali").expect("write identity");

    ingfah(&server)
        .arg("--yes")
        .args(audio_create_args())
        .arg("--set-file")
        .arg(format!("ai_instruction_identity={}", identity.display()))
        .assert()
        .success();

    let expected = json!({
        "type": "audio",
        "visibility": "public",
        "name": "Mali",
        "vocal_name": "มะลิ",
        "ai_greeting_message": "สวัสดีค่ะ",
        "ai_instruction_identity": "You are Mali",
        "ai_instruction_task": "Help",
        "voice_id": 3,
        "description": "Sells policies",
        "ai_plugin_function_ids": [12],
        "phone_tools": [5]
    });
    assert_eq!(posted_bodies(&server).await, vec![expected]);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prints_slug_ids_and_publish_command() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;

    let out = ingfah(&server)
        .arg("--yes")
        .args(audio_create_args())
        .args(["--set", "ai_instruction_identity=You are Mali"])
        .output()
        .expect("run");

    let printed: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    assert_eq!(
        printed,
        json!({
            "slug": SLUG,
            "agent_id": 8,
            "revision_id": 31,
            "publish_command": format!("ingfah agents publish {SLUG} --revision 31")
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_table_output_names_the_agent_and_the_publish_command() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;

    ingfah(&server)
        .args(["--yes", "--format", "table"])
        .args(audio_create_args())
        .args(["--set", "ai_instruction_identity=You are Mali"])
        .assert()
        .success()
        .stdout(
            contains(format!("`{SLUG}` (id 8)"))
                .and(contains("#31"))
                .and(contains(format!(
                    "ingfah agents publish {SLUG} --revision 31"
                ))),
        );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_refuses_missing_required_fields_naming_each_one() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;

    ingfah(&server)
        .args([
            "--yes",
            "agents",
            "create",
            "--visibility",
            "private",
            "--set",
            "name=Mali",
        ])
        .assert()
        .code(3)
        .stderr(
            contains("vocal_name")
                .and(contains("ai_greeting_message"))
                .and(contains("ai_instruction_identity"))
                .and(contains("ai_instruction_task")),
        );

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_refuses_a_phone_tool_on_a_text_agent() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;

    ingfah(&server)
        .args([
            "--yes",
            "agents",
            "create",
            "--type",
            "text",
            "--visibility",
            "private",
            "--set",
            "name=Mali",
            "--set",
            "ai_greeting_message=Hi",
            "--set",
            "ai_instruction_identity=You are Mali",
            "--set",
            "ai_instruction_task=Help",
            "--add-tool",
            "phone:1",
        ])
        .assert()
        .code(3)
        .stderr(contains("phone:1"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_refuses_flow_that_references_an_unlinked_tool() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;
    let flow = json!([{"id": "greet", "transitions": [],
        "guidelines": {"plugin_functions": [{"id": 99, "instruction": "x"}]}}]);

    ingfah(&server)
        .arg("--yes")
        .args(audio_create_args())
        .args(["--set", "ai_instruction_identity=You are Mali", "--set"])
        .arg(format!("ai_instruction_flow={flow}"))
        .assert()
        .code(3)
        .stderr(contains("plugin:99"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_dry_run_sends_nothing() {
    let server = MockServer::start().await;
    mount_create(&server, "Help").await;

    ingfah(&server)
        .arg("--dry-run")
        .args(audio_create_args())
        .args(["--set", "ai_instruction_identity=You are Mali"])
        .assert()
        .success()
        .stdout(contains("\"dry_run\": true"));

    assert_eq!(request_count(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_read_back_mismatch_exits_1() {
    let server = MockServer::start().await;
    mount_create(&server, "Something else").await;

    ingfah(&server)
        .arg("--yes")
        .args(audio_create_args())
        .args(["--set", "ai_instruction_identity=You are Mali"])
        .assert()
        .code(1)
        .stderr(contains("ai_instruction_task").and(contains("Do not publish")));
}
