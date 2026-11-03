//! Public Gemini Enterprise request/results, distinct from the NotebookLM notebook API.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One conversational request. No arbitrary functions or mutative actions are exposed.
#[derive(Debug, Clone)]
pub struct AssistRequest {
    pub question: String,
    /// None uses the assistant default. An explicit model is never replaced on failure.
    pub model_id: Option<String>,
    /// Short ID or full session resource; None starts a new server-side session.
    pub session: Option<String>,
    /// Short datastore IDs or full resources, all in this client's project/location/collection.
    pub data_stores: Vec<String>,
    /// Opt in to public web grounding (requires assistant/admin support).
    pub web_grounding: bool,
}

impl AssistRequest {
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            model_id: None,
            session: None,
            data_stores: Vec::new(),
            web_grounding: false,
        }
    }
}

/// Completed answer. Answer prose never manufactures entries in `grounding`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistResponse {
    pub answer: String,
    pub session: String,
    pub answer_resource: String,
    /// What was requested, not proof of what model was served. Inspect the saved session.
    pub requested_model: Option<String>,
    /// Keep batches separate: reference indices are local to each metadata batch.
    pub grounding: Vec<TextGroundingMetadata>,
    pub invocation_tools: Vec<String>,
    pub warnings: Vec<String>,
}

impl AssistResponse {
    pub fn native_reference_count(&self) -> usize {
        self.grounding.iter().map(|g| g.references.len()).sum()
    }
}

/// One native grounding block. In [`AssistResponse`], each segment's
/// `startIndex`/`endIndex` is normalized to a UTF-8 byte offset into the complete
/// answer. In [`ResearchContent`], offsets instead remain local to that content's
/// text. Reference indices are always local to this metadata batch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextGroundingMetadata {
    #[serde(default)]
    pub references: Vec<GroundingReference>,
    #[serde(default)]
    pub segments: Vec<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundingReference {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub document_metadata: DocumentMetadata,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMetadata {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub document: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistSession {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub turns: Vec<AssistSessionTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistSessionTurn {
    #[serde(default)]
    pub query: Option<SessionQuery>,
    #[serde(default)]
    pub query_config: BTreeMap<String, String>,
    /// Set for chat (streamAssist) turns.
    #[serde(default)]
    pub detailed_assist_answer: Option<Value>,
    /// Answer resource of a folder-scoped (Answer API) turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// Full answer (text, citations, references) of a folder-scoped turn, with
    /// `includeAnswerDetails`. Such turns have no `queryConfig`, so no `model_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed_answer: Option<Value>,
}

impl AssistSessionTurn {
    /// Service-reported persisted selection; not independent attestation of model weights/build.
    pub fn model_id(&self) -> Option<&str> {
        self.query_config
            .get("google.discoveryengine.googleapis.com.Assistant.model_id")
            .map(String::as_str)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionQuery {
    #[serde(default)]
    pub text: String,
}

/// Topic and retrieval configuration for a new Deep Research plan. Planning does
/// not automatically send the separate Start Research turn. No model override.
#[derive(Debug, Clone)]
pub struct ResearchPlanRequest {
    pub topic: String,
    /// Datastore selection, NOT a folder/file isolation guarantee.
    pub data_stores: Vec<String>,
    pub web_grounding: bool,
}

impl ResearchPlanRequest {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            data_stores: Vec::new(),
            web_grounding: false,
        }
    }
}

/// Explicitly authorize one research generation in a previously planned session.
/// Keep the same datastore/web configuration used for the plan. There is no
/// server idempotency key: callers must not automatically repeat this request.
#[derive(Debug, Clone)]
pub struct ResearchStartRequest {
    pub session: String,
    pub data_stores: Vec<String>,
    pub web_grounding: bool,
}

impl ResearchStartRequest {
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            data_stores: Vec::new(),
            web_grounding: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhase {
    Plan,
    Start,
    /// Read an already persisted answer; never generates.
    Read,
}

/// Bounded, terminal-successful agent response. A successful transport/answer is
/// NOT independent proof of agent routing or a completed research report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResponse {
    pub session: String,
    pub answer_resource: String,
    /// None for read-only reconstruction (no agent invocation was sent).
    pub requested_agent: Option<String>,
    pub requested_phase: ResearchPhase,
    /// Ordered visible blocks; intermediate material is never concatenated into a report.
    pub contents: Vec<ResearchContent>,
    /// Only blocks explicitly marked RESEARCH_REPORT by the service.
    pub report_indices: Vec<usize>,
    /// Last nonempty text block if stages were omitted. NOT a verified report.
    pub final_text_candidate_index: Option<usize>,
    pub invocation_tools: Vec<String>,
    pub warnings: Vec<String>,
}

/// Native stage metadata, when available. Public v1 currently omits these labels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchContent {
    pub kind: Option<String>,
    pub agent: Option<String>,
    pub text: Option<String>,
    pub file: Option<ResearchFile>,
    /// Offsets are UTF-8 bytes relative to THIS block's text, not the whole turn.
    /// Reference indices stay local to each batch. Duplicate terminal metadata
    /// is not counted twice. Native references do not attest citation semantics.
    pub grounding: Vec<TextGroundingMetadata>,
    /// Native segments containing -1 reference markers, NOT usable citations.
    /// Valid byte offsets are still required; their original values are preserved.
    pub unattributed_segments: Vec<UnattributedResearchSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnattributedResearchSegment {
    /// The batch in this block's grounding whose references the original segment used.
    pub grounding_batch: usize,
    pub segment: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchFile {
    pub file_id: String,
    pub mime_type: String,
}
