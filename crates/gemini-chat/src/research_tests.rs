use super::*;
use crate::StaticTokenProvider;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ENGINE: &str = "projects/123/locations/us/collections/default_collection/engines/search";
const SESSION: &str =
    "projects/123/locations/us/collections/default_collection/engines/search/sessions/456";

fn client() -> GeminiEnterpriseClient {
    GeminiEnterpriseClient::new(
        Arc::new(StaticTokenProvider::new("synthetic")),
        GeminiEnterpriseConfig::new("123", "us", "search"),
    )
    .unwrap()
}
fn capture(marked: bool) -> Value {
    // Synthetic reproduction of the observed streamed blocks + terminal metadata
    // snapshot. Only synthetic content/token material; no tenant or browser state.
    let meta = json!({"references":[{"documentMetadata":{"uri":"https://example.test/doc"}}],"segments":[{"startIndex":"0","endIndex":"6","text":"Report","referenceIndices":[0]}]});
    let mut report = json!({"content":{"text":"Report"},"textGroundingMetadata":meta});
    if marked {
        report["contentMetadata"] = json!({"contentKind":"RESEARCH_REPORT"});
    }
    let mut response = json!([
        {"sessionInfo":{"session":SESSION,"assistToken":"synthetic-sensitive-token"},"answer":{"state":"IN_PROGRESS","replies":[{"groundedContent":{"content":{"text":"Intermediate"}}}]}},
        {"answer":{"state":"IN_PROGRESS","replies":[{"groundedContent":report}]}},
        {"answer":{"state":"IN_PROGRESS","replies":[{"groundedContent":{"content":{"file":{"fileId":"1234","mimeType":"audio/mp3"}}}}]}},
        {"answer":{"state":"SUCCEEDED","name":format!("{SESSION}/assistAnswers/789"),"replies":[{"groundedContent":{}},{"groundedContent":{"textGroundingMetadata":meta}},{"groundedContent":{}}]}}
    ]);
    if marked {
        response[1]["answer"]["replies"][0]["agent"] = json!("deep_research");
    }
    response
}
fn parse(value: Value) -> Result<ResearchResponse> {
    client().parse_research(
        &serde_json::to_vec(&value).unwrap(),
        None,
        ResearchPhase::Start,
    )
}

#[test]
fn public_response_preserves_blocks_without_inventing_a_report_or_agent_attestation() {
    let r = parse(capture(false)).unwrap();
    assert!(r.report_indices.is_empty());
    assert_eq!(r.final_text_candidate_index, Some(1));
    assert_eq!(r.contents.len(), 3);
    assert_eq!(r.contents[0].text.as_deref(), Some("Intermediate"));
    assert_eq!(r.contents[1].text.as_deref(), Some("Report"));
    assert!(r.contents[1].kind.is_none());
    assert_eq!(r.contents[1].grounding.len(), 1); // terminal repeat not double counted
    assert_eq!(r.contents[1].grounding[0].segments[0]["startIndex"], "0");
    assert_eq!(r.contents[2].file.as_ref().unwrap().mime_type, "audio/mp3");
    assert!(r.warnings.iter().any(|w| w.contains("unverified")));
    let encoded = serde_json::to_string(&r).unwrap();
    assert!(!encoded.contains("assistToken"));
    assert!(!encoded.contains("synthetic-sensitive-token"));
}

#[test]
fn explicit_stage_metadata_not_text_shape_identifies_report() {
    let r = parse(capture(true)).unwrap();
    assert_eq!(r.report_indices, vec![1]);
    assert_eq!(r.final_text_candidate_index, None);
    assert_eq!(r.contents[1].agent.as_deref(), Some("deep_research"));
    assert!(r.warnings.is_empty());
}

#[test]
fn incomplete_failed_foreign_and_invalid_citations_fail_closed() {
    for state in ["IN_PROGRESS", "FAILED", "CANCELLED", "SURPRISE"] {
        let mut v = capture(false);
        v[3]["answer"]["state"] = json!(state);
        assert!(parse(v).is_err(), "{state}");
    }
    let mut v = capture(false);
    v[3]["answer"]["name"] = json!(format!("{ENGINE}/sessions/other/assistAnswers/789"));
    assert!(parse(v).is_err());
    for field in ["endIndex", "referenceIndices"] {
        let mut v = capture(false);
        let segment = &mut v[1]["answer"]["replies"][0]["groundedContent"]["textGroundingMetadata"]
            ["segments"][0];
        segment[field] = if field == "endIndex" {
            json!("999")
        } else {
            json!([1])
        };
        assert!(parse(v).is_err());
    }
    assert!(client()
        .parse_research(b"[]", None, ResearchPhase::Plan)
        .is_err());
    assert!(client()
        .parse_research(b"[{", None, ResearchPhase::Plan)
        .is_err());
    let mut v = capture(false);
    v[3]["answer"]["replies"].as_array_mut().unwrap().pop();
    assert!(parse(v).is_err()); // metadata can't be assigned by guesswork
}

#[tokio::test]
async fn plan_and_start_dispatch_exact_agent_array_sources_and_session_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/{ENGINE}/assistants/default_assistant:streamAssist"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(capture(false)))
        .expect(2)
        .mount(&server)
        .await;
    let c = client()
        .with_base_url(&format!("{}/v1", server.uri()))
        .unwrap();
    let mut plan = ResearchPlanRequest::new("Compare CLI implementations; plan only.");
    plan.data_stores = vec!["sp_file".into()];
    let planned = c.plan_research(plan).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1); // never auto-start
    let mut start = ResearchStartRequest::new(planned.session);
    start.data_stores = vec!["sp_file".into()];
    start.web_grounding = true;
    c.start_research(start).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    for r in &requests {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(
            body["agentsSpec"],
            json!({"agentSpecs":[{"agentId":"deep_research"}]})
        );
        assert!(body.get("generationSpec").is_none());
        assert_eq!(
            body["toolsSpec"]["vertexAiSearchSpec"]["dataStoreSpecs"],
            json!([{"dataStore":"projects/123/locations/us/collections/default_collection/dataStores/sp_file"}])
        );
        assert!(r.headers.get("cookie").is_none());
    }
    let first: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(first["session"], format!("{ENGINE}/sessions/-"));
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(second["query"]["text"], "Start Research");
    assert_eq!(second["session"], SESSION);
    assert_eq!(second["toolsSpec"]["webGroundingSpec"], json!({}));
}

#[tokio::test]
async fn validation_before_network_and_no_retry_on_failure() {
    let server = MockServer::start().await;
    let c = client()
        .with_base_url(&format!("{}/v1", server.uri()))
        .unwrap();
    assert!(c
        .plan_research(ResearchPlanRequest::new("  "))
        .await
        .is_err());
    for session in [
        "-",
        "../id",
        "projects/999/locations/us/collections/default_collection/engines/search/sessions/1",
    ] {
        assert!(c
            .start_research(ResearchStartRequest::new(session))
            .await
            .is_err());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let err = c
        .start_research(ResearchStartRequest::new("456"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Assist {
            session: Some(_),
            ..
        }
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn research_deadline_overrides_short_chat_deadline() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(capture(false))
                .set_delay(Duration::from_millis(100)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut config = GeminiEnterpriseConfig::new("123", "us", "search");
    assert_eq!(config.research_timeout, Duration::from_secs(1800));
    config.timeout = Duration::from_millis(1);
    config.research_timeout = Duration::from_secs(3);
    let c = GeminiEnterpriseClient::new(Arc::new(StaticTokenProvider::new("synthetic")), config)
        .unwrap()
        .with_base_url(&format!("{}/v1", server.uri()))
        .unwrap();
    c.start_research(ResearchStartRequest::new("456"))
        .await
        .unwrap();
}

/// Opt-in local replay; never check private captures into source control.
#[test]
#[ignore = "requires GEMINI_CHAT_RESEARCH_CAPTURE pointing to a private streamAssist JSON capture"]
fn replay_private_research_capture() {
    let path = std::env::var("GEMINI_CHAT_RESEARCH_CAPTURE").expect("set capture path");
    let bytes = std::fs::read(path).unwrap();
    // Use this capture's own configured project/engine, without printing its content.
    let config = GeminiEnterpriseConfig::new(
        std::env::var("GEMINI_CHAT_PROJECT_NUMBER").unwrap(),
        std::env::var("GEMINI_CHAT_LOCATION").unwrap(),
        std::env::var("GEMINI_CHAT_ENGINE_ID").unwrap(),
    );
    let c =
        GeminiEnterpriseClient::new(Arc::new(StaticTokenProvider::new("unused")), config).unwrap();
    let result = c
        .parse_research(&bytes, None, ResearchPhase::Start)
        .unwrap();
    assert!(result.contents.len() >= 3);
    assert!(result.contents.iter().any(|b| b.file.is_some()));
    assert!(result.contents.iter().any(|b| !b.grounding.is_empty()));
}

#[test]
fn terminal_snapshot_can_supply_missing_labels_but_not_conflicting_labels() {
    let mut v = capture(false);
    v[3]["answer"]["replies"][1]["groundedContent"]["contentMetadata"] =
        json!({"contentKind":"RESEARCH_REPORT"});
    let r = parse(v).unwrap();
    assert_eq!(r.report_indices, vec![1]);
    let mut v = capture(true);
    v[3]["answer"]["replies"][1]["groundedContent"]["contentMetadata"] =
        json!({"contentKind":"RESEARCH_ANSWER"});
    assert!(parse(v).is_err());
}

#[test]
fn hidden_thoughts_and_terminal_metadata_are_never_returned() {
    let mut v = capture(false);
    v[0]["answer"]["replies"].as_array_mut().unwrap().insert(0, json!({"groundedContent":{"content":{"text":"private hidden reasoning","thought":true},"contentMetadata":{"contentKind":"SECRET"}}}));
    v[3]["answer"]["replies"].as_array_mut().unwrap().insert(0, json!({"groundedContent":{"contentMetadata":{"contentKind":"SECRET"},"textGroundingMetadata":{"references":[{"content":"private hidden grounding"}]}}}));
    let r = parse(v).unwrap();
    assert_eq!(r.contents.len(), 3);
    let encoded = serde_json::to_string(&r).unwrap();
    assert!(!encoded.contains("private hidden"));
    assert!(!encoded.contains("SECRET"));
}

#[test]
fn research_timeout_must_be_positive() {
    let mut config = GeminiEnterpriseConfig::new("123", "us", "search");
    config.research_timeout = Duration::ZERO;
    assert!(
        GeminiEnterpriseClient::new(Arc::new(StaticTokenProvider::new("unused")), config).is_err()
    );
}

#[test]
fn native_minus_one_markers_are_quarantined_not_cited_or_counted_twice() {
    let mut v = capture(false);
    for frame in [1, 3] {
        let reply = if frame == 1 { 0 } else { 1 };
        let meta =
            &mut v[frame]["answer"]["replies"][reply]["groundedContent"]["textGroundingMetadata"];
        meta["segments"].as_array_mut().unwrap().push(
            json!({"startIndex":"0","endIndex":"6","text":"Report","referenceIndices":[-1,0]}),
        );
    }
    let r = parse(v).unwrap();
    let b = &r.contents[1];
    assert_eq!(b.grounding.len(), 1);
    assert_eq!(b.grounding[0].segments.len(), 1);
    assert_eq!(b.unattributed_segments.len(), 1);
    assert_eq!(b.unattributed_segments[0].grounding_batch, 0);
    assert_eq!(
        b.unattributed_segments[0].segment["referenceIndices"],
        json!([-1, 0])
    );
    assert!(r
        .warnings
        .iter()
        .any(|w| w.contains("NOT exposed as validated citations")));
    for indices in [json!([-2]), json!([-1, 1]), json!([-1, "0"])] {
        let mut v = capture(false);
        v[1]["answer"]["replies"][0]["groundedContent"]["textGroundingMetadata"]["segments"][0]
            ["referenceIndices"] = indices;
        assert!(parse(v).is_err());
    }
    let mut v = capture(false);
    v[1]["answer"]["replies"][0]["groundedContent"]["textGroundingMetadata"]["segments"][0] =
        json!({"startIndex":"0","endIndex":"999","referenceIndices":[-1]});
    assert!(parse(v).is_err());
}

#[tokio::test]
async fn recovery_reads_latest_answer_without_any_generation_or_agent_claim() {
    let server = MockServer::start().await;
    let v = capture(false);
    let answer = json!({"state":"SUCCEEDED","name":format!("{SESSION}/assistAnswers/789"),"replies":[v[0]["answer"]["replies"][0],v[1]["answer"]["replies"][0],v[2]["answer"]["replies"][0]]});
    Mock::given(method("GET"))
        .and(path(format!("/v1/{SESSION}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"name":SESSION,"turns":[{"detailedAssistAnswer":answer}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client()
        .with_base_url(&format!("{}/v1", server.uri()))
        .unwrap();
    let r = c.get_research("456").await.unwrap();
    assert_eq!(r.requested_phase, ResearchPhase::Read);
    assert!(r.requested_agent.is_none());
    assert_eq!(r.contents.len(), 3);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), Some("includeAnswerDetails=true"));
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"name":SESSION,"turns":[{"detailedAssistAnswer":{"state":"IN_PROGRESS"}}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(c.get_research("456").await, Err(Error::Api(_))));
}
