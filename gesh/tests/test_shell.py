from __future__ import annotations

import io
import json
import os
import stat
import threading
from dataclasses import dataclass, field
from typing import Any

import pytest
from rich.console import Console

from gesh.cli import _prompt_history, build_config, main, parse_args
from gesh.config import Config
from gesh.render import citations, clean_snippet, render_research
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
        self.history_calls: list[str] = []
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

    def answer_in_folder(self, question: str, **kwargs: Any) -> dict[str, Any]:
        self.calls.append({"question": question, "folder": True, **kwargs})
        folder = kwargs["sharepoint_folder"].rstrip("/")
        session = kwargs.get("session_id") or f"{ENGINE}/sessions/f{len(self.calls)}"
        return {
            "answer": f"folder answer to {question}",
            "session": session,
            "answer_resource": f"{session}/answers/1",
            "folder": folder,
            "search_query": f'({kwargs["search_terms"]}) path:"{folder}"',
            "citations": [{"start_index": None, "end_index": 5, "reference_ids": [0]}],
            "references": [
                {"title": "A-0001", "uri": f"{folder}/a.txt", "content": "<c0>Alpha</c0> chunk"},
                {"title": "A-0001", "uri": f"{folder}/a.txt", "content": "dup"},
            ],
            "sources": [{"title": "A-0001", "uri": f"{folder}/a.txt", "full_text": True}],
            "warnings": ["1 of 2 files were sent as short snippets only"],
        }

    def search(self, terms: str, **kwargs: Any) -> dict[str, Any]:
        self.calls.append({"search": terms, **kwargs})
        return {
            "query": terms,
            "hits": [{"title": "A-0001", "uri": "u", "content": "x"}],
            "warnings": [],
        }

    def plan_research(self, topic: str, **kwargs: Any) -> dict[str, Any]:
        self.calls.append({"research": "plan", "topic": topic, **kwargs})
        if self.fail:
            raise self.fail
        return _research_result("plan")

    def start_research(self, session_id: str, **kwargs: Any) -> dict[str, Any]:
        self.calls.append({"research": "start", "session_id": session_id, **kwargs})
        if self.fail:
            raise self.fail
        return _research_result("start")

    def get_research(self, session_id: str) -> dict[str, Any]:
        self.history_calls.append(session_id)
        response = _research_result("read")
        response["session"] = f"{ENGINE}/sessions/{session_id.rsplit('/', 1)[-1]}"
        return response

    def get_session(self, session_id: str, include_answer_details: bool = False) -> dict[str, Any]:
        self.history_calls.append(session_id)
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


def test_numeric_notebook_reference_requires_prior_list() -> None:
    shell, _, _, out = make_shell()
    shell.handle("/nb add-file 1 /does/not/exist")
    assert shell.last_command_failed is True
    assert "run /nb list first" in out.getvalue()
    assert "no such file" not in out.getvalue()

    shell.handle("/nb list")
    out.seek(0)
    out.truncate(0)
    shell.handle("/nb add-file 1 /does/not/exist")
    assert "no such file" in out.getvalue()


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
    assert shell.last_command_failed is True
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


def test_abandoned_ask_snapshots_session_and_late_result_cannot_hijack_new_session() -> None:
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
    original = f"{ENGINE}/sessions/7"
    shell.session = original
    shell.handle("slow question")
    shell.handle("/new")
    assert shell.session is None

    release.set()
    assert shell.pending is not None
    shell.pending.thread.join(5)
    shell._run = sync_runner
    shell.handle("/config")  # polls the late result

    assert chat.calls[0]["session_id"] == original
    assert shell.session is None
    assert "Active session unchanged" in out.getvalue()


def test_scripted_errors_return_nonzero_and_success_returns_zero() -> None:
    assert main(["-c", "/bogus"]) == 1
    assert main(["-c", "/config"]) == 0


def test_history_is_private_and_can_be_disabled(
    tmp_path: Any, monkeypatch: pytest.MonkeyPatch
) -> None:
    enabled_root = tmp_path / "enabled"
    monkeypatch.setenv("XDG_STATE_HOME", str(enabled_root))
    history = _prompt_history(True)
    from prompt_toolkit.history import FileHistory, InMemoryHistory

    assert isinstance(history, FileHistory)
    state = enabled_root / "gesh"
    history_path = state / "history"
    assert history_path.is_file()
    if os.name == "posix":
        assert stat.S_IMODE(state.stat().st_mode) == 0o700
        assert stat.S_IMODE(history_path.stat().st_mode) == 0o600

    disabled_root = tmp_path / "disabled"
    monkeypatch.setenv("XDG_STATE_HOME", str(disabled_root))
    history = _prompt_history(False)
    assert isinstance(history, InMemoryHistory)
    assert not (disabled_root / "gesh").exists()


def test_no_history_flag_is_parsed() -> None:
    assert parse_args(["--no-history"]).no_history is True


FOLDER = "https://example.sharepoint.com/sites/eng/Shared%20Documents/Team/Specs"


def test_folder_mode_answers_only_from_the_folder_without_touching_the_session(
    tmp_path: Any,
) -> None:
    shell, chat, _, out = make_shell()
    shell.session = f"{ENGINE}/sessions/9"
    shell.handle(f"/folder {FOLDER}")
    assert shell.prompt_label() == "gesh[folder:Specs]> "
    shell.handle("What does A say?")
    assert "set /terms" in out.getvalue() and shell.last_command_failed
    shell.handle("/terms alpha sync")
    assert "folder Specs" in shell.toolbar() and "terms alpha sync" in shell.toolbar()
    shell.handle("What does A say?")
    call = chat.calls[-1]
    assert call["folder"] and call["search_terms"] == "alpha sync"
    assert call["sharepoint_folder"] == FOLDER and call["data_store_ids"] == ["ds1"]
    text = out.getvalue()
    assert "folder answer to What does A say?" in text
    assert "Sources (1)" in text  # duplicate references collapse to one file
    folder_session = f"{ENGINE}/sessions/f1"
    assert "folder Specs · 1 files (1 full text) · session f1" in text
    assert shell.folder_session == folder_session
    assert shell.session == f"{ENGINE}/sessions/9"  # chat session untouched
    assert "session f1" in shell.toolbar()
    # A follow-up continues the folder session and still sends the folder scope.
    shell.handle("Shorten that.")
    assert chat.calls[-1]["session_id"] == folder_session
    assert chat.calls[-1]["sharepoint_folder"] == FOLDER
    shell.handle("/new")
    assert shell.folder_session is None and shell.session == f"{ENGINE}/sessions/9"
    shell.handle("Fresh question.")
    assert chat.calls[-1]["session_id"] is None
    shell.handle("/refs")
    assert "Alpha chunk" in out.getvalue()
    target = tmp_path / "t.md"
    shell.handle(f"/save {target}")
    assert "_folder-scoped: " + FOLDER + " · terms alpha sync · session f1_" in target.read_text()
    shell.handle("/folder off")
    shell.handle("back to chat")
    assert "folder" not in chat.calls[-1] and chat.calls[-1]["session_id"] == f"{ENGINE}/sessions/9"


def test_folder_mode_needs_a_datastore_and_search_uses_folder_and_terms() -> None:
    shell, chat, _, out = make_shell()
    shell.handle(f"/folder {FOLDER}")
    shell.handle("/terms alpha")
    shell.handle("/ds clear")
    shell.handle("q?")
    assert "needs exactly one datastore" in out.getvalue()
    shell.handle("/ds add ds1")
    shell.handle("/search")
    call = chat.calls[-1]
    assert call["search"] == "alpha" and call["sharepoint_folder"] == FOLDER
    assert "1 file(s)" in out.getvalue()
    shell.handle("/search beta gamma")
    assert chat.calls[-1]["search"] == "beta gamma"
    shell.handle("/ds add ds2")
    before = len(chat.calls)
    shell.handle("/search gamma")
    assert shell.last_command_failed and "exactly one datastore" in out.getvalue()
    assert len(chat.calls) == before
    shell.handle("q with too many stores")
    assert shell.last_command_failed and len(chat.calls) == before


def test_folder_flags_and_env_are_read(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GESH_SHAREPOINT_FOLDER", FOLDER)
    monkeypatch.setenv("GESH_SEARCH_TERMS", "env terms")
    config = build_config(parse_args([]))
    assert config.sharepoint_folder == FOLDER and config.search_terms == "env terms"
    config = build_config(parse_args(["--search-terms", "flag terms"]))
    assert config.search_terms == "flag terms"


def test_folder_urls_are_validated_up_front_and_search_needs_a_datastore() -> None:
    out = io.StringIO()

    def normalize(url: str) -> str:
        if "/:f:/" in url:
            raise ValueError("sharing links cannot be resolved locally")
        return url.replace(" ", "%20").rstrip("/")

    chat = FakeChat()
    shell = Shell(
        Config(project_number="1", engine_id="eng"),
        Console(file=out, width=120, color_system=None),
        chat_factory=lambda: chat,
        runner=sync_runner,
        normalize_folder=normalize,
    )
    shell.handle("/folder https://example.sharepoint.com/:f:/s/eng/Eabc")
    assert shell.last_command_failed and "invalid folder URL" in out.getvalue()
    assert shell.config.sharepoint_folder is None
    shell.handle("/folder https://example.sharepoint.com/sites/eng/Shared Documents/Specs/")
    assert shell.config.sharepoint_folder == (
        "https://example.sharepoint.com/sites/eng/Shared%20Documents/Specs"
    )
    shell.handle("/search alpha")
    assert "needs exactly one datastore" in out.getvalue() and chat.calls == []


def test_folder_session_resets_on_folder_change_and_history_reads_it() -> None:
    shell, chat, _, out = make_shell()
    shell.handle(f"/folder {FOLDER}")
    shell.handle("/terms alpha")
    shell.handle("q1")
    first = shell.folder_session
    assert first
    shell.handle(f"/folder {FOLDER}")  # same folder: keep the conversation
    assert shell.folder_session == first
    shell.handle("/history")
    assert chat.history_calls[-1] == first
    shell.handle("/folder https://example.sharepoint.com/sites/eng/Shared%20Documents/Other")
    assert shell.folder_session is None


def test_folder_answer_turns_render_in_history() -> None:
    from gesh.render import render_session

    out = io.StringIO()
    render_session(
        Console(file=out, width=120, color_system=None),
        {
            "name": f"{ENGINE}/sessions/f1",
            "turns": [
                {
                    "query": {"text": "q1"},
                    "answer": "a/1",
                    "detailedAnswer": {"answerText": "folder reply"},
                }
            ],
        },
    )
    assert "folder answer" in out.getvalue() and "folder reply" in out.getvalue()


OTHER = "https://example.sharepoint.com/sites/eng/Shared%20Documents/Other"


def test_late_answer_from_a_previous_folder_is_marked_and_not_made_current(tmp_path: Any) -> None:
    release = threading.Event()
    result: dict[str, Any] = {}

    def abandon_runner(func: Any, label: str) -> Any:
        def target() -> None:
            release.wait(5)
            result["value"] = func()

        thread = threading.Thread(target=target, daemon=True)
        thread.start()
        raise _Abandoned(thread, result)

    shell, _, _, out = make_shell(runner=abandon_runner)
    shell.handle(f"/folder {FOLDER}")
    shell.handle("/terms alpha")
    shell.handle("question for A")  # Ctrl-C: still running
    assert shell.pending is not None and shell.pending.folder == FOLDER
    shell._run = sync_runner
    shell.handle(f"/folder {OTHER}")  # switch to folder B
    release.set()
    shell.pending.thread.join(5)
    shell.handle("/config")  # polls the late result

    text = out.getvalue()
    assert "Late result from folder Specs (terms: alpha)" in text
    assert shell.last_folder_answer is None  # /refs does not point at A's answer
    assert shell.folder_session is None  # B's session untouched
    turn = shell.transcript[-1]
    assert turn.stale and turn.folder == FOLDER and turn.search_terms == "alpha"
    target = tmp_path / "t.md"
    shell.handle(f"/save {target}")
    assert "late result" in target.read_text()


def test_session_command_is_folder_aware() -> None:
    shell, chat, _, out = make_shell()
    shell.session = f"{ENGINE}/sessions/9"
    shell.handle(f"/folder {FOLDER}")
    shell.handle("/session")
    assert "No folder session yet" in out.getvalue()
    shell.handle("/session 55")
    assert shell.last_command_failed
    assert "cannot be imported by ID" in out.getvalue()
    assert shell.folder_session is None
    assert shell.session == f"{ENGINE}/sessions/9"  # chat session untouched
    shell.handle("/terms alpha")
    shell.handle("first scoped question")
    assert chat.calls[-1]["session_id"] is None
    assert shell.folder_session
    shell.handle("/session")
    assert "Current folder session:" in out.getvalue()


def _research_result(phase: str) -> dict[str, Any]:
    return {
        "session": f"{ENGINE}/sessions/research",
        "requested_phase": phase,
        "contents": [
            {"text": "intermediate material", "kind": None, "grounding": [], "file": None},
            {"text": "final candidate", "kind": None, "grounding": [], "file": None},
        ],
        "report_indices": [],
        "final_text_candidate_index": 1,
        "warnings": ["stage labels absent; unverified"],
    }


def test_research_separate_session_explicit_start_and_pinned_sources(tmp_path: Any) -> None:
    shell, chat, _, out = make_shell()
    shell.handle("chat question")
    chat_session = shell.session
    shell.handle("/research plan test CLI clients")
    assert len(chat.calls) == 2  # no automatic start
    shell.handle("/ds clear")
    shell.handle("/ds add changed")
    shell.handle("/web on")
    shell.handle("/research start")
    assert chat.calls[-1] == {
        "research": "start",
        "session_id": f"{ENGINE}/sessions/research",
        "data_store_ids": ["ds1"],
        "web_grounding": False,
    }
    assert shell.session == chat_session
    before = len(chat.calls)
    shell.handle("/research start")
    assert shell.last_command_failed
    assert len(chat.calls) == before
    path = tmp_path / "research.json"
    shell.handle(f"/research save {path}")
    assert not shell.last_command_failed
    assert json.loads(path.read_text())["contents"][0]["text"] == "intermediate material"
    if os.name == "posix":
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
    shell.handle(f"/research save {path}")
    assert shell.last_command_failed  # no overwrite
    shell.handle("/research history")
    assert chat.history_calls[-1] == f"{ENGINE}/sessions/research"
    assert "stage unverified" in out.getvalue()


def test_research_refuses_folder_scope_and_start_without_plan() -> None:
    shell, chat, _, out = make_shell()
    shell.handle("/research start")
    assert shell.last_command_failed
    shell.handle("/folder https://example.sharepoint.com/sites/eng/Docs")
    shell.handle("/research plan topic")
    assert shell.last_command_failed
    assert not chat.calls
    assert "does not enforce folder scope" in out.getvalue()


def test_failed_research_does_not_hijack_chat_or_resend() -> None:
    shell, chat, _, _ = make_shell()
    shell.handle("/research plan topic")
    shell.session = "chat-original"
    chat.fail = FakeError("delivery uncertain", f"{ENGINE}/sessions/research")
    shell.handle("/research start")
    assert shell.last_command_failed
    assert shell.session == "chat-original"
    shell.handle("/research start")
    assert shell.last_command_failed
    assert len(chat.calls) == 2


def test_abandoned_research_blocks_generations_and_accepts_late_result() -> None:
    ready = threading.Event()
    thread = threading.Thread(target=lambda: ready.wait(2))
    thread.start()
    result: dict[str, Any] = {}

    def abandon(func: Any, label: str) -> Any:
        result["value"] = func()
        raise _Abandoned(thread, result)

    shell, chat, _, _ = make_shell(runner=abandon)
    shell.session = "chat-original"
    shell.handle("/research plan topic")
    assert shell.pending is not None
    shell.handle("/research plan another")
    assert shell.last_command_failed
    shell.handle("normal question")
    assert shell.last_command_failed
    ready.set()
    thread.join()
    shell.poll_pending()
    assert shell.pending is None
    assert shell.research_plan is not None
    assert shell.session == "chat-original"
    assert len(chat.calls) == 1
    shell._run = sync_runner
    shell.handle("/research start")
    assert not shell.last_command_failed
    assert len(chat.calls) == 2


def test_research_read_does_not_invoke_agent_or_import_start_authorization() -> None:
    shell, chat, _, _ = make_shell()
    shell.session = "chat-original"
    shell.handle("/research read saved-research")
    assert not shell.last_command_failed
    assert not chat.calls
    assert chat.history_calls == ["saved-research"]
    assert shell.last_research is not None
    assert shell.last_research["requested_phase"] == "read"
    assert shell.session == "chat-original"
    shell.handle("/research start")
    assert shell.last_command_failed
    assert not chat.calls


@pytest.mark.parametrize("mime", [{}, {"mimeType": None}, {"mimeType": "audio/mp3"}])
def test_research_render_mixed_stage_candidate_and_optional_audio_mime(
    mime: dict[str, Any],
) -> None:
    out = io.StringIO()
    result = _research_result("read")
    result["contents"] = [
        {"text": "question material", "kind": "RESEARCH_QUESTION"},
        {"text": "intermediate material", "kind": "RESEARCH_ANSWER"},
        {"text": "final candidate", "kind": None},
        {"file": {"fileId": "1234", **mime}},
    ]
    result["final_text_candidate_index"] = 2
    render_research(Console(file=out, width=120, color_system=None), result)
    text = out.getvalue()
    assert "final candidate" in text
    assert "unverified final-text candidate" in text
    assert "intermediate material" not in text
    assert "question material" not in text
    assert f"File metadata: 1234 ({mime.get('mimeType') or 'MIME type unavailable'})" in text


def test_research_render_labels_even_a_labelled_fallback_as_unverified() -> None:
    out = io.StringIO()
    result = _research_result("start")
    result["contents"][1]["kind"] = "RESEARCH_ANSWER"
    render_research(Console(file=out, width=120, color_system=None), result)
    assert "RESEARCH_ANSWER" in out.getvalue()
    assert "unverified final-text candidate" in out.getvalue()


@pytest.mark.parametrize("read_id", ["another-research", "research"])
def test_research_read_clears_an_existing_plan_start_authorization(read_id: str) -> None:
    shell, chat, _, out = make_shell()
    shell.session = "chat-original"
    shell.handle("/research plan topic A")
    assert shell.research_plan is not None
    shell.handle(f"/research read {read_id}")
    assert not shell.last_command_failed
    shell.handle("/research show")
    assert not shell.last_command_failed
    assert shell.last_research is not None
    assert shell.last_research["requested_phase"] == "read"
    assert shell.last_research["session"] == f"{ENGINE}/sessions/{read_id}"
    assert shell.research_plan is None
    assert shell._research_sources == ([], False)
    shell.handle("/research start")
    assert shell.last_command_failed
    assert len(chat.calls) == 1  # only planning; never start the old plan
    assert chat.history_calls == [read_id]
    assert shell.session == "chat-original"
    assert "create and review /research plan" in out.getvalue()
    # Explicit new planning restores authorization, using the new sources.
    shell.handle("/ds clear")
    shell.handle("/ds add changed")
    shell.handle("/research plan topic C")
    shell.handle("/research start")
    assert not shell.last_command_failed
    assert chat.calls[-1]["data_store_ids"] == ["changed"]
