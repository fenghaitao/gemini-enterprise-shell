"""Official Gemini Enterprise assistant, separate from the NotebookLM notebook API."""

from typing import Any

from ._auth import TokenProvider

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
    def get_session(
        self, session_id: str, *, include_answer_details: bool = False
    ) -> dict[str, Any]:
        """Read a saved session, including persisted model selection; never generates."""

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
