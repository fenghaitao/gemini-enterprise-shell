use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ENGINE: &str = "projects/123456/locations/us/collections/default_collection/engines/search";
fn command(server: &MockServer) -> Command {
    let mut c = Command::cargo_bin("gemini-chat").unwrap();
    for (key, _) in std::env::vars() {
        if key.starts_with("GEMINI_CHAT_") {
            c.env_remove(key);
        }
    }
    c.env("GEMINI_CHAT_ACCESS_TOKEN", "DUMMY_TOKEN").args([
        "--project-number",
        "123456",
        "--location",
        "us",
        "--auth",
        "env",
        "--base-url",
        &format!("{}/v1", server.uri()),
        "--engine-id",
        "search",
    ]);
    c
}
fn response(cited: bool) -> Value {
    let mut replies = vec![json!({"groundedContent":{"content":{"text":"Answer"}}})];
    if cited {
        replies.push(json!({"groundedContent":{"textGroundingMetadata":{"references":[{"documentMetadata":{"title":"Doc","uri":"https://example.test/doc"}}]}}}));
    }
    json!([{"answer":{"state":"SUCCEEDED","name":format!("{ENGINE}/sessions/456/assistAnswers/789"),"replies":replies},"sessionInfo":{"session":format!("{ENGINE}/sessions/456")}}])
}
#[tokio::test]
async fn ask_and_continue_use_explicit_model_datastore_and_server_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/assistants/default_assistant:streamAssist"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(response(true)))
        .expect(2)
        .mount(&server)
        .await;
    let output = command(&server)
        .args([
            "ask",
            "--model",
            "gemini-4-argon",
            "--data-store-id",
            "sharepoint_file",
            "--json",
            "q",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["requested_model"], "gemini-4-argon");
    assert_eq!(
        value["grounding"][0]["references"][0]["documentMetadata"]["uri"],
        "https://example.test/doc"
    );
    command(&server)
        .args([
            "ask",
            "--session-id",
            "456",
            "--model",
            "gemini-4-argon",
            "q2",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Answer"));
    let requests = server.received_requests().await.unwrap();
    let first: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(first["generationSpec"]["modelId"], "gemini-4-argon");
    assert_eq!(
        first["toolsSpec"]["vertexAiSearchSpec"]["dataStoreSpecs"][0]["dataStore"],
        "projects/123456/locations/us/collections/default_collection/dataStores/sharepoint_file"
    );
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(second["session"], format!("{ENGINE}/sessions/456"));
    assert!(requests.iter().all(|r| r.headers.get("cookie").is_none()));
}
#[tokio::test]
async fn session_get_reports_model_without_generation() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/v1/{ENGINE}/sessions/456"))).and(query_param("includeAnswerDetails","true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":format!("{ENGINE}/sessions/456"),"turns":[{"queryConfig":{"google.discoveryengine.googleapis.com.Assistant.model_id":"gemini-4-argon"}}]})))
        .expect(1).mount(&server).await;
    command(&server)
        .args([
            "session",
            "--session-id",
            "456",
            "--include-answer-details",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("gemini-4-argon"));
}
#[tokio::test]
async fn uncited_answers_warn_and_failed_generations_never_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response(false)))
        .expect(1)
        .mount(&server)
        .await;
    command(&server)
        .args(["ask", "q"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Native grounding references: 0"))
        .stderr(predicate::str::contains("No native grounding references"));
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    command(&server)
        .args(["ask", "--model", "gemini-4-argon", "q"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("do not automatically repeat"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn invalid_session_is_rejected_without_network() {
    let server = MockServer::start().await;
    command(&server)
        .args(["ask", "--session-id", "../other", "q"])
        .assert()
        .failure();
    assert!(server.received_requests().await.unwrap().is_empty());
}
