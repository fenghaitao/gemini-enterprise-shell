use std::sync::Arc;

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::{GeminiEnterpriseConfig, StaticTokenProvider};

// Synthetic tenant and files; shapes follow live Search/Answer responses.
const FOLDER: &str = "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs";
const ENGINE: &str = "projects/123/locations/us/collections/default_collection/engines/search";
const STORE: &str = "projects/123/locations/us/collections/default_collection/dataStores/sp_file";

async fn client(server: &MockServer) -> GeminiEnterpriseClient {
    GeminiEnterpriseClient::new(
        Arc::new(StaticTokenProvider::new("t")),
        GeminiEnterpriseConfig::new("123", "us", "search"),
    )
    .unwrap()
    .with_base_url(&format!("{}/v1", server.uri()))
    .unwrap()
}

fn folder() -> SharePointFolder {
    SharePointFolder::parse(FOLDER).unwrap()
}

fn hit(name: &str, file: &str, content: Option<&str>) -> Value {
    let mut data = json!({
        "title": name,
        "url": format!("{FOLDER}/{file}?web=1"),
        "file_type": "txt",
        "description": format!("<ddd/> {name} <c0>sync</c0> snippet"),
    });
    if let Some(content) = content {
        data["content"] = json!(content);
    }
    json!({"id": name, "document": {"name": format!("{STORE}/branches/0/documents/{name}"), "structData": data}})
}

fn search_response() -> Value {
    json!({"results": [
        hit("A-0001", "sub/A-0001.txt", Some("Alpha full text about sync.")),
        hit("B-0002", "B-0002.txt", None),
    ], "totalSize": 2})
}

fn answer_response(uri: &str) -> Value {
    json!({"answer": {
        "state": "SUCCEEDED",
        "answerText": "Alpha says sync works.",
        "citations": [{"endIndex": "22", "sources": [{"referenceId": "0"}]}],
        "references": [{"chunkInfo": {"content": "Alpha full text", "relevanceScore": 0.7,
            "documentMetadata": {"document": format!("{STORE}/branches/0/documents/A-0001"),
                                 "uri": uri, "title": "A-0001"}}}]
    }, "answerQueryToken": "tok"})
}

async fn mount_search(server: &MockServer, response: Value) {
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:search"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(server)
        .await;
}

async fn bodies(server: &MockServer, suffix: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with(suffix))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn search_sends_a_folder_restricted_keyword_query_and_parses_hits() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    let mut request = SearchRequest::new("sync (alpha OR beta)");
    request.folder = Some(folder());
    request.data_stores = vec!["sp_file".into()];
    request.page_size = 2;
    let results = client(&server).await.search(request).await.unwrap();

    let body = &bodies(&server, ":search").await[0];
    assert_eq!(
        body["query"],
        format!("(sync (alpha OR beta)) path:\"{FOLDER}\"")
    );
    assert_eq!(body["pageSize"], 2);
    assert_eq!(body["spellCorrectionSpec"]["mode"], "SUGGESTION_ONLY");
    assert_eq!(body["dataStoreSpecs"][0]["dataStore"], STORE);

    assert_eq!(results.hits.len(), 2);
    assert_eq!(
        results.hits[0].content.as_deref(),
        Some("Alpha full text about sync.")
    );
    assert_eq!(results.hits[1].content, None);
    assert_eq!(
        results.hits[1].snippet.as_deref(),
        Some("B-0002 sync snippet")
    );
    // As many hits as requested: more may exist.
    assert!(results.warnings[0].contains("more files may match"));
}

#[tokio::test]
async fn search_fails_closed_on_any_result_outside_the_folder() {
    let server = MockServer::start().await;
    let mut response = search_response();
    response["results"][1]["document"]["structData"]["url"] =
        json!("https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs-old/x.txt");
    mount_search(&server, response).await;
    let mut request = SearchRequest::new("sync");
    request.folder = Some(folder());
    let error = client(&server).await.search(request).await.unwrap_err();
    assert!(
        error.to_string().contains("outside the SharePoint folder"),
        "{error}"
    );
}

#[tokio::test]
async fn folder_answer_sends_only_the_searched_files_and_checks_references() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer_response(&format!("{FOLDER}/sub/A-0001.txt?web=1"))),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut request = FolderAnswerRequest::new("What does A say about sync?", "sync", folder());
    request.data_stores = vec!["sp_file".into()];
    let answer = client(&server)
        .await
        .answer_in_folder(request)
        .await
        .unwrap();

    let body = &bodies(&server, ":answer").await[0];
    assert_eq!(body["query"]["text"], "What does A say about sync?");
    assert_eq!(
        body["queryUnderstandingSpec"]["queryRephraserSpec"]["disable"],
        true
    );
    assert_eq!(
        body["queryUnderstandingSpec"]["disableSpellCorrection"],
        true
    );
    assert_eq!(body["answerGenerationSpec"]["includeCitations"], true);
    assert!(
        body["searchSpec"].get("searchParams").is_none(),
        "no server-side search"
    );
    let results = body["searchSpec"]["searchResultList"]["searchResults"]
        .as_array()
        .unwrap();
    assert_eq!(results.len(), 2);
    let full = &results[0]["unstructuredDocumentInfo"];
    assert_eq!(
        full["documentContexts"][0]["content"],
        "Alpha full text about sync."
    );
    assert_eq!(full["uri"], format!("{FOLDER}/sub/A-0001.txt?web=1"));
    let snippet = &results[1]["unstructuredDocumentInfo"];
    assert_eq!(
        snippet["extractiveSegments"][0]["content"],
        "B-0002 sync snippet"
    );

    assert_eq!(answer.answer, "Alpha says sync works.");
    assert_eq!(answer.folder, FOLDER);
    assert_eq!(answer.citations[0].reference_ids, [0]);
    assert_eq!(answer.citations[0].end_index, Some(22));
    assert_eq!(answer.references[0].title.as_deref(), Some("A-0001"));
    assert_eq!(answer.sources.len(), 2);
    assert!(answer.sources[0].full_text && !answer.sources[1].full_text);
    assert!(answer
        .warnings
        .iter()
        .any(|w| w.contains("1 of 2 files were sent as short snippets")));
}

#[tokio::test]
async fn folder_answer_refuses_references_outside_the_folder() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(answer_response(
            "https://example.sharepoint.com/sites/other/Shared%20Documents/x.txt",
        )))
        .mount(&server)
        .await;
    let mut request = FolderAnswerRequest::new("q?", "sync", folder());
    request.data_stores = vec!["sp_file".into()];
    let error = client(&server)
        .await
        .answer_in_folder(request)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("outside the SharePoint folder"),
        "{error}"
    );
}

#[tokio::test]
async fn skipped_answers_are_errors_even_when_state_is_succeeded() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answer": {
            "state": "SUCCEEDED",
            "answerText": "A summary could not be generated for your search query.",
            "answerSkippedReasons": ["OUT_OF_DOMAIN_QUERY_IGNORED"]}})))
        .mount(&server)
        .await;
    let mut request = FolderAnswerRequest::new("q?", "sync", folder());
    request.data_stores = vec!["sp_file".into()];
    match client(&server).await.answer_in_folder(request).await {
        Err(Error::Skipped { reasons, session }) => {
            assert_eq!(reasons, ["OUT_OF_DOMAIN_QUERY_IGNORED"]);
            assert!(session.is_none());
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
}

#[tokio::test]
async fn no_matching_files_skips_without_calling_the_answer_api() {
    let server = MockServer::start().await;
    mount_search(&server, json!({"totalSize": 0})).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    let mut request = FolderAnswerRequest::new("q?", "nothing matches", folder());
    request.data_stores = vec!["sp_file".into()];
    let error = client(&server)
        .await
        .answer_in_folder(request)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Skipped { .. }));
    assert!(
        error.to_string().contains("no files in the folder matched"),
        "{error}"
    );
}

#[tokio::test]
async fn invalid_requests_are_rejected_without_network() {
    let server = MockServer::start().await;
    let c = client(&server).await;
    for size in [0, MAX_SEARCH_RESULTS + 1] {
        let mut request = SearchRequest::new("sync");
        request.page_size = size;
        assert!(c.search(request).await.is_err());
    }
    assert!(c
        .search(SearchRequest::new("x path:\"https://e.test/a\""))
        .await
        .is_err());
    let no_store = FolderAnswerRequest::new("q?", "sync", folder());
    assert!(c.answer_in_folder(no_store).await.is_err());
    let mut empty_question = FolderAnswerRequest::new("  ", "sync", folder());
    empty_question.data_stores = vec!["sp_file".into()];
    assert!(c.answer_in_folder(empty_question).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn snippet_highlight_markup_is_removed_but_other_text_is_kept() {
    assert_eq!(
        clean_snippet("<ddd/>a <c0>b</c0>  c <c12>d</c12>"),
        "a b c d"
    );
    assert_eq!(clean_snippet("x < y <b>z</b>"), "x < y <b>z</b>");
}
