//! Official Gemini Enterprise v1 `streamAssist` and `sessions.get`.
//!
//! Uses OAuth only: no browser cookies, automatic
//! POST retries, model fallback, connector mutations, or client-supplied functions.
//! Responses are bounded and buffered to EOF before a named terminal SUCCEEDED
//! answer is returned; a partial stream is never presented as a completed answer.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::{redirect::Policy, Client, Method, Url};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{models::*, tls::TlsRoots, Error, Result, TokenProvider};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAMES: usize = 10_000;

#[derive(Debug, Clone)]
pub struct GeminiEnterpriseConfig {
    pub project_number: String,
    pub location: String,
    pub engine_id: String,
    pub collection_id: String,
    pub assistant_id: String,
    pub timeout: Duration,
}

impl GeminiEnterpriseConfig {
    pub fn new(
        project_number: impl Into<String>,
        location: impl Into<String>,
        engine_id: impl Into<String>,
    ) -> Self {
        Self {
            project_number: project_number.into(),
            location: location.into(),
            engine_id: engine_id.into(),
            collection_id: "default_collection".into(),
            assistant_id: "default_assistant".into(),
            timeout: Duration::from_secs(360),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.project_number.is_empty()
            || self.project_number.len() > 32
            || !self.project_number.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Error::validation(
                "enterprise project_number must be numeric",
            ));
        }
        for (name, value) in [
            ("location", &self.location),
            ("engine_id", &self.engine_id),
            ("collection_id", &self.collection_id),
            ("assistant_id", &self.assistant_id),
        ] {
            validate_segment(value, name)?;
        }
        if !SUPPORTED_LOCATIONS.contains(&self.location.as_str()) {
            return Err(Error::validation(format!(
                "unsupported enterprise location {:?}; use one of: {}",
                self.location,
                SUPPORTED_LOCATIONS.join(", ")
            )));
        }
        if self.timeout.is_zero() {
            return Err(Error::validation("enterprise timeout must be positive"));
        }
        Ok(())
    }

    fn collection(&self) -> String {
        format!(
            "projects/{}/locations/{}/collections/{}",
            self.project_number, self.location, self.collection_id
        )
    }

    fn engine(&self) -> String {
        format!("{}/engines/{}", self.collection(), self.engine_id)
    }
}

/// Independent official client, reusing the SDK's OAuth token providers.
#[derive(Clone)]
pub struct GeminiEnterpriseClient {
    http: Client,
    provider: Arc<dyn TokenProvider>,
    config: GeminiEnterpriseConfig,
    base_url: Url,
    /// Trusted provenance for folder sessions created by this client instance.
    /// Clones share it; a fresh process intentionally starts empty.
    folder_sessions: Arc<Mutex<HashMap<String, (String, String)>>>,
}

impl GeminiEnterpriseClient {
    pub fn new(provider: Arc<dyn TokenProvider>, config: GeminiEnterpriseConfig) -> Result<Self> {
        config.validate()?;
        let http = TlsRoots::from_env()?
            .apply(Client::builder())
            .user_agent(concat!("gemini-chat/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .timeout(config.timeout)
            .build()?;
        let base_url = Url::parse(&endpoint(&config.location))?;
        Ok(Self {
            http,
            provider,
            config,
            base_url,
            folder_sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Explicit test/proxy override. No userinfo, query, fragment, or insecure remote HTTP.
    /// Include `/v1` in the base URL. Never uses the NotebookLM v1alpha base URL.
    pub fn with_base_url(mut self, base: &str) -> Result<Self> {
        let url = Url::parse(&format!("{}/", base.trim_end_matches('/')))?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/v1/"
        {
            return Err(Error::validation("enterprise API base must be an HTTPS /v1 URL (local HTTP allowed for tests), without credentials/query/fragment"));
        }
        self.base_url = url;
        Ok(self)
    }

    /// Generate once. No HTTP, auth-refresh, model-fallback, or application retry.
    /// On any post-dispatch failure the server may have recorded the turn.
    pub async fn ask(&self, request: AssistRequest) -> Result<AssistResponse> {
        let (body, requested_session) = self.request_body(&request)?;
        // Token acquisition is pre-dispatch and may fail normally. Never log the token/body.
        let token = self.provider.access_token().await?;
        let resource = format!(
            "{}/assistants/{}:streamAssist",
            self.config.engine(),
            self.config.assistant_id
        );
        let url = self.base_url.join(&resource)?;
        let result = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("x-goog-user-project", &self.config.project_number)
            .json(&body)
            .send()
            .await;
        let response = result.map_err(|_| {
            assist_error(
                "transport failure; delivery is uncertain",
                requested_session.clone(),
            )
        })?;
        let status = response.status();
        let bytes = bounded_body(response)
            .await
            .map_err(|reason| assist_error(reason, requested_session.clone()))?;
        if !status.is_success() {
            return Err(assist_error(
                http_reason(status.as_u16(), &bytes),
                requested_session,
            ));
        }
        self.parse_answer(&bytes, request.model_id, requested_session)
    }

    /// Read a saved server session. Does not generate or continue a conversation.
    pub async fn get_session(
        &self,
        session: &str,
        include_answer_details: bool,
    ) -> Result<AssistSession> {
        let resource = self.session_resource(session)?;
        let mut url = self.base_url.join(&resource)?;
        if include_answer_details {
            url.query_pairs_mut()
                .append_pair("includeAnswerDetails", "true");
        }
        let token = self.provider.access_token().await?;
        let response = self
            .http
            .request(Method::GET, url)
            .bearer_auth(token)
            .header("x-goog-user-project", &self.config.project_number)
            .send()
            .await
            .map_err(|_| Error::Api("session read transport failure".into()))?;
        let status = response.status();
        let bytes = bounded_body(response).await.map_err(Error::Api)?;
        if !status.is_success() {
            return Err(Error::Api(http_reason(status.as_u16(), &bytes)));
        }
        let session: AssistSession = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Api("invalid session JSON/schema".into()))?;
        if session.name != resource {
            return Err(Error::Api("session resource mismatch".into()));
        }
        Ok(session)
    }

    fn session_resource(&self, value: &str) -> Result<String> {
        let prefix = format!("{}/sessions/", self.config.engine());
        let id = value.strip_prefix(&prefix).unwrap_or(value);
        validate_segment(id, "session")?;
        Ok(format!("{prefix}{id}"))
    }

    fn datastore_resource(&self, value: &str) -> Result<String> {
        let prefix = format!("{}/dataStores/", self.config.collection());
        let id = value.strip_prefix(&prefix).unwrap_or(value);
        validate_segment(id, "datastore")?;
        Ok(format!("{prefix}{id}"))
    }

    fn require_folder_session_binding(
        &self,
        session: &str,
        folder: &str,
        datastore: &str,
    ) -> Result<()> {
        let bindings = self
            .folder_sessions
            .lock()
            .map_err(|_| Error::Api("folder session registry unavailable".into()))?;
        match bindings.get(session) {
            Some((bound_folder, bound_datastore))
                if bound_folder == folder && bound_datastore == datastore =>
            {
                Ok(())
            }
            Some(_) => Err(Error::validation(
                "cannot continue folder session in a different folder or datastore; start a new folder session",
            )),
            None => Err(Error::validation(
                "cannot continue folder session: it was not created by this client instance; start a new folder session",
            )),
        }
    }

    fn remember_folder_session_binding(
        &self,
        session: &str,
        folder: &str,
        datastore: &str,
    ) -> Result<()> {
        let mut bindings = self
            .folder_sessions
            .lock()
            .map_err(|_| Error::Api("folder session registry unavailable".into()))?;
        match bindings.get(session) {
            Some((bound_folder, bound_datastore))
                if bound_folder == folder && bound_datastore == datastore =>
            {
                Ok(())
            }
            Some(_) => Err(Error::Api(
                "returned folder session is already bound to a different folder or datastore"
                    .into(),
            )),
            None => {
                bindings.insert(
                    session.to_string(),
                    (folder.to_string(), datastore.to_string()),
                );
                Ok(())
            }
        }
    }

    fn request_body(&self, request: &AssistRequest) -> Result<(Value, Option<String>)> {
        if request.question.trim().is_empty() || request.question.len() > 256 * 1024 {
            return Err(Error::validation(
                "enterprise question must be nonempty and at most 256 KiB UTF-8",
            ));
        }
        if let Some(model) = &request.model_id {
            if model.is_empty()
                || model.len() > 256
                || model.trim() != model
                || model.chars().any(char::is_control)
            {
                return Err(Error::validation("invalid enterprise model_id"));
            }
        }
        if request.data_stores.len() > 100 {
            return Err(Error::validation(
                "at most 100 enterprise datastores are allowed",
            ));
        }
        let requested_session = request
            .session
            .as_deref()
            .map(|s| self.session_resource(s))
            .transpose()?;
        let mut body = json!({
            "query": {"text": request.question},
            "session": requested_session.clone().unwrap_or_else(|| format!("{}/sessions/-", self.config.engine()))
        });
        if let Some(model) = &request.model_id {
            body["generationSpec"] = json!({"modelId": model});
        }
        let mut tools = serde_json::Map::new();
        if !request.data_stores.is_empty() {
            let stores = request
                .data_stores
                .iter()
                .map(|s| {
                    self.datastore_resource(s)
                        .map(|name| json!({"dataStore":name}))
                })
                .collect::<Result<Vec<_>>>()?;
            tools.insert(
                "vertexAiSearchSpec".into(),
                json!({"dataStoreSpecs": stores}),
            );
        }
        if request.web_grounding {
            tools.insert("webGroundingSpec".into(), json!({}));
        }
        if !tools.is_empty() {
            body["toolsSpec"] = Value::Object(tools);
        }
        Ok((body, requested_session))
    }

    fn parse_answer(
        &self,
        bytes: &[u8],
        model: Option<String>,
        requested_session: Option<String>,
    ) -> Result<AssistResponse> {
        let Frames(frames) = serde_json::from_slice(bytes).map_err(|_| {
            assist_error(
                "invalid or incomplete JSON response/schema",
                requested_session.clone(),
            )
        })?;
        if frames.is_empty() || frames.len() > MAX_FRAMES {
            return Err(assist_error(
                "empty or over-limit frame array",
                requested_session,
            ));
        }
        let mut session = requested_session;
        let mut text = String::new();
        // Each grounding block with the coordinate bases its offsets may be relative to.
        let mut pending_grounding: Vec<(TextGroundingMetadata, Vec<OffsetBase>)> = Vec::new();
        let mut last_grounding_end = 0;
        // Per-connector auth failures are not fatal: the assistant may still answer from
        // other sources. Only their count is kept; the payload may contain private details.
        let mut connector_auth_failures = 0usize;
        let mut tools = Vec::new();
        let mut terminal = None;
        let mut skipped_reasons: Vec<String> = Vec::new();
        for frame in frames {
            if let Some(info) = frame.session_info {
                let canonical = self.session_resource(&info.session).map_err(|_| {
                    assist_error("invalid returned session resource", session.clone())
                })?;
                if canonical != info.session || session.as_ref().is_some_and(|s| s != &canonical) {
                    return Err(assist_error("returned session resource mismatch", session));
                }
                session = Some(canonical);
            }
            connector_auth_failures += frame.connector_auth_errors.len();
            if frame.error.is_some() {
                return Err(assist_error(
                    with_connector_hint("native API error", connector_auth_failures),
                    session,
                ));
            }
            for tool in frame.invocation_tools {
                if !tools.contains(&tool) {
                    tools.push(tool);
                }
            }
            if let Some(answer) = frame.answer {
                if terminal.is_some() {
                    return Err(assist_error("answer data after a terminal state", session));
                }
                for reply in answer.replies {
                    let mut fragment = None;
                    if let Some(content) = reply.grounded_content.content {
                        if !content.thought {
                            let start = text.len();
                            text.push_str(&content.text.unwrap_or_default());
                            fragment = Some(OffsetBase::Range(start, text.len()));
                        }
                    }
                    if let Some(meta) = reply.grounded_content.text_grounding_metadata {
                        // Preserve per-batch indices rather than silently flattening distinct
                        // references. Metadata attached to a text fragment is tried against
                        // that fragment first; standalone metadata (the observed live shape)
                        // against the whole answer first, then the text since the last block.
                        let bases = match fragment {
                            Some(own) => vec![own, OffsetBase::Whole],
                            None => vec![
                                OffsetBase::Whole,
                                OffsetBase::Range(last_grounding_end, text.len()),
                            ],
                        };
                        pending_grounding.push((meta, bases));
                        last_grounding_end = text.len();
                    }
                }
                match answer.state.as_deref() {
                    Some("SUCCEEDED" | "FAILED" | "SKIPPED" | "CANCELLED") => {
                        terminal = Some((answer.state.unwrap(), answer.name));
                        skipped_reasons = answer.skipped_reasons;
                    }
                    Some("IN_PROGRESS") | None => {}
                    _ => return Err(assist_error("unknown answer state", session)),
                }
            }
        }
        if terminal
            .as_ref()
            .is_some_and(|(state, _)| state == "SKIPPED")
        {
            if skipped_reasons.is_empty() {
                skipped_reasons.push("UNSPECIFIED".into());
            }
            return Err(Error::Skipped {
                reasons: skipped_reasons,
                session,
            });
        }
        let Some((state, Some(name))) = terminal else {
            return Err(assist_error(
                with_connector_hint("missing named terminal answer", connector_auth_failures),
                session,
            ));
        };
        if state != "SUCCEEDED" {
            return Err(assist_error(
                with_connector_hint(&format!("terminal state {state}"), connector_auth_failures),
                session,
            ));
        }
        let Some(session) = session else {
            return Err(assist_error("missing session resource", None));
        };
        let prefix = format!("{session}/assistAnswers/");
        let valid_name = name
            .strip_prefix(&prefix)
            .is_some_and(|id| validate_segment(id, "answer").is_ok());
        if !valid_name {
            return Err(assist_error(
                "returned answer resource mismatch",
                Some(session),
            ));
        }
        if text.trim().is_empty() {
            return Err(assist_error("no textual answer", Some(session)));
        }
        let mut grounding = Vec::with_capacity(pending_grounding.len());
        for (mut metadata, bases) in pending_grounding {
            // One base per block, so a block never mixes coordinate systems.
            let base = bases.into_iter().find_map(|base| {
                let (start, end) = match base {
                    OffsetBase::Whole => (0, text.len()),
                    OffsetBase::Range(start, end) => (start, end),
                };
                let slice = text.get(start..end)?;
                metadata
                    .segments
                    .iter()
                    .all(|segment| segment_is_valid(segment, metadata.references.len(), slice))
                    .then_some(start)
            });
            let Some(base) = base else {
                return Err(assist_error(
                    "invalid native grounding indices or UTF-8 byte offsets",
                    Some(session),
                ));
            };
            if base > 0 {
                for segment in &mut metadata.segments {
                    shift_segment(segment, base);
                }
            }
            grounding.push(metadata);
        }
        let mut warnings = Vec::new();
        if connector_auth_failures > 0 {
            warnings.push(format!(
                "{connector_auth_failures} connector authentication error(s) reported; results from those sources may be missing. Re-authorize the connector in Gemini Enterprise if needed."
            ));
        }
        if !grounding.iter().any(|g| !g.references.is_empty()) {
            warnings.push("No native grounding references returned; citations written in answer text are not structured citations.".into());
        }
        Ok(AssistResponse {
            answer: text,
            session,
            answer_resource: name,
            requested_model: model,
            grounding,
            invocation_tools: tools,
            warnings,
        })
    }
}

fn validate_segment(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(Error::validation(format!("invalid enterprise {label}; use a resource ID or a resource in the configured project/location/engine")));
    }
    Ok(())
}

/// Text that a grounding block's segment offsets are measured against.
#[derive(Clone, Copy)]
enum OffsetBase {
    Whole,
    /// Byte range of the aggregated answer text.
    Range(usize, usize),
}

fn segment_offset(segment: &Value, key: &str) -> Option<usize> {
    match segment.get(key) {
        None => Some(0),
        Some(value) => value
            .as_u64()
            .or_else(|| value.as_str()?.parse().ok())
            .and_then(|n| usize::try_from(n).ok()),
    }
}

fn segment_is_valid(segment: &Value, reference_count: usize, text: &str) -> bool {
    let valid_references = segment.get("referenceIndices").is_none_or(|indices| {
        indices.as_array().is_some_and(|indices| {
            indices.iter().all(|index| {
                index
                    .as_u64()
                    .is_some_and(|index| index < reference_count as u64)
            })
        })
    });
    let valid_offsets = segment_offset(segment, "startIndex")
        .zip(segment_offset(segment, "endIndex"))
        .is_some_and(|(start, end)| {
            text.get(start..end).is_some_and(|value| {
                segment
                    .get("text")
                    .is_none_or(|quoted| quoted.as_str() == Some(value))
            })
        });
    segment.is_object() && valid_references && valid_offsets
}

/// Rebase a validated segment's offsets onto the whole answer, keeping each field's
/// own JSON type (the live API sends decimal strings such as "254"). A missing
/// `startIndex` (meaning 0) takes the type of `endIndex`.
fn shift_segment(segment: &mut Value, base: usize) {
    let end_is_string = segment.get("endIndex").is_some_and(Value::is_string);
    for key in ["startIndex", "endIndex"] {
        let as_string = segment.get(key).map_or(end_is_string, Value::is_string);
        let shifted = segment_offset(segment, key).unwrap_or(0) + base;
        segment[key] = if as_string {
            Value::String(shifted.to_string())
        } else {
            Value::from(shifted)
        };
    }
}

fn with_connector_hint(reason: &str, connector_auth_failures: usize) -> String {
    if connector_auth_failures == 0 {
        reason.to_string()
    } else {
        format!("{reason} ({connector_auth_failures} connector authentication error(s) were also reported)")
    }
}

fn assist_error(reason: impl Into<String>, session: Option<String>) -> Error {
    Error::Assist {
        reason: reason.into(),
        session,
    }
}

async fn bounded_body(mut response: reqwest::Response) -> std::result::Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err("response exceeds the 16 MiB limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "response read failed before EOF")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("response exceeds the 16 MiB limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// Content-free diagnostics: do not echo server bodies, prompts, answer text or token material.
/// Gemini Enterprise multi-regions accepted by this client.
pub(crate) const SUPPORTED_LOCATIONS: [&str; 3] = ["global", "us", "eu"];

/// Discovery Engine v1 endpoint: the bare host for `global`, `{location}-` otherwise.
fn endpoint(location: &str) -> String {
    if location == "global" {
        "https://discoveryengine.googleapis.com/v1/".to_string()
    } else {
        format!("https://{location}-discoveryengine.googleapis.com/v1/")
    }
}

fn http_reason(status: u16, bytes: &[u8]) -> String {
    let value: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let permission = value["error"]["details"].as_array().and_then(|details| {
        details.iter().find_map(|d| {
            d["metadata"]["permission"].as_str().filter(|s| {
                s.len() <= 150 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
            })
        })
    });
    match permission {
        Some(p) => format!("HTTP {status}; missing permission {p}"),
        None if status == 404 => format!(
            "HTTP {status}; check the engine ID and that the location matches the engine's own location (an engine in us or eu is not found under global)"
        ),
        None => format!(
            "HTTP {status}; inspect access, model entitlement, quota and request configuration"
        ),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    answer: Option<Answer>,
    session_info: Option<SessionInfo>,
    error: Option<Value>,
    #[serde(default)]
    connector_auth_errors: Vec<Value>,
    #[serde(default)]
    invocation_tools: Vec<String>,
}
#[derive(Deserialize)]
struct SessionInfo {
    session: String,
}
#[derive(Deserialize)]
struct Answer {
    state: Option<String>,
    name: Option<String>,
    #[serde(default, rename = "assistSkippedReasons")]
    skipped_reasons: Vec<String>,
    #[serde(default)]
    replies: Vec<Reply>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    #[serde(default)]
    grounded_content: GroundedContent,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroundedContent {
    content: Option<Content>,
    text_grounding_metadata: Option<TextGroundingMetadata>,
}
#[derive(Deserialize)]
struct Content {
    text: Option<String>,
    #[serde(default)]
    thought: bool,
}

// Enforce the frame budget while deserializing, before allocating an unbounded Vec.
struct Frames(Vec<Frame>);
impl<'de> Deserialize<'de> for Frames {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Frames;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a bounded streamAssist JSON frame array")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Frames, A::Error> {
                let mut frames = Vec::new();
                while let Some(frame) = seq.next_element::<Frame>()? {
                    if frames.len() == MAX_FRAMES {
                        return Err(serde::de::Error::custom(
                            "streamAssist frame budget exceeded",
                        ));
                    }
                    frames.push(frame);
                }
                Ok(Frames(frames))
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}

#[path = "scoped.rs"]
mod scoped;
pub use scoped::{
    AnswerCitation, AnswerReference, AnswerSource, FolderAnswer, FolderAnswerRequest, SearchHit,
    SearchRequest, SearchResults, MAX_SEARCH_RESULTS,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
