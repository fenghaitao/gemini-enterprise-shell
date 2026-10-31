//! Python bindings for the independent official Gemini Enterprise client.
use std::{sync::Arc, time::Duration};

use gemini_chat::{AssistRequest, AssistResponse, GeminiEnterpriseConfig};
use pyo3::{exceptions::PyValueError, prelude::*};

use crate::{
    auth::PyTokenProvider,
    error::{NblmError, PyResult},
    runtime::block_on_with_runtime,
};

/// Lets the SDK's token providers (gcloud, env, user OAuth) drive the independent
/// `gemini-chat` client.
struct SdkTokens(Arc<dyn nblm_core::TokenProvider>);

#[async_trait::async_trait]
impl gemini_chat::TokenProvider for SdkTokens {
    async fn access_token(&self) -> gemini_chat::Result<String> {
        self.0
            .access_token()
            .await
            .map_err(|err| gemini_chat::Error::TokenProvider(err.to_string()))
    }
}

/// Failed generations keep their server session reachable and are marked do-not-retry.
fn map_chat_error(err: gemini_chat::Error) -> PyErr {
    let message = err.to_string();
    match err {
        gemini_chat::Error::Assist { session, .. } => Python::attach(|py| {
            let py_err = NblmError::new_err(message);
            let _ = py_err.value(py).setattr("session", session);
            let _ = py_err.value(py).setattr("do_not_retry", true);
            py_err
        }),
        _ => NblmError::new_err(message),
    }
}

fn block_on_chat<F, T>(future: F) -> PyResult<T>
where
    F: std::future::Future<Output = gemini_chat::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    block_on_with_runtime(async move { Ok(future.await) })?.map_err(map_chat_error)
}

#[pyclass(module = "nblm")]
pub struct GeminiEnterpriseClient {
    inner: Arc<gemini_chat::GeminiEnterpriseClient>,
}

#[pymethods]
impl GeminiEnterpriseClient {
    /// Official v1 API. Uses OAuth, never browser state. No generation retries/fallback.
    #[new]
    #[pyo3(signature = (token_provider, project_number, engine_id, location="global".to_string(), *, collection_id="default_collection".to_string(), assistant_id="default_assistant".to_string(), timeout_secs=360.0, base_url=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        token_provider: PyTokenProvider,
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
        let mut config = GeminiEnterpriseConfig::new(project_number, location, engine_id);
        config.collection_id = collection_id;
        config.assistant_id = assistant_id;
        config.timeout = timeout;
        let provider = Arc::new(SdkTokens(token_provider.get_inner()));
        let mut inner =
            gemini_chat::GeminiEnterpriseClient::new(provider, config).map_err(map_chat_error)?;
        if let Some(base) = base_url {
            inner = inner.with_base_url(&base).map_err(map_chat_error)?;
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
        py: Python,
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
        let inner = self.inner.clone();
        let response = py.detach(move || block_on_chat(async move { inner.ask(request).await }))?;
        Ok(GeminiAssistResponse { inner: response })
    }

    /// Read persisted queries/model configuration. Does not generate a turn.
    #[pyo3(signature = (session_id, *, include_answer_details=false))]
    fn get_session(
        &self,
        py: Python,
        session_id: String,
        include_answer_details: bool,
    ) -> PyResult<Py<PyAny>> {
        let inner = self.inner.clone();
        let session = py.detach(move || {
            block_on_chat(
                async move { inner.get_session(&session_id, include_answer_details).await },
            )
        })?;
        to_python(
            py,
            serde_json::to_value(&session)
                .map_err(|_| PyValueError::new_err("cannot serialize session"))?,
        )
    }

    fn __repr__(&self) -> &'static str {
        "GeminiEnterpriseClient(official_v1=True)"
    }
}

/// Completed official Gemini Enterprise assistant answer.
#[pyclass(module = "nblm")]
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
    fn grounding(&self, py: Python) -> PyResult<Py<PyAny>> {
        to_python(py, serde_json::json!(&self.inner.grounding))
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
    fn to_dict(&self, py: Python) -> PyResult<Py<PyAny>> {
        to_python(py, serde_json::json!(&self.inner))
    }
    fn __repr__(&self) -> String {
        format!(
            "GeminiAssistResponse(native_reference_count={})",
            self.native_reference_count()
        )
    }
}

// serde_json is the common representation for nested native metadata; never flatten
// reference batches or reclassify citations found only in generated prose.
fn to_python(py: Python, value: serde_json::Value) -> PyResult<Py<PyAny>> {
    let encoded = serde_json::to_string(&value)
        .map_err(|_| PyValueError::new_err("cannot serialize Gemini Enterprise result"))?;
    Ok(py
        .import("json")?
        .call_method1("loads", (encoded,))?
        .unbind())
}
