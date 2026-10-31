//! Python bindings for `gemini-chat`, the client for the official Gemini Enterprise
//! assistant API. Independent of NotebookLM: depends only on the `gemini-chat` crate.
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};

use gemini_chat::{
    AssistRequest, AssistResponse, FolderAnswerRequest, GeminiEnterpriseConfig, SearchRequest,
    SharePointFolder,
};
use pyo3::{
    create_exception,
    exceptions::{PyException, PyRuntimeError, PyTypeError, PyValueError},
    prelude::*,
};

create_exception!(
    gemini_chat,
    GeminiChatError,
    PyException,
    "A Gemini Enterprise request failed. May carry `session`, `do_not_retry` and `skipped_reasons`."
);

const DEFAULT_TOKEN_ENV: &str = "GEMINI_CHAT_ACCESS_TOKEN";

/// Gets access tokens by running `gcloud auth print-access-token`.
#[pyclass(module = "gemini_chat", frozen)]
pub struct GcloudTokenProvider {
    binary: String,
}

#[pymethods]
impl GcloudTokenProvider {
    #[new]
    #[pyo3(signature = (binary="gcloud".to_string()))]
    fn new(binary: String) -> Self {
        Self { binary }
    }

    #[getter]
    fn binary(&self) -> &str {
        &self.binary
    }

    fn __repr__(&self) -> String {
        format!("GcloudTokenProvider(binary={:?})", self.binary)
    }
}

/// Reads an access token from an environment variable (default `GEMINI_CHAT_ACCESS_TOKEN`).
#[pyclass(module = "gemini_chat", frozen)]
pub struct EnvTokenProvider {
    key: String,
}

#[pymethods]
impl EnvTokenProvider {
    #[new]
    #[pyo3(signature = (key=DEFAULT_TOKEN_ENV.to_string()))]
    fn new(key: String) -> Self {
        Self { key }
    }

    #[getter]
    fn key(&self) -> &str {
        &self.key
    }

    fn __repr__(&self) -> String {
        format!("EnvTokenProvider(key={:?})", self.key)
    }
}

fn token_provider(value: &Bound<'_, PyAny>) -> PyResult<Arc<dyn gemini_chat::TokenProvider>> {
    if let Ok(gcloud) = value.extract::<PyRef<'_, GcloudTokenProvider>>() {
        return Ok(Arc::new(gemini_chat::GcloudTokenProvider::new(
            gcloud.binary.clone(),
        )));
    }
    if let Ok(env) = value.extract::<PyRef<'_, EnvTokenProvider>>() {
        return Ok(Arc::new(gemini_chat::EnvTokenProvider::new(
            env.key.clone(),
        )));
    }
    Err(PyTypeError::new_err(
        "token_provider must be gemini_chat.GcloudTokenProvider or gemini_chat.EnvTokenProvider",
    ))
}

/// Failed generations keep their server session reachable and are marked do-not-retry;
/// skipped questions (nothing generated) report why.
fn map_error(err: gemini_chat::Error) -> PyErr {
    let message = err.to_string();
    Python::attach(|py| {
        let py_err = GeminiChatError::new_err(message);
        let value = py_err.value(py);
        let (session, do_not_retry, skipped) = match err {
            gemini_chat::Error::Assist { session, .. } => (session, true, None),
            gemini_chat::Error::Skipped { reasons, session } => (session, false, Some(reasons)),
            _ => (None, false, None),
        };
        let _ = value.setattr("session", session);
        let _ = value.setattr("do_not_retry", do_not_retry);
        let _ = value.setattr("skipped_reasons", skipped.unwrap_or_default());
        py_err
    })
}

fn runtime() -> PyResult<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    let built = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| PyRuntimeError::new_err(format!("cannot start async runtime: {err}")))?;
    Ok(RUNTIME.get_or_init(|| built))
}

/// Run a request without holding the GIL, so callers' threads, spinners and Ctrl-C keep working.
fn block_on<F, T>(py: Python<'_>, future: F) -> PyResult<T>
where
    F: Future<Output = gemini_chat::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let runtime = runtime()?;
    py.detach(move || runtime.block_on(future))
        .map_err(map_error)
}

/// Client for the official Gemini Enterprise assistant (Discovery Engine v1).
/// OAuth only; never retries generation or falls back to another model.
#[pyclass(module = "gemini_chat", frozen)]
pub struct GeminiEnterpriseClient {
    inner: Arc<gemini_chat::GeminiEnterpriseClient>,
}

#[pymethods]
impl GeminiEnterpriseClient {
    #[new]
    #[pyo3(signature = (token_provider, project_number, engine_id, location="global".to_string(), *, collection_id="default_collection".to_string(), assistant_id="default_assistant".to_string(), timeout_secs=360.0, base_url=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        token_provider: &Bound<'_, PyAny>,
        project_number: String,
        engine_id: String,
        location: String,
        collection_id: String,
        assistant_id: String,
        timeout_secs: f64,
        base_url: Option<String>,
    ) -> PyResult<Self> {
        let timeout = Duration::try_from_secs_f64(timeout_secs)
            .map_err(|_| PyValueError::new_err("timeout_secs must be positive and finite"))?;
        if timeout.is_zero() {
            return Err(PyValueError::new_err("timeout_secs must be positive"));
        }
        let provider = self::token_provider(token_provider)?;
        let mut config = GeminiEnterpriseConfig::new(project_number, location, engine_id);
        config.collection_id = collection_id;
        config.assistant_id = assistant_id;
        config.timeout = timeout;
        let mut inner =
            gemini_chat::GeminiEnterpriseClient::new(provider, config).map_err(map_error)?;
        if let Some(base) = base_url {
            inner = inner.with_base_url(&base).map_err(map_error)?;
        }
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Ask once. On a timeout/failure, inspect the saved session before any manual retry.
    /// Data stores are selected explicitly; a prompt URL/folder is not an isolation boundary.
    #[pyo3(signature = (question, *, model_id=None, session_id=None, data_store_ids=None, web_grounding=false))]
    fn ask(
        &self,
        py: Python<'_>,
        question: String,
        model_id: Option<String>,
        session_id: Option<String>,
        data_store_ids: Option<Vec<String>>,
        web_grounding: bool,
    ) -> PyResult<GeminiAssistResponse> {
        let mut request = AssistRequest::new(question);
        request.model_id = model_id;
        request.session = session_id;
        request.data_stores = data_store_ids.unwrap_or_default();
        request.web_grounding = web_grounding;
        let inner = Arc::clone(&self.inner);
        let response = block_on(py, async move { inner.ask(request).await })?;
        Ok(GeminiAssistResponse { inner: response })
    }

    /// Read persisted queries/model configuration. Does not generate a turn.
    #[pyo3(signature = (session_id, *, include_answer_details=false))]
    fn get_session(
        &self,
        py: Python<'_>,
        session_id: String,
        include_answer_details: bool,
    ) -> PyResult<Py<PyAny>> {
        let inner = Arc::clone(&self.inner);
        let session = block_on(py, async move {
            inner.get_session(&session_id, include_answer_details).await
        })?;
        to_python(py, &session)
    }

    /// Search files without generating. With `sharepoint_folder`, results are restricted
    /// to that folder (recursive) by SharePoint and re-checked; terms are keywords.
    #[pyo3(signature = (terms, *, sharepoint_folder=None, data_store_ids=None, page_size=10))]
    fn search(
        &self,
        py: Python<'_>,
        terms: String,
        sharepoint_folder: Option<String>,
        data_store_ids: Option<Vec<String>>,
        page_size: u32,
    ) -> PyResult<Py<PyAny>> {
        let mut request = SearchRequest::new(terms);
        request.folder = sharepoint_folder
            .as_deref()
            .map(SharePointFolder::parse)
            .transpose()
            .map_err(map_error)?;
        request.data_stores = data_store_ids.unwrap_or_default();
        request.page_size = page_size;
        let inner = Arc::clone(&self.inner);
        let results = block_on(py, async move { inner.search(request).await })?;
        to_python(py, &results)
    }

    /// Answer only from files in one SharePoint folder: a folder-scoped search with
    /// `search_terms` (keywords), then the Answer API with just those files. Sessionless,
    /// one generation. Raises if anything outside the folder is returned or cited.
    #[pyo3(signature = (question, *, search_terms, sharepoint_folder, data_store_ids, page_size=10))]
    fn answer_in_folder(
        &self,
        py: Python<'_>,
        question: String,
        search_terms: String,
        sharepoint_folder: String,
        data_store_ids: Vec<String>,
        page_size: u32,
    ) -> PyResult<Py<PyAny>> {
        let folder = SharePointFolder::parse(&sharepoint_folder).map_err(map_error)?;
        let mut request = FolderAnswerRequest::new(question, search_terms, folder);
        request.data_stores = data_store_ids;
        request.page_size = page_size;
        let inner = Arc::clone(&self.inner);
        let answer = block_on(py, async move { inner.answer_in_folder(request).await })?;
        to_python(py, &answer)
    }

    fn __repr__(&self) -> &'static str {
        "GeminiEnterpriseClient(official_v1=True)"
    }
}

/// Completed official Gemini Enterprise assistant answer.
#[pyclass(module = "gemini_chat", frozen)]
pub struct GeminiAssistResponse {
    inner: AssistResponse,
}

#[pymethods]
impl GeminiAssistResponse {
    #[getter]
    fn answer(&self) -> &str {
        &self.inner.answer
    }
    #[getter]
    fn session(&self) -> &str {
        &self.inner.session
    }
    #[getter]
    fn answer_resource(&self) -> &str {
        &self.inner.answer_resource
    }
    /// Requested model only; get_session reports the service's persisted selection.
    #[getter]
    fn requested_model(&self) -> Option<&str> {
        self.inner.requested_model.as_deref()
    }
    #[getter]
    fn grounding(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.inner.grounding)
    }
    #[getter]
    fn native_reference_count(&self) -> usize {
        self.inner.native_reference_count()
    }
    #[getter]
    fn invocation_tools(&self) -> Vec<String> {
        self.inner.invocation_tools.clone()
    }
    #[getter]
    fn warnings(&self) -> Vec<String> {
        self.inner.warnings.clone()
    }
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.inner)
    }
    fn __repr__(&self) -> String {
        format!(
            "GeminiAssistResponse(session={:?}, native_reference_count={})",
            self.inner.session,
            self.inner.native_reference_count()
        )
    }
}

fn to_python(py: Python<'_>, value: &impl serde::Serialize) -> PyResult<Py<PyAny>> {
    let encoded = serde_json::to_string(value)
        .map_err(|_| PyValueError::new_err("cannot serialize Gemini Enterprise result"))?;
    Ok(py
        .import("json")?
        .call_method1("loads", (encoded,))?
        .unbind())
}

/// Normalize a SharePoint folder URL as used for folder scope (spaces as %20, no
/// trailing slash, library views resolved); raises GeminiChatError if it is unusable.
#[pyfunction]
fn normalize_sharepoint_folder(url: &str) -> PyResult<String> {
    SharePointFolder::parse(url)
        .map(|folder| folder.url().to_string())
        .map_err(map_error)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(normalize_sharepoint_folder, m)?)?;
    m.add_class::<GeminiEnterpriseClient>()?;
    m.add_class::<GeminiAssistResponse>()?;
    m.add_class::<GcloudTokenProvider>()?;
    m.add_class::<EnvTokenProvider>()?;
    m.add("GeminiChatError", m.py().get_type::<GeminiChatError>())?;
    Ok(())
}
