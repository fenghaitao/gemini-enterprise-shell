"""Official Gemini Enterprise bindings against a local, synthetic HTTP server."""

import json
import threading
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import ClassVar

import pytest

from gemini_chat import (
    EnvTokenProvider,
    GcloudTokenProvider,
    GeminiAssistResponse,
    GeminiChatError,
    GeminiEnterpriseClient,
)

ENGINE = "projects/123/locations/us/collections/default_collection/engines/search"
SESSION = f"{ENGINE}/sessions/456"


class _Handler(BaseHTTPRequestHandler):
    requests: ClassVar[list[tuple[str, str, dict[str, object] | None]]] = []
    status: ClassVar[int] = 200
    cited: ClassVar[bool] = True
    skipped: ClassVar[bool] = False

    def log_message(self, format: str, *args: object) -> None:  # noqa: ARG002
        return

    def _send(self, value: object) -> None:
        raw = json.dumps(value).encode()
        self.send_response(type(self).status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        type(self).requests.append(("POST", self.path, body))
        assert self.headers["Authorization"] == "Bearer synthetic-token"
        assert self.headers["x-goog-user-project"] == "123"
        assert self.headers.get("Cookie") is None
        if type(self).status != 200:
            self._send({"error": {"message": "sensitive server text"}})
            return
        if type(self).skipped:  # live shape for small talk such as "Hello"
            self._send(
                [
                    {
                        "answer": {
                            "state": "SKIPPED",
                            "assistSkippedReasons": ["NON_ASSIST_SEEKING_QUERY_IGNORED"],
                        }
                    }
                ]
            )
            return
        grounding = (
            {
                "references": [
                    {"documentMetadata": {"uri": "https://example.test/doc", "title": "Doc"}}
                ],
                "segments": [{"referenceIndices": [0], "startIndex": "0", "endIndex": "6"}],
            }
            if type(self).cited
            else None
        )
        replies: list[dict[str, object]] = [{"groundedContent": {"content": {"text": "Answer"}}}]
        if grounding:
            replies.append({"groundedContent": {"textGroundingMetadata": grounding}})
        self._send(
            [
                {
                    "answer": {
                        "name": f"{SESSION}/assistAnswers/789",
                        "state": "SUCCEEDED",
                        "replies": replies,
                    },
                    "sessionInfo": {"session": SESSION},
                }
            ]
        )

    def do_GET(self) -> None:
        type(self).requests.append(("GET", self.path, None))
        self._send(
            {
                "name": SESSION,
                "turns": [
                    {
                        "query": {"text": "q"},
                        "queryConfig": {
                            "google.discoveryengine.googleapis.com.Assistant.model_id": "gemini-4-argon"
                        },
                    }
                ],
            }
        )


@pytest.fixture()
def server() -> Iterator[str]:
    _Handler.requests = []
    _Handler.status = 200
    _Handler.cited = True
    _Handler.skipped = False
    httpd = HTTPServer(("127.0.0.1", 0), _Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{httpd.server_address[1]}/v1"
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join()


def _client(monkeypatch: pytest.MonkeyPatch, server: str) -> GeminiEnterpriseClient:
    monkeypatch.setenv("GEMINI_CHAT_TEST_TOKEN", "synthetic-token")
    return GeminiEnterpriseClient(
        EnvTokenProvider("GEMINI_CHAT_TEST_TOKEN"), "123", "search", "us", base_url=server
    )


def test_explicit_model_datastore_native_grounding_and_continuation(
    monkeypatch: pytest.MonkeyPatch, server: str
) -> None:
    client = _client(monkeypatch, server)
    result = client.ask("q", model_id="gemini-4-argon", data_store_ids=["sharepoint_file"])
    assert isinstance(result, GeminiAssistResponse)
    assert result.answer == "Answer"
    assert result.session == SESSION
    assert result.requested_model == "gemini-4-argon"
    assert result.native_reference_count == 1
    assert (
        result.grounding[0]["references"][0]["documentMetadata"]["uri"]
        == "https://example.test/doc"
    )
    assert result.warnings == []
    assert result.to_dict()["session"] == SESSION
    request = _Handler.requests[0][2]
    assert request is not None
    assert request["generationSpec"] == {"modelId": "gemini-4-argon"}
    client.ask("q2", model_id="gemini-4-argon", session_id=result.session)
    request = _Handler.requests[1][2]
    assert request is not None and request["session"] == SESSION
    saved = client.get_session(result.session, include_answer_details=True)
    assert (
        saved["turns"][0]["queryConfig"]["google.discoveryengine.googleapis.com.Assistant.model_id"]
        == "gemini-4-argon"
    )
    assert _Handler.requests[-1][0] == "GET"
    assert _Handler.requests[-1][1].endswith("?includeAnswerDetails=true")


def test_no_grounding_is_explicit(monkeypatch: pytest.MonkeyPatch, server: str) -> None:
    _Handler.cited = False
    result = _client(monkeypatch, server).ask("q")
    assert result.native_reference_count == 0
    assert result.grounding == []
    assert result.warnings and "No native grounding" in result.warnings[0]


def test_skipped_small_talk_explains_why(monkeypatch: pytest.MonkeyPatch, server: str) -> None:
    _Handler.skipped = True
    with pytest.raises(GeminiChatError, match="ignores greetings") as exc:
        _client(monkeypatch, server).ask("Hello")
    assert getattr(exc.value, "skipped_reasons", None) == ["NON_ASSIST_SEEKING_QUERY_IGNORED"]
    assert getattr(exc.value, "session", "unset") is None
    assert not getattr(exc.value, "do_not_retry", False)
    assert "may have recorded" not in str(exc.value)


@pytest.mark.parametrize("status", [400, 401, 403, 429, 503])
def test_errors_have_no_retry_or_fallback(
    monkeypatch: pytest.MonkeyPatch, server: str, status: int
) -> None:
    _Handler.status = status
    with pytest.raises(GeminiChatError, match="do not automatically repeat") as exc:
        _client(monkeypatch, server).ask("q", session_id="456", model_id="gemini-4-argon")
    assert getattr(exc.value, "session", None) == SESSION
    assert getattr(exc.value, "do_not_retry", False) is True
    assert "sensitive server text" not in str(exc.value)
    assert len(_Handler.requests) == 1


def test_validation_is_local_and_timeout_values_never_panic(
    monkeypatch: pytest.MonkeyPatch, server: str
) -> None:
    client = _client(monkeypatch, server)
    for session in [
        "../bad",
        "projects/999/locations/us/collections/default_collection/engines/search/sessions/456",
    ]:
        with pytest.raises(GeminiChatError, match="invalid enterprise session"):
            client.ask("q", session_id=session)
    for timeout in [float("nan"), float("inf"), -1.0, 0.0]:
        with pytest.raises((ValueError, GeminiChatError), match="positive"):
            GeminiEnterpriseClient(EnvTokenProvider(), "123", "search", timeout_secs=timeout)
    assert _Handler.requests == []


def test_public_api_and_token_providers_are_independent_of_nblm() -> None:
    import sys

    assert EnvTokenProvider().key == "GEMINI_CHAT_ACCESS_TOKEN"
    assert GcloudTokenProvider().binary == "gcloud"
    with pytest.raises(TypeError, match="token_provider"):
        GeminiEnterpriseClient(object(), "123", "search")  # type: ignore[arg-type]
    assert "nblm" not in sys.modules
