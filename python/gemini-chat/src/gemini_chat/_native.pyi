"""Official Gemini Enterprise assistant API (Discovery Engine v1), independent of NotebookLM."""

from typing import Any

class GeminiChatError(Exception):
    """A request failed. `session`/`do_not_retry` are set after a dispatched generation
    failed; `skipped_reasons` is set when the assistant skipped the question."""

    session: str | None
    do_not_retry: bool
    skipped_reasons: list[str]

class GcloudTokenProvider:
    """Access tokens from `gcloud auth print-access-token`."""

    def __init__(self, binary: str = "gcloud") -> None: ...
    @property
    def binary(self) -> str: ...

class EnvTokenProvider:
    """Access token from an environment variable."""

    def __init__(self, key: str = "GEMINI_CHAT_ACCESS_TOKEN") -> None: ...
    @property
    def key(self) -> str: ...

TokenProvider = GcloudTokenProvider | EnvTokenProvider

def normalize_sharepoint_folder(url: str) -> str:
    """Normalized folder URL (spaces as %20, no trailing slash, library views resolved);
    raises GeminiChatError for unusable URLs such as sharing links."""

class GeminiEnterpriseClient:
    def __init__(
        self,
        token_provider: TokenProvider,
        project_number: str,
        engine_id: str,
        location: str = "global",
        *,
        collection_id: str = "default_collection",
        assistant_id: str = "default_assistant",
        timeout_secs: float = 360.0,
        base_url: str | None = None,
        research_timeout_secs: float = 1800.0,
    ) -> None: ...
    def ask(
        self,
        question: str,
        *,
        model_id: str | None = None,
        session_id: str | None = None,
        data_store_ids: list[str] | None = None,
        web_grounding: bool = False,
    ) -> GeminiAssistResponse:
        """Generate once, with no automatic POST retries or model fallback."""
    def plan_research(
        self,
        topic: str,
        *,
        data_store_ids: list[str] | None = None,
        web_grounding: bool = False,
    ) -> dict[str, Any]:
        """New Deep Research plan; no automatic start. Returns session, contents,
        report_indices, final_text_candidate_index, warnings. Public v1 may omit stages."""
    def start_research(
        self,
        session_id: str,
        *,
        data_store_ids: list[str] | None = None,
        web_grounding: bool = False,
    ) -> dict[str, Any]:
        """Start ONCE in the plan's session with the same sources; may take 15+ minutes.
        No retries/model override. Returned contents keep text/grounding/file metadata
        separate; only labelled reports enter report_indices. Never assume a candidate
        is a completed report. Datastore selection is not folder isolation."""
    def get_research(self, session_id: str) -> dict[str, Any]:
        """Recover the latest saved assistant answer without generation. requested_phase
        is read and requested_agent is None; this does not attest original routing."""
    def get_session(
        self, session_id: str, *, include_answer_details: bool = False
    ) -> dict[str, Any]:
        """Read a saved session, including persisted model selection; never generates."""
    def search(
        self,
        terms: str,
        *,
        sharepoint_folder: str | None = None,
        data_store_ids: list[str] | None = None,
        page_size: int = 10,
    ) -> dict[str, Any]:
        """Search without generating: {query, hits[{document, title, uri, file_type,
        content, snippet}], total_size, more_results, warnings}. With a folder (which
        requires data_store_ids), results are restricted to it."""
    def answer_in_folder(
        self,
        question: str,
        *,
        search_terms: str,
        sharepoint_folder: str,
        data_store_ids: list[str],
        page_size: int = 10,
        session_id: str | None = None,
    ) -> dict[str, Any]:
        """Answer only from files in one SharePoint folder: {answer, session,
        answer_resource, folder, search_query, citations, references, sources, warnings}.
        data_store_ids must contain exactly one SharePoint datastore. A follow-up may
        pass a session returned earlier by this same client instance; arbitrary or
        cross-process session IDs are rejected. Each turn searches the folder again with
        that turn's search_terms."""

class GeminiAssistResponse:
    @property
    def answer(self) -> str: ...
    @property
    def session(self) -> str: ...
    @property
    def answer_resource(self) -> str: ...
    @property
    def requested_model(self) -> str | None: ...
    @property
    def grounding(self) -> list[dict[str, Any]]: ...
    @property
    def native_reference_count(self) -> int: ...
    @property
    def invocation_tools(self) -> list[str]: ...
    @property
    def warnings(self) -> list[str]: ...
    def to_dict(self) -> dict[str, Any]: ...
