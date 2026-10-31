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

/// One native grounding block. After parsing, every segment's `startIndex`/`endIndex`
/// is a UTF-8 byte offset into [`AssistResponse::answer`], even when the service sent
/// offsets relative to the block's own text fragment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextGroundingMetadata {
    #[serde(default)]
    pub references: Vec<GroundingReference>,
    #[serde(default)]
    pub segments: Vec<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundingReference {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub document_metadata: DocumentMetadata,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(default)]
    pub detailed_assist_answer: Option<Value>,
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
