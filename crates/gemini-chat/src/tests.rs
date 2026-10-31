use super::*;
use crate::StaticTokenProvider;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ENGINE: &str = "projects/123/locations/us/collections/default_collection/engines/search";
const SESSION: &str =
    "projects/123/locations/us/collections/default_collection/engines/search/sessions/456";
const PATH: &str = "/v1/projects/123/locations/us/collections/default_collection/engines/search/assistants/default_assistant:streamAssist";

fn client() -> GeminiEnterpriseClient {
    GeminiEnterpriseClient::new(
        Arc::new(StaticTokenProvider::new("secret-token")),
        GeminiEnterpriseConfig::new("123", "us", "search"),
    )
    .unwrap()
}
fn frames() -> Value {
    json!([
        {"answer":{"state":"IN_PROGRESS","replies":[{"groundedContent":{"content":{"text":"答え "}}}]},"sessionInfo":{"session":SESSION}},
        {"answer":{"state":"IN_PROGRESS","replies":[{"groundedContent":{"content":{"text":"evidence"}}},{"groundedContent":{"content":{"text":"hidden thought","thought":true}}}]}},
        {"answer":{"state":"SUCCEEDED","name":format!("{SESSION}/assistAnswers/789"),"replies":[{"groundedContent":{"textGroundingMetadata":{"references":[{"content":"source extract","documentMetadata":{"uri":"https://example.test/doc","title":"Doc"}}],"segments":[{"startIndex":"0","endIndex":"6","referenceIndices":[0],"text":"答え"}]}}}]},"sessionInfo":{"session":SESSION},"invocationTools":["search"]}
    ])
}
fn parse(value: Value) -> Result<AssistResponse> {
    client().parse_answer(
        &serde_json::to_vec(&value).unwrap(),
        Some("gemini-4-argon".into()),
        None,
    )
}

#[test]
fn exact_v1_request_uses_model_session_and_datastore_without_browser_fields() {
    let mut r = AssistRequest::new("question");
    r.model_id = Some("gemini-4-argon".into());
    r.session = Some("456".into());
    r.data_stores = vec!["sharepoint_file".into()];
    let (body, session) = client().request_body(&r).unwrap();
    assert_eq!(session.as_deref(), Some(SESSION));
    assert_eq!(
        body,
        json!({"query":{"text":"question"},"session":SESSION,"generationSpec":{"modelId":"gemini-4-argon"},"toolsSpec":{"vertexAiSearchSpec":{"dataStoreSpecs":[{"dataStore":"projects/123/locations/us/collections/default_collection/dataStores/sharepoint_file"}]}}})
    );
}

#[test]
fn defaults_and_explicit_web_grounding_are_distinct() {
    let mut r = AssistRequest::new("q");
    let (body, session) = client().request_body(&r).unwrap();
    assert!(session.is_none());
    assert_eq!(body["session"], format!("{ENGINE}/sessions/-"));
    assert!(body.get("generationSpec").is_none());
    assert!(body.get("toolsSpec").is_none());
    r.web_grounding = true;
    assert_eq!(
        client().request_body(&r).unwrap().0["toolsSpec"],
        json!({"webGroundingSpec":{}})
    );
}

#[test]
fn foreign_resources_and_path_injections_are_rejected() {
    let c = client();
    for id in [
        "",
        "-",
        "../id",
        "a/b",
        "a%2fb",
        "a?x",
        "a#x",
        "https://bad/",
        " a",
        "a\n",
    ] {
        assert!(c.session_resource(id).is_err(), "{id:?}");
        assert!(c.datastore_resource(id).is_err(), "{id:?}");
    }
    assert!(c
        .session_resource(&SESSION.replace("/123/", "/999/"))
        .is_err());
    assert!(c
        .session_resource(&SESSION.replace("/search/", "/other/"))
        .is_err());
    assert_eq!(c.session_resource(SESSION).unwrap(), SESSION);
}

#[test]
fn validate_config_and_question_before_dispatch() {
    for config in [
        GeminiEnterpriseConfig::new("project-id", "us", "search"),
        GeminiEnterpriseConfig::new("123", "us/bad", "search"),
        GeminiEnterpriseConfig::new("123", "us", "../search"),
    ] {
        assert!(
            GeminiEnterpriseClient::new(Arc::new(StaticTokenProvider::new("x")), config).is_err()
        );
    }
    for question in [" ".into(), "x".repeat(256 * 1024 + 1)] {
        assert!(client()
            .request_body(&AssistRequest::new(question))
            .is_err());
    }
    let mut r = AssistRequest::new("q");
    for model in ["", " bad", "bad\n"] {
        r.model_id = Some(model.into());
        assert!(client().request_body(&r).is_err());
    }
}

#[test]
fn explicit_base_override_is_versioned_and_safe() {
    for base in [
        "http://remote.test/v1",
        "https://user:pass@remote.test/v1",
        "https://remote.test/v1?token=x",
        "https://remote.test/v1alpha",
        "https://remote.test/v1#x",
    ] {
        assert!(client().with_base_url(base).is_err(), "{base}");
    }
    assert!(client().with_base_url("http://127.0.0.1:123/v1").is_ok());
    assert_eq!(
        client().base_url.as_str(),
        "https://us-discoveryengine.googleapis.com/v1/"
    );
}

#[test]
fn terminal_success_preserves_grounding_excludes_thought_and_tracks_requested_model() {
    let r = parse(frames()).unwrap();
    assert_eq!(r.answer, "答え evidence");
    assert_eq!(r.session, SESSION);
    assert_eq!(r.requested_model.as_deref(), Some("gemini-4-argon"));
    assert_eq!(r.native_reference_count(), 1);
    assert_eq!(r.grounding[0].segments[0]["referenceIndices"], json!([0]));
    assert_eq!(r.invocation_tools, vec!["search"]);
    assert!(r.warnings.is_empty());
}

#[test]
fn answer_prose_is_never_promoted_to_native_citations() {
    let r = parse(json!([{"answer":{"name":format!("{SESSION}/assistAnswers/789"),"state":"SUCCEEDED","replies":[{"groundedContent":{"content":{"text":"Native Citation: made-up source"}}}]},"sessionInfo":{"session":SESSION}}])).unwrap();
    assert_eq!(r.native_reference_count(), 0);
    assert_eq!(r.warnings.len(), 1);
}

#[test]
fn partial_failed_skipped_unknown_or_mismatched_results_never_succeed() {
    let original = frames();
    for state in ["IN_PROGRESS", "FAILED", "SKIPPED", "CANCELLED", "NEW_STATE"] {
        let mut f = original.clone();
        f[2]["answer"]["state"] = json!(state);
        assert!(parse(f).is_err(), "{state}");
    }
    for field in ["name", "state"] {
        let mut f = original.clone();
        f[2]["answer"].as_object_mut().unwrap().remove(field);
        assert!(parse(f).is_err());
    }
    for replacement in [
        SESSION.replace("/123/", "/999/"),
        format!("{ENGINE}/sessions/999"),
    ] {
        let mut f = original.clone();
        f[2]["sessionInfo"]["session"] = json!(replacement);
        assert!(parse(f).is_err());
    }
    let mut f = original.clone();
    f[2]["answer"]["name"] = json!(format!("{ENGINE}/sessions/other/assistAnswers/789"));
    assert!(parse(f).is_err());
    let mut f = original.clone();
    f.as_array_mut().unwrap().push(original[0].clone());
    assert!(parse(f).is_err());
    let mut f = original;
    f[2]["error"] = json!({"message":"sensitive"});
    assert!(parse(f).is_err());
    for bytes in [b"[".as_slice(), b"[]", b"{}", b"[{}]", b"[]trailing"] {
        assert!(client().parse_answer(bytes, None, None).is_err());
    }
}

#[tokio::test]
async fn official_post_uses_oauth_quota_header_and_does_not_preflight_or_list() {
    let server = MockServer::start().await;
    let c = client()
        .with_base_url(&format!("{}/v1", server.uri()))
        .unwrap();
    let request = AssistRequest::new("q");
    let expected = c.request_body(&request).unwrap().0;
    Mock::given(method("POST"))
        .and(path(PATH))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("x-goog-user-project", "123"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(frames()))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(c.ask(request).await.unwrap().session, SESSION);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn no_generation_retries_on_auth_throttling_server_error_or_redirect() {
    for status in [401, 403, 429, 500, 503, 307] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(PATH))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Location", format!("{}/elsewhere", server.uri()))
                    .set_body_json(json!({"error":{"message":"secret-token sensitive question"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let c = client()
            .with_base_url(&format!("{}/v1", server.uri()))
            .unwrap();
        let error = c
            .ask(AssistRequest::new("private question"))
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret-token"));
        assert!(!error.contains("sensitive question"));
        assert!(error.contains("do not automatically repeat"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn bounded_responses_and_incomplete_json_fail_without_retry() {
    for body in ["[".to_string(), "x".repeat(MAX_RESPONSE_BYTES + 1)] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let c = client()
            .with_base_url(&format!("{}/v1", server.uri()))
            .unwrap();
        assert!(c.ask(AssistRequest::new("q")).await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn saved_session_read_returns_persisted_model_metadata_and_can_include_answers() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/v1/{SESSION}"))).and(query_param("includeAnswerDetails","true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":SESSION,"turns":[{"query":{"text":"q"},"queryConfig":{"google.discoveryengine.googleapis.com.Assistant.model_id":"gemini-4-argon"},"detailedAssistAnswer":{"state":"SUCCEEDED"}}]})))
        .expect(1).mount(&server).await;
    let c = client()
        .with_base_url(&format!("{}/v1", server.uri()))
        .unwrap();
    let session = c.get_session("456", true).await.unwrap();
    assert_eq!(session.turns[0].model_id(), Some("gemini-4-argon"));
    assert!(session.turns[0].detailed_assist_answer.is_some());
}

#[test]
fn invalid_native_reference_indices_and_unicode_offsets_fail_closed() {
    for segment in [
        Value::Null,
        json!({"referenceIndices":[1],"endIndex":"6"}),
        json!({"referenceIndices":[-1],"endIndex":"6"}),
        json!({"referenceIndices":[0],"startIndex":"1","endIndex":"6"}),
        json!({"referenceIndices":[0],"endIndex":"999"}),
        json!({"referenceIndices":[0],"endIndex":"6","text":"not the returned segment"}),
    ] {
        let mut f = frames();
        f[2]["answer"]["replies"][0]["groundedContent"]["textGroundingMetadata"]["segments"] =
            json!([segment]);
        assert!(parse(f).is_err());
    }
}

#[test]
fn frame_budget_is_enforced_during_deserialization() {
    let frames = json!(vec![json!({}); MAX_FRAMES + 1]);
    assert!(parse(frames).is_err());
}

#[test]
fn skipped_small_talk_reports_reason_instead_of_a_generic_failure() {
    // Captured live for the query "Hello": no answer name, replies or session.
    let frames = json!([{"answer":{"state":"SKIPPED","assistSkippedReasons":["NON_ASSIST_SEEKING_QUERY_IGNORED"]},"assistToken":"t"}]);
    match parse(frames) {
        Err(Error::Skipped { reasons, session }) => {
            assert_eq!(reasons, ["NON_ASSIST_SEEKING_QUERY_IGNORED"]);
            assert!(session.is_none());
        }
        other => panic!("expected Skipped, got {:?}", other.map(|r| r.answer)),
    }
    let message = parse(json!([{"answer":{"state":"SKIPPED","assistSkippedReasons":["NON_ASSIST_SEEKING_QUERY_IGNORED"]}}]))
        .unwrap_err()
        .to_string();
    assert!(message.contains("ignores greetings"), "{message}");
    assert!(!message.contains("may have recorded"), "{message}");
    // A skip without reasons is still a skip, not a success.
    assert!(matches!(
        parse(json!([{"answer":{"state":"SKIPPED"}}])),
        Err(Error::Skipped { .. })
    ));
}

fn grounded(text: &str, segments: Value) -> Value {
    json!({"groundedContent":{
        "content":{"text":text},
        "textGroundingMetadata":{
            "references":[{"documentMetadata":{"uri":"https://example.test/doc","title":"Doc"}}],
            "segments":segments}}})
}

fn answer_frame(replies: Value) -> Value {
    json!([{"answer":{"state":"SUCCEEDED","name":format!("{SESSION}/assistAnswers/789"),"replies":replies},
            "sessionInfo":{"session":SESSION}}])
}

#[test]
fn fragment_relative_grounding_offsets_are_validated_locally_and_rebased() {
    // Two fragments, each carrying metadata whose offsets are relative to that fragment.
    let replies = json!([
        grounded(
            "First part. ",
            json!([{"startIndex":"0","endIndex":"10","referenceIndices":[0],"text":"First part"}])
        ),
        grounded(
            "Second part.",
            json!([{"startIndex":"0","endIndex":"11","referenceIndices":[0],"text":"Second part"}])
        ),
    ]);
    let r = parse(answer_frame(replies)).unwrap();
    assert_eq!(r.answer, "First part. Second part.");
    assert_eq!(r.grounding.len(), 2);
    // After parsing, offsets point into the whole answer and keep the string type.
    let second = &r.grounding[1].segments[0];
    assert_eq!(second["startIndex"], "12");
    assert_eq!(second["endIndex"], "23");
    assert_eq!(&r.answer[12..23], "Second part");
    assert_eq!(r.grounding[0].segments[0]["startIndex"], "0");
}

#[test]
fn standalone_grounding_with_whole_answer_offsets_still_passes() {
    // Observed live shape: text fragments first, then one metadata-only reply whose
    // offsets are relative to the whole answer.
    let replies = json!([
        {"groundedContent":{"content":{"text":"Alpha. "}}},
        {"groundedContent":{"content":{"text":"Beta gamma."}}},
        {"groundedContent":{"textGroundingMetadata":{
            "references":[{"documentMetadata":{"uri":"https://example.test/doc"}}],
            "segments":[{"startIndex":"7","endIndex":"17","referenceIndices":[0],"text":"Beta gamma"},
                        {"startIndex":"0","endIndex":"5","referenceIndices":[0],"text":"Alpha"}]}}},
    ]);
    let r = parse(answer_frame(replies)).unwrap();
    assert_eq!(r.grounding[0].segments[0]["startIndex"], "7");
    assert_eq!(r.native_reference_count(), 1);
}

#[test]
fn grounding_offsets_valid_in_no_base_are_rejected() {
    let replies = json!([grounded(
        "Short.",
        json!([{"startIndex":"0","endIndex":"40","referenceIndices":[0]}])
    ),]);
    assert!(parse(answer_frame(replies)).is_err());
    // Quoted text that matches neither the fragment nor the whole answer.
    let replies = json!([
        {"groundedContent":{"content":{"text":"One. "}}},
        grounded("Two.", json!([{"startIndex":"0","endIndex":"3","referenceIndices":[0],"text":"Six"}])),
    ]);
    assert!(parse(answer_frame(replies)).is_err());
}

#[test]
fn connector_auth_errors_are_warnings_when_the_answer_succeeds() {
    // One connector fails authentication; the assistant still completes from others.
    let mut f = frames();
    f[1]["connectorAuthErrors"] = json!([{"error":"private-details","dataStore":"secret-store"}]);
    let r = parse(f).unwrap();
    assert_eq!(r.answer, "答え evidence");
    assert_eq!(r.native_reference_count(), 1);
    let warning = r
        .warnings
        .iter()
        .find(|w| w.contains("connector authentication"))
        .expect("connector warning");
    assert!(warning.starts_with("1 connector"), "{warning}");
    assert!(!warning.contains("private-details") && !warning.contains("secret-store"));
}

#[test]
fn connector_auth_errors_without_an_answer_explain_the_failure() {
    let mut f = frames();
    f[2]["connectorAuthErrors"] = json!([{"error":"private-details"}, {"error":"more"}]);
    f[2]["answer"]["state"] = json!("FAILED");
    let error = parse(f).unwrap_err().to_string();
    assert!(
        error.contains("2 connector authentication error(s)"),
        "{error}"
    );
    assert!(error.contains("do not automatically repeat"));
    assert!(!error.contains("private-details"));
    // A native API error stays fatal even when the answer would otherwise succeed.
    let mut f = frames();
    f[2]["error"] = json!({"message":"sensitive"});
    let error = parse(f).unwrap_err().to_string();
    assert!(!error.contains("sensitive"));
}
