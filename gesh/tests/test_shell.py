from __future__ import annotations

import io
import threading
from dataclasses import dataclass, field
from typing import Any

import pytest
from rich.console import Console

from gesh.cli import build_config, parse_args
from gesh.config import Config
from gesh.render import citations, clean_snippet
from gesh.shell import Shell, _Abandoned

ENGINE = "projects/1/locations/us/collections/default_collection/engines/eng"


@dataclass
class FakeResponse:
    answer: str
    session: str
    grounding: list[dict[str, Any]] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    requested_model: str | None = None
    invocation_tools: list[str] = field(default_factory=list)


class FakeError(Exception):
    def __init__(self, message: str, session: str | None = None) -> None:
        super().__init__(message)
        self.session = session
        self.do_not_retry = True


class FakeChat:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.fail: Exception | None = None

    def ask(self, question: str, **kwargs: Any) -> FakeResponse:
        self.calls.append({"question": question, **kwargs})
        if self.fail:
            raise self.fail
        ref = {
            "content": "<ddd/><c0>COR</c0>-AP text",
            "documentMetadata": {"title": "H-0001.txt", "uri": "https://sp/H-0001.txt"},
        }
        return FakeResponse(
            answer=f"answer to {question}",
            session=kwargs.get("session_id") or f"{ENGINE}/sessions/42",
            grounding=[{"references": [ref, ref]}],
            requested_model=kwargs.get("model_id"),
        )

    def get_session(self, session_id: str, include_answer_details: bool = False) -> dict[str, Any]:
        return {
            "name": f"{ENGINE}/sessions/{session_id.rsplit('/', 1)[-1]}",
            "displayName": "demo",
            "turns": [
                {
                    "query": {"text": "q1"},
                    "queryConfig": {
                        "google.discoveryengine.googleapis.com.Assistant.model_id": "m1"
                    },
                    "detailedAssistAnswer": {
                        "replies": [{"groundedContent": {"content": {"text": "a1"}}}]
                    }
                    if include_answer_details
                    else None,
                }
            ],
        }


@dataclass
class FakeNotebook:
    title: str
    notebook_id: str
    name: str
    emoji: str | None = None
    metadata: Any = None
    sources: list[Any] = field(default_factory=list)


@dataclass
class FakeList:
    notebooks: list[FakeNotebook]


@dataclass
class FakeDelete:
    deleted_notebooks: list[str]
    failed_notebooks: list[str] = field(default_factory=list)


class FakeNotebooks:
    def __init__(self) -> None:
        self.deleted: list[list[str]] = []
        self.created: list[str] = []

    def list_recently_viewed(self, page_size: int | None = None) -> FakeList:
        return FakeList(
            [
                FakeNotebook("Alpha", "a1", "projects/1/locations/us/notebooks/a1"),
                FakeNotebook("Beta", "b2", "projects/1/locations/us/notebooks/b2"),
            ]
        )

    def create_notebook(self, title: str) -> FakeNotebook:
        self.created.append(title)
        return FakeNotebook(title, "new1", "projects/1/locations/us/notebooks/new1")

    def delete_notebooks(self, names: list[str]) -> FakeDelete:
        self.deleted.append(names)
        return FakeDelete(names)


def sync_runner(func: Any, label: str) -> Any:
    return func()


def make_shell(
    confirm: bool = True, runner: Any = sync_runner
) -> tuple[Shell, FakeChat, FakeNotebooks, io.StringIO]:
    out = io.StringIO()
    console = Console(file=out, width=120, color_system=None)
    chat, nbs = FakeChat(), FakeNotebooks()
    config = Config(project_number="1", engine_id="eng", model="m1", data_stores=["ds1"])
    shell = Shell(
        config,
        console,
        chat_factory=lambda: chat,
        notebook_factory=lambda: nbs,
        confirm=lambda prompt: confirm,
        runner=runner,
    )
    return shell, chat, nbs, out


def test_ask_continues_session_and_resends_model_and_datastores() -> None:
    shell, chat, _, out = make_shell()
    shell.handle("first question")
    shell.handle("second question")
    assert shell.session == f"{ENGINE}/sessions/42"
    assert chat.calls[0]["session_id"] is None
    assert chat.calls[1]["session_id"] == f"{ENGINE}/sessions/42"
    assert all(c["model_id"] == "m1" and c["data_store_ids"] == ["ds1"] for c in chat.calls)
    text = out.getvalue()
    assert "answer to first question" in text
    assert text.count("https://sp/H-0001.txt") == 2  # deduped within each answer
    assert "Sources (1)" in text


def test_new_session_and_settings_commands() -> None:
    shell, chat, _, _ = make_shell()
    shell.handle("q")
    shell.handle("/new")
    shell.handle("/model default")
    shell.handle("/ds add ds2 ds3")
    shell.handle("/ds rm ds1")
    shell.handle("/web on")
    shell.handle("q2")
    call = chat.calls[-1]
    assert call["session_id"] is None
    assert call["model_id"] is None
    assert call["data_store_ids"] == ["ds2", "ds3"]
    assert call["web_grounding"] is True
    shell.handle("/ds clear")
    shell.handle("q3")
    assert chat.calls[-1]["data_store_ids"] is None


def test_failed_generation_adopts_session_and_is_not_retried() -> None:
    shell, chat, _, out = make_shell()
    chat.fail = FakeError("generation failed; Reauthentication failed", f"{ENGINE}/sessions/7")
    shell.handle("q")
    assert len(chat.calls) == 1
    assert shell.session == f"{ENGINE}/sessions/7"
    text = out.getvalue()
    assert "Not retried" in text and "gcloud auth login" in text


def test_session_switch_and_history() -> None:
    shell, _, _, out = make_shell()
    shell.handle("/session 99")
    assert shell.session == f"{ENGINE}/sessions/99"
    shell.handle("/history")
    text = out.getvalue()
    assert "q1" in text and "a1" in text and "model m1" in text


def test_save_transcript(tmp_path: Any) -> None:
    shell, _, _, _ = make_shell()
    shell.handle("hello")
    target = tmp_path / "t.md"
    shell.handle(f"/save {target}")
    content = target.read_text()
    assert "## hello" in content and "[H-0001.txt](https://sp/H-0001.txt)" in content


def test_notebooks_list_create_and_delete_by_number() -> None:
    shell, _, nbs, out = make_shell()
    shell.handle("/nb list")
    assert "Alpha" in out.getvalue() and "Beta" in out.getvalue()
    shell.handle("/nb delete 2")
    assert nbs.deleted == [["projects/1/locations/us/notebooks/b2"]]
    shell.handle("/nb create My notebook")
    assert nbs.created == ["My notebook"]


def test_notebook_delete_requires_confirmation() -> None:
    shell, _, nbs, out = make_shell(confirm=False)
    shell.handle("/nb list")
    shell.handle("/nb delete 1")
    assert nbs.deleted == []
    assert "Cancelled" in out.getvalue()


def test_missing_engine_is_a_clear_error() -> None:
    out = io.StringIO()
    shell = Shell(Config(project_number="1"), Console(file=out), chat_factory=FakeChat)
    shell.handle("question")
    assert "engine ID" in out.getvalue()


def test_unknown_command_and_quit() -> None:
    shell, _, _, out = make_shell()
    assert shell.handle("/bogus") is True
    assert "unknown command" in out.getvalue()
    assert shell.handle("/quit") is False


def test_abandoned_ask_is_reported_later_and_blocks_new_asks() -> None:
    release = threading.Event()
    result: dict[str, Any] = {}

    def abandon_runner(func: Any, label: str) -> Any:
        def target() -> None:
            release.wait(5)
            result["value"] = func()

        thread = threading.Thread(target=target, daemon=True)
        thread.start()
        raise _Abandoned(thread, result)

    shell, chat, _, out = make_shell(runner=abandon_runner)
    shell.handle("slow question")
    assert shell.pending is not None and "Stopped waiting" in out.getvalue()
    shell.handle("another")  # refused while the first is pending
    assert "still running" in out.getvalue()
    release.set()
    shell.pending.thread.join(5)
    shell._run = sync_runner
    shell.handle("/config")  # any input polls the late result
    assert shell.pending is None
    assert "Late result for: slow question" in out.getvalue()
    assert shell.session == f"{ENGINE}/sessions/42"
    assert [c["question"] for c in chat.calls] == ["slow question"]


def test_clean_snippet_and_citation_dedupe() -> None:
    assert clean_snippet("<ddd/><c0>COR</c0>-<c1>AP</c1>  x") == "COR-AP x"
    refs = citations([{"references": [{"documentMetadata": {"uri": "u", "title": "t"}}] * 3}])
    assert len(refs) == 1


def test_config_from_env_and_flags(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NBLM_PROJECT_NUMBER", "123")
    monkeypatch.setenv("NBLM_ENGINE_ID", "nblm-engine")
    monkeypatch.setenv("GESH_ENGINE_ID", "gesh-engine")
    monkeypatch.setenv("NBLM_DATA_STORE_ID", "ds-env")
    config = build_config(parse_args(["--model", "m9"]))
    assert config.project_number == "123"
    assert config.engine_id == "gesh-engine"  # GESH_ wins over NBLM_
    assert config.data_stores == ["ds-env"]
    assert config.model == "m9"
    config = build_config(parse_args(["--data-store-id", "a", "--data-store-id", "a"]))
    assert config.data_stores == ["a"]


def test_help_shows_literal_brackets() -> None:
    shell, _, _, out = make_shell()
    shell.handle("/help")
    assert "/web [on|off]" in out.getvalue()


def test_skipped_question_is_explained_without_retry_warning() -> None:
    shell, chat, _, out = make_shell()
    err = Exception(
        "Gemini Enterprise skipped this question (NON_ASSIST_SEEKING_QUERY_IGNORED): "
        "the assistant ignores greetings and small talk; ask a concrete question"
    )
    err.skipped_reasons = ["NON_ASSIST_SEEKING_QUERY_IGNORED"]  # type: ignore[attr-defined]
    err.session = None  # type: ignore[attr-defined]
    chat.fail = err
    shell.handle("Hello")
    text = out.getvalue()
    assert "ignores greetings" in text
    assert "Not retried" not in text
    assert shell.session is None


def test_real_clients_come_from_the_two_independent_sdks() -> None:
    import gemini_chat
    import nblm

    from gesh.cli import build_shell

    config = Config(project_number="1", engine_id="eng", location="us")
    shell = build_shell(config, Console(file=io.StringIO()))
    assert isinstance(shell.chat, gemini_chat.GeminiEnterpriseClient)
    assert isinstance(shell.notebooks, nblm.NblmClient)
    assert not hasattr(nblm, "GeminiEnterpriseClient")
