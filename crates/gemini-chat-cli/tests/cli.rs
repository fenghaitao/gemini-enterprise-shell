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
#[tokio::test]
async fn closed_stdout_exits_quietly_without_panicking() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/{ENGINE}/sessions/456")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"name":format!("{ENGINE}/sessions/456"),"turns":[]}))
                // Give the test time to close the pipe before any output is written.
                .set_delay(std::time::Duration::from_millis(500)),
        )
        .mount(&server)
        .await;
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("gemini-chat"));
    for (key, _) in std::env::vars() {
        if key.starts_with("GEMINI_CHAT_") {
            cmd.env_remove(key);
        }
    }
    let base = format!("{}/v1", server.uri());
    let mut child = cmd
        .env("GEMINI_CHAT_ACCESS_TOKEN", "DUMMY_TOKEN")
        .args([
            "--project-number",
            "123456",
            "--location",
            "us",
            "--auth",
            "env",
            "--base-url",
            &base,
            "--engine-id",
            "search",
            "session",
            "--session-id",
            "456",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // the reader goes away, like `| head` exiting early
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "status {:?}, stderr: {stderr}",
        output.status
    );
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn session_output_is_a_summary_or_json_never_both() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/{ENGINE}/sessions/456")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": format!("{ENGINE}/sessions/456"),
            "displayName": "Demo",
            "turns": [{"query": {"text": "first question"},
                       "queryConfig": {"google.discoveryengine.googleapis.com.Assistant.model_id": "gemini-4-argon"}}]
        })))
        .mount(&server)
        .await;
    let text = command(&server)
        .args(["session", "--session-id", "456"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.contains("Turn 1: model gemini-4-argon | first question"),
        "{text}"
    );
    assert!(text.contains("Title: Demo"));
    assert!(
        !text.contains('{'),
        "summary must not include the JSON dump: {text}"
    );
    let json_out = command(&server)
        .args(["--json", "session", "--session-id", "456"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&json_out).expect("--json prints only JSON");
    assert_eq!(value["displayName"], "Demo");
}

const FOLDER: &str = "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs";

fn folder_search_response() -> Value {
    json!({"results": [
        {"document": {"name": "docs/a", "structData": {"title": "A-0001", "url": format!("{FOLDER}/A-0001.txt?web=1"), "content": "Alpha full text"}}},
        {"document": {"name": "docs/b", "structData": {"title": "B-0002", "url": format!("{FOLDER}/sub/B-0002.txt"), "description": "<c0>Beta</c0> snippet"}}}
    ]})
}

#[tokio::test]
async fn search_restricts_to_the_folder_and_lists_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:search"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(folder_search_response()))
        .expect(1)
        .mount(&server)
        .await;
    command(&server)
        .args([
            "search",
            "--sharepoint-folder",
            "https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs/",
            "--data-store-id",
            "sp_file",
            "alpha",
            "beta",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"))
        .stdout(predicate::str::contains("[1] A-0001"))
        .stdout(predicate::str::contains("(full text)"))
        .stdout(predicate::str::contains("(snippet)"));
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["query"], format!("(alpha beta) path:\"{FOLDER}\""));
}

#[tokio::test]
async fn answer_uses_only_folder_files_and_prints_references() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:search"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(folder_search_response()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answer": {
            "state": "SUCCEEDED", "answerText": "Alpha answer.",
            "citations": [{"endIndex": "6", "sources": [{"referenceId": "0"}]}],
            "references": [{"chunkInfo": {"content": "Alpha", "documentMetadata": {"uri": format!("{FOLDER}/A-0001.txt"), "title": "A-0001"}}}]
        }})))
        .expect(1)
        .mount(&server)
        .await;
    command(&server)
        .args([
            "answer",
            "--sharepoint-folder",
            FOLDER,
            "--search-terms",
            "alpha",
            "--data-store-id",
            "sp_file",
            "What does A say?",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha answer."))
        .stdout(predicate::str::contains(
            "Files used: 2 (1 full text, 1 snippet)",
        ))
        .stdout(predicate::str::contains("[0] A-0001"))
        .stderr(predicate::str::contains(
            "1 of 2 files were sent as short snippets",
        ));
    // Sharing links and path: in the terms are rejected before any request.
    let before = server.received_requests().await.unwrap().len();
    command(&server)
        .args([
            "answer",
            "--sharepoint-folder",
            "https://example.sharepoint.com/:f:/s/eng/Eabc",
            "--search-terms",
            "x",
            "--data-store-id",
            "sp_file",
            "q",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sharing links"));
    command(&server)
        .args([
            "search",
            "--sharepoint-folder",
            FOLDER,
            "x",
            "path:\"https://example.sharepoint.com/sites/other\"",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must not contain path:"));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}
