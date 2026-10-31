//! Folder-scoped retrieval: Discovery Engine v1 Search and Answer.
//!
//! `streamAssist` cannot be restricted to a SharePoint folder, so a scoped answer takes two
//! steps: (1) Search with `(<terms>) path:"<folder>"`, which SharePoint enforces, and
//! (2) Answer with only those results in `searchSpec.searchResultList`, so no other
//! server-side search runs. Every returned URL is re-checked against the folder and
//! anything outside it fails the call. Answers run in a server session so follow-ups
//! can use the history, but the session does not store the scope: every turn searches
//! again and sends only those files.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{bounded_body, http_reason, GeminiEnterpriseClient};
use crate::sharepoint::{validate_search_terms, SharePointFolder};
use crate::{Error, Result};

/// The Search API returns at most 100 results per call and does not page further.
pub const MAX_SEARCH_RESULTS: u32 = 100;

/// A Search API request.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Keywords. With a folder, SharePoint requires every word to match, so use
    /// keywords rather than a natural-language question.
    pub terms: String,
    /// Restrict results to this folder (recursive), enforced by SharePoint.
    pub folder: Option<SharePointFolder>,
    pub data_stores: Vec<String>,
    /// 1-100 (default 10).
    pub page_size: u32,
}

impl SearchRequest {
    pub fn new(terms: impl Into<String>) -> Self {
        Self {
            terms: terms.into(),
            folder: None,
            data_stores: Vec::new(),
            page_size: 10,
        }
    }
}

/// One search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Discovery Engine document resource.
    pub document: String,
    pub title: Option<String>,
    pub uri: Option<String>,
    pub file_type: Option<String>,
    /// Full text, returned for only about the top five results of a call.
    pub content: Option<String>,
    /// Short description snippet (highlight markup removed).
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    /// The query sent, including the folder restriction.
    pub query: String,
    pub hits: Vec<SearchHit>,
    /// `totalSize` as reported by the service, if any.
    pub total_size: Option<u64>,
    /// `true` when the service signals more matches than were returned (`nextPageToken`
    /// or a larger `totalSize`), or the page came back full.
    pub more_results: bool,
    pub warnings: Vec<String>,
}

/// A question answered only from files in one SharePoint folder.
#[derive(Debug, Clone)]
pub struct FolderAnswerRequest {
    pub question: String,
    /// Keywords used to find the files (not the question; see [`SearchRequest::terms`]).
    pub search_terms: String,
    pub folder: SharePointFolder,
    /// Datastores to search; at least one (e.g. the SharePoint file datastore).
    pub data_stores: Vec<String>,
    /// Files to retrieve, 1-100 (default 10).
    pub page_size: u32,
    /// Continue this folder-answer session (ID or full name); `None` starts a new one.
    /// The folder scope is not stored in the session: every turn searches the folder
    /// again and sends only those files.
    pub session: Option<String>,
}

impl FolderAnswerRequest {
    pub fn new(
        question: impl Into<String>,
        search_terms: impl Into<String>,
        folder: SharePointFolder,
    ) -> Self {
        Self {
            question: question.into(),
            search_terms: search_terms.into(),
            folder,
            data_stores: Vec::new(),
            page_size: 10,
            session: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerCitation {
    /// UTF-8 byte offsets into [`FolderAnswer::answer`], when reported.
    pub start_index: Option<usize>,
    pub end_index: Option<usize>,
    /// Indices into [`FolderAnswer::references`].
    pub reference_ids: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerReference {
    pub document: Option<String>,
    pub title: Option<String>,
    pub uri: Option<String>,
    /// The cited chunk of the file.
    pub content: Option<String>,
    pub relevance_score: Option<f64>,
}

/// A file supplied to the Answer step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerSource {
    pub title: Option<String>,
    pub uri: Option<String>,
    /// `true` when sent as full text, `false` when only a snippet was available.
    pub full_text: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderAnswer {
    pub answer: String,
    /// Server session holding this turn; pass it back to continue the conversation.
    pub session: String,
    /// The answer resource (`…/sessions/<sid>/answers/<aid>`).
    pub answer_resource: String,
    pub folder: String,
    pub search_query: String,
    pub citations: Vec<AnswerCitation>,
    pub references: Vec<AnswerReference>,
    pub sources: Vec<AnswerSource>,
    pub warnings: Vec<String>,
}

impl GeminiEnterpriseClient {
    /// Search the engine (no generation). With a folder, every result must be inside
    /// it; otherwise the call fails rather than returning out-of-folder files.
    pub async fn search(&self, request: SearchRequest) -> Result<SearchResults> {
        let terms = validate_search_terms(&request.terms)?;
        if request.page_size == 0 || request.page_size > MAX_SEARCH_RESULTS {
            return Err(Error::validation(format!(
                "search page size must be 1-{MAX_SEARCH_RESULTS}"
            )));
        }
        if request.folder.is_some() && request.data_stores.is_empty() {
            // Without explicit datastores the engine would fan the query out to all of its
            // connectors before the local folder check could reject their results.
            return Err(Error::validation(
                "a folder-scoped search needs at least one datastore (the SharePoint file datastore)",
            ));
        }
        let query = match &request.folder {
            Some(folder) => format!("({terms}) {}", folder.kql()),
            None => terms.to_string(),
        };
        let mut body = json!({
            "query": query,
            "pageSize": request.page_size,
            // AUTO correction rewrites the folder URL; suggestions only.
            "spellCorrectionSpec": {"mode": "SUGGESTION_ONLY"},
        });
        if !request.data_stores.is_empty() {
            let specs = request
                .data_stores
                .iter()
                .map(|store| Ok(json!({"dataStore": self.datastore_resource(store)?})))
                .collect::<Result<Vec<_>>>()?;
            body["dataStoreSpecs"] = Value::Array(specs);
        }
        let bytes = self.post_serving_config("search", &body, "search").await?;
        let wire: SearchWire = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Api("invalid search response JSON/schema".into()))?;
        let hits: Vec<SearchHit> = wire.results.into_iter().map(SearchHit::from).collect();
        if let Some(folder) = &request.folder {
            if hits
                .iter()
                .any(|hit| !hit.uri.as_deref().is_some_and(|uri| folder.contains(uri)))
            {
                return Err(Error::Api(
                    "search returned a result outside the SharePoint folder; refusing all results"
                        .into(),
                ));
            }
        }
        // The service sets nextPageToken when the page is full (following it returns an
        // empty page for federated SharePoint search), and totalSize when it knows more.
        let more_results = wire
            .next_page_token
            .as_deref()
            .is_some_and(|t| !t.is_empty())
            || wire
                .total_size
                .is_some_and(|total| total > hits.len() as u64)
            || hits.len() as u32 >= request.page_size;
        let mut warnings = Vec::new();
        if more_results {
            warnings.push(format!(
                "Returned {} results, but more files may match (the service reports more; this client does not page and a search returns at most {MAX_SEARCH_RESULTS}). Narrow the terms or raise the page size.",
                hits.len()
            ));
        }
        Ok(SearchResults {
            query,
            hits,
            total_size: wire.total_size,
            more_results,
            warnings,
        })
    }

    /// Answer `question` using only files from `folder` that match `search_terms`:
    /// a scoped Search, then the Answer API restricted to those results (one generation).
    /// Starts a server session, or continues `request.session`; the scope is re-applied
    /// on every turn. Fails if any reference points outside the folder.
    pub async fn answer_in_folder(&self, request: FolderAnswerRequest) -> Result<FolderAnswer> {
        let question = request.question.trim();
        if question.is_empty() || question.len() > 256 * 1024 {
            return Err(Error::validation(
                "question must be nonempty and at most 256 KiB UTF-8",
            ));
        }
        if request.data_stores.is_empty() {
            return Err(Error::validation(
                "a folder-scoped answer needs at least one datastore (the SharePoint file datastore)",
            ));
        }
        let requested_session = request
            .session
            .as_deref()
            .map(|s| self.session_resource(s))
            .transpose()?;
        let mut search = SearchRequest::new(request.search_terms);
        search.folder = Some(request.folder.clone());
        search.data_stores = request.data_stores;
        search.page_size = request.page_size;
        let found = self.search(search).await?;

        let mut sources = Vec::new();
        let mut results = Vec::new();
        for hit in &found.hits {
            let (key, content, full_text) = match (&hit.content, &hit.snippet) {
                (Some(text), _) if !text.trim().is_empty() => ("documentContexts", text, true),
                (_, Some(snippet)) if !snippet.trim().is_empty() => {
                    ("extractiveSegments", snippet, false)
                }
                _ => continue,
            };
            let mut info = json!({"document": hit.document});
            if let Some(uri) = &hit.uri {
                info["uri"] = json!(uri);
            }
            if let Some(title) = &hit.title {
                info["title"] = json!(title);
            }
            info[key] = json!([{"content": content}]);
            results.push(json!({"unstructuredDocumentInfo": info}));
            sources.push(AnswerSource {
                title: hit.title.clone(),
                uri: hit.uri.clone(),
                full_text,
            });
        }
        if results.is_empty() {
            // Nothing to answer from: do not call the Answer API at all.
            return Err(Error::Skipped {
                reasons: vec!["NO_MATCHING_FILES_IN_FOLDER".into()],
                session: None,
            });
        }

        let body = json!({
            "query": {"text": question},
            // "-" creates a session so follow-ups can use the conversation history.
            "session": requested_session
                .clone()
                .unwrap_or_else(|| format!("{}/sessions/-", self.config.engine())),
            "queryUnderstandingSpec": {
                "queryRephraserSpec": {"disable": true},
                "disableSpellCorrection": true,
            },
            "searchSpec": {"searchResultList": {"searchResults": results}},
            "answerGenerationSpec": {"includeCitations": true, "ignoreLowRelevantContent": false},
        });
        let bytes = self.post_serving_config("answer", &body, "answer").await?;
        let mut answer = parse_folder_answer(&bytes, &request.folder)?;
        // The session must be ours (and the requested one when continuing), and the answer
        // must belong to it.
        let canonical = self
            .session_resource(&answer.session)
            .map_err(|_| Error::Api("invalid returned session resource".into()))?;
        if canonical != answer.session
            || requested_session
                .as_ref()
                .is_some_and(|requested| requested != &canonical)
        {
            return Err(Error::Api("returned session resource mismatch".into()));
        }
        let answer_id = answer
            .answer_resource
            .strip_prefix(&format!("{canonical}/answers/"))
            .unwrap_or("");
        if answer_id.is_empty() || answer_id.contains('/') {
            return Err(Error::Api("returned answer resource mismatch".into()));
        }
        answer.search_query = found.query;
        answer.sources = sources;
        answer.warnings.extend(found.warnings);
        let snippets = answer.sources.iter().filter(|s| !s.full_text).count();
        if snippets > 0 {
            answer.warnings.push(format!(
                "{snippets} of {} files were sent as short snippets only (Search returns full text for only about 5 files); the answer may miss content in them.",
                answer.sources.len()
            ));
        }
        Ok(answer)
    }

    async fn post_serving_config(
        &self,
        method: &str,
        body: &Value,
        label: &str,
    ) -> Result<Vec<u8>> {
        let resource = format!(
            "{}/servingConfigs/default_search:{method}",
            self.config.engine()
        );
        let url = self.base_url.join(&resource)?;
        let token = self.provider.access_token().await?;
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("x-goog-user-project", &self.config.project_number)
            .json(body)
            .send()
            .await
            .map_err(|_| Error::Api(format!("{label} transport failure")))?;
        let status = response.status();
        let bytes = bounded_body(response).await.map_err(Error::Api)?;
        if !status.is_success() {
            return Err(Error::Api(format!(
                "{label}: {}",
                http_reason(status.as_u16(), &bytes)
            )));
        }
        Ok(bytes)
    }
}

fn parse_folder_answer(bytes: &[u8], folder: &SharePointFolder) -> Result<FolderAnswer> {
    let wire: AnswerWire = serde_json::from_slice(bytes)
        .map_err(|_| Error::Api("invalid answer response JSON/schema".into()))?;
    let session = wire.session.and_then(|s| s.name);
    let answer = wire
        .answer
        .ok_or_else(|| Error::Api("answer response has no answer".into()))?;
    // Skips can arrive with state SUCCEEDED and a canned "could not be generated" text.
    if !answer.answer_skipped_reasons.is_empty() {
        return Err(Error::Skipped {
            reasons: answer.answer_skipped_reasons,
            session,
        });
    }
    if answer.state.as_deref() != Some("SUCCEEDED") {
        return Err(Error::Assist {
            reason: format!(
                "answer state {}",
                answer.state.as_deref().unwrap_or("missing")
            ),
            session,
        });
    }
    let session = session.ok_or_else(|| Error::Api("answer response has no session".into()))?;
    let answer_resource = answer
        .name
        .ok_or_else(|| Error::Api("answer response has no answer name".into()))?;
    let text = answer.answer_text.unwrap_or_default();
    if text.trim().is_empty() {
        return Err(Error::Api("no textual answer".into()));
    }
    let references: Vec<AnswerReference> = answer
        .references
        .into_iter()
        .map(AnswerReference::from)
        .collect();
    if references
        .iter()
        .any(|r| !r.uri.as_deref().is_some_and(|uri| folder.contains(uri)))
    {
        return Err(Error::Api(
            "answer cited a file outside the SharePoint folder; refusing the answer".into(),
        ));
    }
    let mut citations = Vec::new();
    for citation in answer.citations {
        let reference_ids = citation
            .sources
            .iter()
            .map(|source| index(&source.reference_id).filter(|i| *i < references.len()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Error::Api("answer citation references an unknown source".into()))?;
        let start_index = strict_offset(citation.start_index.as_ref(), &text)?;
        let end_index = strict_offset(citation.end_index.as_ref(), &text)?;
        if let (Some(start), Some(end)) = (start_index, end_index) {
            if start > end {
                return Err(Error::Api("answer citation starts after it ends".into()));
            }
        }
        citations.push(AnswerCitation {
            start_index,
            end_index,
            reference_ids,
        });
    }
    let mut warnings = Vec::new();
    if citations.is_empty() {
        warnings.push("No structured citations returned.".into());
    }
    Ok(FolderAnswer {
        answer: text,
        session,
        answer_resource,
        folder: folder.url().to_string(),
        search_query: String::new(),
        citations,
        references,
        sources: Vec::new(),
        warnings,
    })
}

/// A present citation offset must be a valid index ("269" or 269), within the answer
/// and on a UTF-8 character boundary; malformed values fail instead of being dropped.
fn strict_offset(value: Option<&Value>, text: &str) -> Result<Option<usize>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let offset =
        index(value).ok_or_else(|| Error::Api("answer citation offset is malformed".into()))?;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(Error::Api(
            "answer citation offset is out of range or not on a character boundary".into(),
        ));
    }
    Ok(Some(offset))
}

/// Offsets and IDs arrive as decimal strings ("269") or numbers.
fn index(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .and_then(|n| usize::try_from(n).ok())
}

fn clean_snippet(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        // Only the search highlight markup: <c0>, </c0>, <ddd/>.
        let end = tail.find('>').map(|i| i + 1);
        match end {
            Some(end) if is_highlight_tag(&tail[..end]) => rest = &tail[end..],
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_highlight_tag(tag: &str) -> bool {
    let inner = tag.trim_start_matches('<').trim_end_matches('>');
    let inner = inner.trim_start_matches('/').trim_end_matches('/');
    inner == "ddd"
        || (inner.starts_with('c')
            && inner[1..].bytes().all(|b| b.is_ascii_digit())
            && inner.len() > 1)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchWire {
    #[serde(default)]
    results: Vec<SearchResultWire>,
    #[serde(default, deserialize_with = "lenient_u64")]
    total_size: Option<u64>,
    next_page_token: Option<String>,
}

/// `totalSize` is an int32 in the schema but may be sent as a string.
fn lenient_u64<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> std::result::Result<Option<u64>, D::Error> {
    let value = Option::<Value>::deserialize(de)?;
    Ok(value
        .as_ref()
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok())))
}
#[derive(Deserialize)]
struct SearchResultWire {
    document: SearchDocumentWire,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchDocumentWire {
    name: String,
    #[serde(default)]
    struct_data: Value,
    #[serde(default)]
    derived_struct_data: Value,
}

impl From<SearchResultWire> for SearchHit {
    fn from(result: SearchResultWire) -> Self {
        let doc = result.document;
        let field = |key: &str| {
            doc.struct_data
                .get(key)
                .or_else(|| doc.derived_struct_data.get(key))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        SearchHit {
            title: field("title"),
            uri: field("url"),
            file_type: field("file_type"),
            content: field("content"),
            snippet: field("description").map(|d| clean_snippet(&d)),
            document: doc.name,
        }
    }
}

#[derive(Deserialize)]
struct AnswerWire {
    answer: Option<AnswerBodyWire>,
    session: Option<AnswerSessionWire>,
}
#[derive(Deserialize)]
struct AnswerSessionWire {
    name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerBodyWire {
    name: Option<String>,
    state: Option<String>,
    answer_text: Option<String>,
    #[serde(default)]
    citations: Vec<CitationWire>,
    #[serde(default)]
    references: Vec<ReferenceWire>,
    #[serde(default)]
    answer_skipped_reasons: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CitationWire {
    start_index: Option<Value>,
    end_index: Option<Value>,
    #[serde(default)]
    sources: Vec<CitationSourceWire>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CitationSourceWire {
    #[serde(default)]
    reference_id: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceWire {
    chunk_info: Option<ChunkInfoWire>,
    unstructured_document_info: Option<DocumentInfoWire>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChunkInfoWire {
    content: Option<String>,
    relevance_score: Option<f64>,
    document_metadata: Option<DocumentInfoWire>,
}
#[derive(Deserialize)]
struct DocumentInfoWire {
    document: Option<String>,
    uri: Option<String>,
    title: Option<String>,
}

impl From<ReferenceWire> for AnswerReference {
    fn from(reference: ReferenceWire) -> Self {
        let (meta, content, relevance_score) = match reference.chunk_info {
            Some(chunk) => (
                chunk.document_metadata,
                chunk.content,
                chunk.relevance_score,
            ),
            None => (reference.unstructured_document_info, None, None),
        };
        let meta = meta.unwrap_or(DocumentInfoWire {
            document: None,
            uri: None,
            title: None,
        });
        AnswerReference {
            document: meta.document,
            title: meta.title,
            uri: meta.uri,
            content,
            relevance_score,
        }
    }
}

#[cfg(test)]
#[path = "scoped_tests.rs"]
mod tests;
