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
const SESSION: &str =
    "projects/123/locations/us/collections/default_collection/engines/search/sessions/77";

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
    json!({"session": {"name": SESSION, "state": "IN_PROGRESS"}, "answer": {
        "name": format!("{SESSION}/answers/88"),
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
    request.data_stores = vec!["sp_file".into()];
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

#[tokio::test]
async fn folder_search_requires_an_explicit_datastore() {
    let server = MockServer::start().await;
    let mut request = SearchRequest::new("sync");
    request.folder = Some(folder());
    let error = client(&server).await.search(request).await.unwrap_err();
    assert!(
        error.to_string().contains("needs exactly one datastore"),
        "{error}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn pagination_signals_are_reported_even_below_the_page_size() {
    let server = MockServer::start().await;
    // Fewer hits than requested, but the service says there are more.
    let mut response = search_response();
    response["nextPageToken"] = json!("token");
    mount_search(&server, response).await;
    let mut request = SearchRequest::new("sync");
    request.folder = Some(folder());
    request.data_stores = vec!["sp_file".into()];
    request.page_size = 10;
    let results = client(&server).await.search(request.clone()).await.unwrap();
    assert!(results.more_results);
    assert!(results.warnings[0].contains("more files may match"));

    server.reset().await;
    let mut response = search_response();
    response["totalSize"] = json!("7");
    mount_search(&server, response).await;
    let results = client(&server).await.search(request.clone()).await.unwrap();
    assert_eq!(results.total_size, Some(7));
    assert!(results.more_results);

    server.reset().await;
    mount_search(&server, search_response()).await; // totalSize 2, no token, 2 < 10
    let results = client(&server).await.search(request).await.unwrap();
    assert!(!results.more_results);
    assert!(results.warnings.is_empty());
}

fn answer_with_citation(citation: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"session": {"name": SESSION}, "answer": {
        "name": format!("{SESSION}/answers/1"),
        "state": "SUCCEEDED",
        "answerText": "Ä sync answer.",
        "citations": [citation],
        "references": [{"chunkInfo": {"content": "x",
            "documentMetadata": {"uri": format!("{FOLDER}/a.txt"), "title": "A"}}}]
    }}))
    .unwrap()
}

#[test]
fn citation_offsets_are_parsed_strictly() {
    let folder = folder();
    let allowed = allowed_a();
    let ok = parse_folder_answer(
        &answer_with_citation(
            json!({"startIndex": "2", "endIndex": 7, "sources": [{"referenceId": "0"}]}),
        ),
        &folder,
        &allowed,
        &same_session,
    )
    .unwrap();
    assert_eq!(
        (ok.citations[0].start_index, ok.citations[0].end_index),
        (Some(2), Some(7))
    );
    for bad in [
        json!({"startIndex": "5", "endIndex": "999999", "sources": [{"referenceId": "0"}]}),
        json!({"endIndex": "abc", "sources": [{"referenceId": "0"}]}),
        json!({"startIndex": -1, "sources": [{"referenceId": "0"}]}),
        json!({"startIndex": "7", "endIndex": "2", "sources": [{"referenceId": "0"}]}),
        // Byte 1 is inside the two-byte "Ä".
        json!({"startIndex": "1", "sources": [{"referenceId": "0"}]}),
        json!({"endIndex": "3", "sources": [{"referenceId": "1"}]}),
    ] {
        assert!(
            parse_folder_answer(
                &answer_with_citation(bad.clone()),
                &folder,
                &allowed,
                &same_session
            )
            .is_err(),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn folder_answers_create_and_continue_a_session_rescoping_every_turn() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer_response(&format!("{FOLDER}/sub/A-0001.txt"))),
        )
        .expect(2)
        .mount(&server)
        .await;
    let c = client(&server).await;
    let mut first = FolderAnswerRequest::new("What does A say?", "sync", folder());
    first.data_stores = vec!["sp_file".into()];
    let turn1 = c.answer_in_folder(first).await.unwrap();
    assert_eq!(turn1.session, SESSION);
    assert_eq!(turn1.answer_resource, format!("{SESSION}/answers/88"));

    let mut follow_up = FolderAnswerRequest::new("Shorten that.", "sync alpha", folder());
    follow_up.data_stores = vec!["sp_file".into()];
    follow_up.session = Some("77".into());
    c.answer_in_folder(follow_up).await.unwrap();

    let answers = bodies(&server, ":answer").await;
    assert_eq!(answers[0]["session"], format!("{ENGINE}/sessions/-"));
    assert_eq!(answers[1]["session"], SESSION);
    // The follow-up searched the folder again and sent only those files.
    let searches = bodies(&server, ":search").await;
    assert_eq!(searches.len(), 2);
    assert_eq!(
        searches[1]["query"],
        format!("(sync alpha) path:\"{FOLDER}\"")
    );
    assert!(answers[1]["searchSpec"].get("searchParams").is_none());
    assert!(answers[1]["searchSpec"]["searchResultList"].is_object());
}

#[tokio::test]
async fn folder_answer_session_and_answer_names_are_checked() {
    for (session, answer_name) in [
        (
            "projects/123/locations/us/collections/default_collection/engines/other/sessions/77",
            format!("{SESSION}/answers/88"),
        ),
        (SESSION, format!("{ENGINE}/sessions/99/answers/88")),
        (
            "projects/123/locations/us/collections/default_collection/engines/search/sessions/78",
            format!("{SESSION}/answers/88"),
        ),
    ] {
        let server = MockServer::start().await;
        mount_search(&server, search_response()).await;
        let mut response = answer_response(&format!("{FOLDER}/sub/A-0001.txt"));
        response["session"]["name"] = json!(session);
        response["answer"]["name"] = json!(answer_name);
        Mock::given(method("POST"))
            .and(path(format!(
                "/v1/{ENGINE}/servingConfigs/default_search:answer"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        let mut request = FolderAnswerRequest::new("q?", "sync", folder());
        request.data_stores = vec!["sp_file".into()];
        request.session = Some("77".into());
        let c = client(&server).await;
        c.remember_folder_session_binding(SESSION, FOLDER, STORE)
            .unwrap();
        let error = c.answer_in_folder(request).await.unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("mismatch") || message.contains("invalid returned session"),
            "{session}: {message}"
        );
    }
}

#[test]
fn folder_answer_turns_survive_session_readback() {
    let turn: crate::AssistSessionTurn = serde_json::from_value(json!({
        "query": {"text": "What does A say?"},
        "answer": format!("{SESSION}/answers/88"),
        "detailedAnswer": {"name": format!("{SESSION}/answers/88"), "state": "SUCCEEDED",
                           "answerText": "Alpha says sync works.", "references": []}
    }))
    .unwrap();
    assert_eq!(
        turn.answer.as_deref(),
        Some(format!("{SESSION}/answers/88").as_str())
    );
    assert_eq!(
        turn.detailed_answer.as_ref().unwrap()["answerText"],
        "Alpha says sync works."
    );
    assert_eq!(turn.model_id(), None);
    let round_trip = serde_json::to_value(&turn).unwrap();
    assert_eq!(
        round_trip["detailedAnswer"]["answerText"],
        "Alpha says sync works."
    );
}

fn same_session(name: &str) -> Result<String> {
    Ok(name.to_string())
}

fn allowed_a() -> AllowedSources {
    let mut allowed = AllowedSources::default();
    allowed.add(
        &format!("{STORE}/branches/0/documents/A"),
        Some(&format!("{FOLDER}/a.txt?web=1")),
    );
    allowed
}

fn answer_citing(reference_meta: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"session": {"name": SESSION}, "answer": {
        "name": format!("{SESSION}/answers/1"), "state": "SUCCEEDED", "answerText": "Answer.",
        "citations": [{"endIndex": "6", "sources": [{"referenceId": "0"}]}],
        "references": [{"chunkInfo": {"content": "x", "documentMetadata": reference_meta}}]
    }}))
    .unwrap()
}

#[test]
fn references_must_come_from_this_turns_search_results() {
    let (folder, allowed) = (folder(), allowed_a());
    let parse =
        |meta: Value| parse_folder_answer(&answer_citing(meta), &folder, &allowed, &same_session);
    // Supplied document (by resource) passes.
    assert!(parse(json!({"document": format!("{STORE}/branches/0/documents/A"), "uri": format!("{FOLDER}/a.txt")})).is_ok());
    // No document resource: normalized URI fallback (case, encoding and ?web=1 ignored).
    assert!(parse(json!({"uri": format!("{FOLDER}/A.TXT")})).is_ok());
    // In the folder but not supplied this turn: refused, by document and by URI.
    for meta in [
        json!({"document": format!("{STORE}/branches/0/documents/Z"), "uri": format!("{FOLDER}/a.txt")}),
        json!({"uri": format!("{FOLDER}/z.txt")}),
    ] {
        let error = parse(meta.clone()).unwrap_err().to_string();
        assert!(
            error.contains("not among this turn's search results"),
            "{meta}: {error}"
        );
    }
}

#[tokio::test]
async fn folder_answer_rejects_an_in_folder_file_that_was_not_supplied() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    let mut response = answer_response(&format!("{FOLDER}/other/Z-9999.txt"));
    response["answer"]["references"][0]["chunkInfo"]["documentMetadata"]["document"] =
        json!(format!("{STORE}/branches/0/documents/Z-9999"));
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
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
        error
            .to_string()
            .contains("not among this turn's search results"),
        "{error}"
    );
}

#[tokio::test]
async fn skipped_and_failed_answers_never_carry_an_unvalidated_session() {
    let foreign =
        "projects/123/locations/us/collections/default_collection/engines/search/sessions/999";
    for answer in [
        json!({"state": "SUCCEEDED", "answerSkippedReasons": ["OUT_OF_DOMAIN_QUERY_IGNORED"]}),
        json!({"state": "FAILED"}),
    ] {
        let server = MockServer::start().await;
        mount_search(&server, search_response()).await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/v1/{ENGINE}/servingConfigs/default_search:answer"
            )))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"session": {"name": foreign}, "answer": answer})),
            )
            .mount(&server)
            .await;
        let mut request = FolderAnswerRequest::new("q?", "sync", folder());
        request.data_stores = vec!["sp_file".into()];
        request.session = Some("77".into()); // continuing a different session
        let c = client(&server).await;
        c.remember_folder_session_binding(SESSION, FOLDER, STORE)
            .unwrap();
        match c.answer_in_folder(request).await {
            Err(Error::Api(reason)) => assert!(reason.contains("mismatch"), "{reason}"),
            other => panic!("expected a session mismatch, got {other:?}"),
        }
    }
    // A matching session is attached to the skip as before.
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/{ENGINE}/servingConfigs/default_search:answer")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"session": {"name": SESSION},
            "answer": {"state": "SUCCEEDED", "answerSkippedReasons": ["OUT_OF_DOMAIN_QUERY_IGNORED"]}})))
        .mount(&server)
        .await;
    let mut request = FolderAnswerRequest::new("q?", "sync", folder());
    request.data_stores = vec!["sp_file".into()];
    request.session = Some("77".into());
    let c = client(&server).await;
    c.remember_folder_session_binding(SESSION, FOLDER, STORE)
        .unwrap();
    match c.answer_in_folder(request).await {
        Err(Error::Skipped { session, .. }) => assert_eq!(session.as_deref(), Some(SESSION)),
        other => panic!("expected Skipped, got {other:?}"),
    }
}

#[tokio::test]
async fn continuing_folder_session_requires_trusted_local_binding() {
    let server = MockServer::start().await;
    let c = client(&server).await;
    let mut request = FolderAnswerRequest::new("q?", "sync", folder());
    request.data_stores = vec!["sp_file".into()];
    request.session = Some("77".into());
    let error = c.answer_in_folder(request).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("not created by this client instance"),
        "{error}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn trusted_folder_session_is_bound_to_folder_and_datastore() {
    let server = MockServer::start().await;
    let c = client(&server).await;
    c.remember_folder_session_binding(SESSION, FOLDER, STORE)
        .unwrap();

    let other = SharePointFolder::parse(
        "https://example.sharepoint.com/sites/eng/Shared%20Documents/Other",
    )
    .unwrap();
    let mut wrong_folder = FolderAnswerRequest::new("q?", "sync", other);
    wrong_folder.data_stores = vec!["sp_file".into()];
    wrong_folder.session = Some("77".into());
    assert!(c
        .answer_in_folder(wrong_folder)
        .await
        .unwrap_err()
        .to_string()
        .contains("different folder or datastore"));

    let mut wrong_store = FolderAnswerRequest::new("q?", "sync", folder());
    wrong_store.data_stores = vec!["other_store".into()];
    wrong_store.session = Some("77".into());
    assert!(c
        .answer_in_folder(wrong_store)
        .await
        .unwrap_err()
        .to_string()
        .contains("different folder or datastore"));

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn folder_session_without_structured_references_can_continue() {
    let server = MockServer::start().await;
    mount_search(&server, search_response()).await;
    let response = json!({"session": {"name": SESSION}, "answer": {
        "name": format!("{SESSION}/answers/88"),
        "state": "SUCCEEDED",
        "answerText": "Scoped answer without references.",
        "citations": [],
        "references": []
    }});
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/servingConfigs/default_search:answer"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(2)
        .mount(&server)
        .await;

    let c = client(&server).await;
    let mut first = FolderAnswerRequest::new("q1?", "sync", folder());
    first.data_stores = vec!["sp_file".into()];
    let first = c.answer_in_folder(first).await.unwrap();
    assert!(first.references.is_empty());

    let mut follow = FolderAnswerRequest::new("q2?", "sync", folder());
    follow.data_stores = vec!["sp_file".into()];
    follow.session = Some(first.session);
    c.answer_in_folder(follow).await.unwrap();

    assert_eq!(bodies(&server, ":answer").await.len(), 2);
}

#[tokio::test]
async fn folder_scope_rejects_multiple_datastores_before_network() {
    let server = MockServer::start().await;
    let c = client(&server).await;

    let mut search = SearchRequest::new("sync");
    search.folder = Some(folder());
    search.data_stores = vec!["sp_file".into(), "crm".into()];
    assert!(c
        .search(search)
        .await
        .unwrap_err()
        .to_string()
        .contains("exactly one datastore"));

    let mut answer = FolderAnswerRequest::new("q?", "sync", folder());
    answer.data_stores = vec!["sp_file".into(), "crm".into()];
    assert!(c
        .answer_in_folder(answer)
        .await
        .unwrap_err()
        .to_string()
        .contains("exactly one datastore"));

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn empty_content_is_not_counted_as_full_text() {
    let server = MockServer::start().await;
    let mut response = search_response();
    response["results"][0]["document"]["structData"]["content"] = json!("   ");
    mount_search(&server, response).await;
    let mut request = SearchRequest::new("sync");
    request.folder = Some(folder());
    request.data_stores = vec!["sp_file".into()];
    let results = client(&server).await.search(request).await.unwrap();
    assert_eq!(results.hits[0].content, None);
}
