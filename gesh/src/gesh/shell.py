"""Command dispatcher and chat/notebook state for the gesh REPL.

Clients are injected (or built lazily by factories) so the shell can be tested with fakes.
Generation is never retried: a failed or abandoned ask may still be recorded server-side.
"""

from __future__ import annotations

import shlex
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from rich.console import Console
from rich.markup import escape
from rich.text import Text

from . import render
from .config import Config

HELP = """\
[bold]Chat (Gemini Enterprise assistant)[/bold]
  <question>              Ask; continues the current session. Esc+Enter inserts a newline.
  /new                    Start a new session on the next question
  /session [ID]           Show the current session, or switch to an existing one
  /history                Read back the current session (questions and answers)
  /model [ID|default]     Show or set the model (default = assistant default)
  /ds                     List datastores used for retrieval
  /ds add ID [ID...]      Add datastores     /ds rm ID     /ds clear
  /web \\[on|off]           Public web grounding (needs assistant support)
  /refs                   Re-show the last answer's sources with snippets
  /save FILE              Save this shell's transcript as Markdown

[bold]Notebooks (NotebookLM Enterprise)[/bold]
  /nb list [N]            Recently viewed notebooks (numbered for later commands)
  /nb show NB             Sources of a notebook (NB = number from /nb list, or notebook ID)
  /nb create TITLE        Create a notebook
  /nb add-url NB URL [NAME]   Add a web source
  /nb add-file NB PATH        Upload a local file as a source
  /nb delete NB           Delete a notebook (asks for confirmation)

[bold]Other[/bold]
  /config  /help  /quit
"""


class ShellError(Exception):
    """A user-facing error for a single command."""


Runner = Callable[[Callable[[], Any], str], Any]


@dataclass
class _Pending:
    """An ask the user stopped waiting for; its outcome is reported when it arrives."""

    question: str
    thread: threading.Thread
    session_epoch: int
    result: dict[str, Any] = field(default_factory=dict)


@dataclass
class Turn:
    question: str
    answer: str
    session: str
    sources: list[dict[str, str]]
    warnings: list[str]


class Shell:
    def __init__(
        self,
        config: Config,
        console: Console,
        *,
        chat_factory: Callable[[], Any] | None = None,
        notebook_factory: Callable[[], Any] | None = None,
        confirm: Callable[[str], bool] | None = None,
        runner: Runner | None = None,
    ) -> None:
        self.config = config
        self.console = console
        self._chat_factory = chat_factory
        self._notebook_factory = notebook_factory
        self._chat: Any = None
        self._notebooks: Any = None
        self._confirm = confirm or (lambda prompt: input(prompt).strip().lower() in {"y", "yes"})
        self._run: Runner = runner or self._run_in_thread
        self.session: str | None = None
        self.last: Any = None
        self.transcript: list[Turn] = []
        self.notebook_cache: list[Any] = []
        self.pending: _Pending | None = None
        self.last_command_failed = False
        # Incremented only by explicit user session changes (/new or /session ID).
        # A late result from an abandoned request may update the active session only
        # if the user has not changed session context in the meantime.
        self._session_epoch = 0

    # ----------------------------------------------------------------- clients
    @property
    def chat(self) -> Any:
        if self._chat is None:
            if not self.config.project_number or not self.config.engine_id:
                raise ShellError(
                    "chat needs a project number and engine ID "
                    "(--project-number/--engine-id or NBLM_PROJECT_NUMBER/NBLM_ENGINE_ID)"
                )
            if self._chat_factory is None:
                raise ShellError("chat client is not configured")
            self._chat = self._chat_factory()
        return self._chat

    @property
    def notebooks(self) -> Any:
        if self._notebooks is None:
            if not self.config.project_number:
                raise ShellError("notebooks need a project number (--project-number)")
            if self._notebook_factory is None:
                raise ShellError("notebook client is not configured")
            self._notebooks = self._notebook_factory()
        return self._notebooks

    # --------------------------------------------------------------- dispatch
    def prompt_label(self) -> str:
        return f"gesh[{render.short_id(self.session) if self.session else 'new'}]> "

    def toolbar(self) -> str:
        c = self.config
        ds = ",".join(c.data_stores) if c.data_stores else "none"
        web = " · web" if c.web_grounding else ""
        return (
            f" engine {c.engine_id or '-'} · model {c.model or 'default'} · ds {ds}{web}"
            f" · session {render.short_id(self.session) if self.session else 'new'}"
        )

    def handle(self, line: str) -> bool:
        """Process one input line. Returns False when the shell should exit."""
        self.last_command_failed = False
        self.poll_pending()
        text = line.strip()
        if not text:
            return True
        try:
            if text.startswith("/"):
                return self._command(text)
            self.ask(line.strip())
        except ShellError as err:
            self.last_command_failed = True
            self.console.print(Text(f"error: {err}", style="red"))
        except _Abandoned:
            self.last_command_failed = True
            self.console.print(
                Text(
                    "Stopped waiting; the request may still complete on the server.", style="yellow"
                )
            )
        except KeyboardInterrupt:
            self.last_command_failed = True
            self.console.print(Text("interrupted", style="yellow"))
        except Exception as err:  # SDK errors (NblmError) and validation errors
            self.last_command_failed = True
            self._report_error(err)
        return True

    def _command(self, text: str) -> bool:
        try:
            parts = shlex.split(text)
        except ValueError as err:
            raise ShellError(f"cannot parse command: {err}") from None
        cmd, args = parts[0].lower(), parts[1:]
        if cmd in {"/quit", "/exit", "/q"}:
            return False
        handler = {
            "/help": self.cmd_help,
            "/?": self.cmd_help,
            "/new": self.cmd_new,
            "/session": self.cmd_session,
            "/history": self.cmd_history,
            "/model": self.cmd_model,
            "/ds": self.cmd_ds,
            "/web": self.cmd_web,
            "/refs": self.cmd_refs,
            "/save": self.cmd_save,
            "/config": self.cmd_config,
            "/nb": self.cmd_nb,
        }.get(cmd)
        if handler is None:
            raise ShellError(f"unknown command {cmd}; try /help")
        handler(args)
        return True

    # ------------------------------------------------------------------- chat
    def ask(self, question: str) -> None:
        if self.pending is not None:
            raise ShellError(
                "an earlier question is still running; wait for its answer before asking again"
            )
        chat = self.chat
        # Snapshot every request setting before the worker starts. After Ctrl-C the
        # daemon thread can begin or finish later while the user edits shell state.
        session_at_dispatch = self.session
        model = self.config.model
        data_stores = list(self.config.data_stores) or None
        web_grounding = self.config.web_grounding
        session_epoch = self._session_epoch

        def call() -> Any:
            return chat.ask(
                question,
                model_id=model,
                session_id=session_at_dispatch,
                data_store_ids=data_stores,
                web_grounding=web_grounding,
            )

        try:
            response = self._run(call, "Asking Gemini Enterprise")
        except _Abandoned as abandoned:
            # Interactive mode can continue, but scripted callers must see this command
            # as unsuccessful and return a nonzero status instead of silently succeeding.
            self.last_command_failed = True
            self.pending = _Pending(question, abandoned.thread, session_epoch, abandoned.result)
            where = (
                f" in session {render.short_id(session_at_dispatch)}" if session_at_dispatch else ""
            )
            self.console.print(
                Text(
                    f"Stopped waiting. The server may still record this turn{where}; "
                    "it is not resent. Its answer is shown here if it arrives.",
                    style="yellow",
                )
            )
            return
        self._accept(question, response)

    def _accept(self, question: str, response: Any, *, adopt_session: bool = True) -> None:
        if adopt_session:
            self.session = response.session
        self.last = response
        self.transcript.append(
            Turn(
                question,
                response.answer,
                response.session,
                render.citations(response.grounding),
                list(response.warnings),
            )
        )
        render.render_answer(self.console, response)

    def poll_pending(self) -> None:
        pending = self.pending
        if pending is None or pending.thread.is_alive():
            return
        self.pending = None
        self.console.print(Text(f"Late result for: {pending.question}", style="bold yellow"))
        adopt_session = pending.session_epoch == self._session_epoch
        if "error" in pending.result:
            self._report_error(pending.result["error"], adopt_session=adopt_session)
        elif "value" in pending.result:
            self._accept(
                pending.question,
                pending.result["value"],
                adopt_session=adopt_session,
            )
        if not adopt_session:
            self.console.print(
                Text(
                    "Active session unchanged because you switched session context while "
                    "that request was still running.",
                    style="dim",
                )
            )

    def cmd_new(self, args: list[str]) -> None:
        self._session_epoch += 1
        self.session = None
        self.last = None
        self.console.print("New session starts with your next question.")

    def cmd_session(self, args: list[str]) -> None:
        if not args:
            if self.session:
                self.console.print(f"Current session: {self.session}")
            else:
                self.console.print("No session yet; the next question creates one.")
            return
        session = self._run(lambda: self.chat.get_session(args[0]), "Reading session")
        self._session_epoch += 1
        self.session = session.get("name") or args[0]
        self.last = None
        render.render_session(self.console, session)
        self.console.print(Text("Continuing this session.", style="dim"))

    def cmd_history(self, args: list[str]) -> None:
        if not self.session:
            raise ShellError("no session yet")
        session_id = self.session
        session = self._run(
            lambda: self.chat.get_session(session_id, include_answer_details=True),
            "Reading session",
        )
        render.render_session(self.console, session)

    def cmd_model(self, args: list[str]) -> None:
        if args:
            self.config.model = None if args[0].lower() in {"default", "none", "-"} else args[0]
        self.console.print(f"Model: {self.config.model or 'assistant default'}")

    def cmd_ds(self, args: list[str]) -> None:
        stores = self.config.data_stores
        if args:
            action, ids = args[0].lower(), args[1:]
            if action == "add" and ids:
                stores.extend(i for i in ids if i not in stores)
            elif action in {"rm", "remove", "del"} and ids:
                missing = [i for i in ids if i not in stores]
                if missing:
                    raise ShellError(f"not in the list: {', '.join(missing)}")
                stores[:] = [s for s in stores if s not in ids]
            elif action in {"clear", "none"}:
                stores.clear()
            elif action != "list":
                raise ShellError("usage: /ds [list | add ID... | rm ID... | clear]")
        if stores:
            for s in stores:
                self.console.print(f"  {s}")
        else:
            self.console.print("No datastores: the assistant uses its default tools.")

    def cmd_web(self, args: list[str]) -> None:
        if args:
            if args[0].lower() not in {"on", "off"}:
                raise ShellError("usage: /web [on|off]")
            self.config.web_grounding = args[0].lower() == "on"
        self.console.print(f"Web grounding: {'on' if self.config.web_grounding else 'off'}")

    def cmd_refs(self, args: list[str]) -> None:
        if self.last is None:
            raise ShellError("no answer yet")
        refs = render.citations(self.last.grounding)
        if not refs:
            self.console.print("The last answer has no native references.")
            return
        for i, ref in enumerate(refs, 1):
            self.console.print(Text(f"[{i}] ", style="cyan") + Text(ref["title"], style="bold"))
            if ref["uri"]:
                self.console.print(Text("    " + ref["uri"], style="dim"))
            if ref["snippet"]:
                self.console.print(Text("    " + ref["snippet"], style="italic"))

    def cmd_save(self, args: list[str]) -> None:
        if len(args) != 1:
            raise ShellError("usage: /save FILE")
        if not self.transcript:
            raise ShellError("nothing to save yet")
        path = Path(args[0]).expanduser()
        lines = ["# gesh transcript", ""]
        for turn in self.transcript:
            lines += [f"## {turn.question}", "", f"_session {render.short_id(turn.session)}_", ""]
            lines += [turn.answer, ""]
            if turn.sources:
                lines.append("Sources:")
                lines += [
                    f"- [{s['title']}]({s['uri']})" if s["uri"] else f"- {s['title']}"
                    for s in turn.sources
                ]
                lines.append("")
            lines += [f"> {w}" for w in turn.warnings]
            lines.append("")
        path.write_text("\n".join(lines), encoding="utf-8")
        self.console.print(f"Saved {len(self.transcript)} turns to {path}")

    def cmd_config(self, args: list[str]) -> None:
        c = self.config
        rows = {
            "project number": c.project_number,
            "location": c.location,
            "notebook endpoint location": c.notebook_endpoint_location,
            "engine": c.engine_id,
            "collection / assistant": f"{c.collection_id} / {c.assistant_id}",
            "model": c.model or "assistant default",
            "datastores": ", ".join(c.data_stores) or "none",
            "web grounding": "on" if c.web_grounding else "off",
            "auth": c.auth,
            "session": self.session or "new",
        }
        for key, value in rows.items():
            self.console.print(f"  {key:<27} {value or '-'}")

    def cmd_help(self, args: list[str]) -> None:
        self.console.print(HELP)

    # -------------------------------------------------------------- notebooks
    def cmd_nb(self, args: list[str]) -> None:
        if not args:
            raise ShellError("usage: /nb list|show|create|add-url|add-file|delete ...")
        sub, rest = args[0].lower(), args[1:]
        if sub in {"list", "ls"}:
            size = int(rest[0]) if rest else 20
            resp = self._run(
                lambda: self.notebooks.list_recently_viewed(page_size=size), "Listing notebooks"
            )
            self.notebook_cache = list(resp.notebooks)
            if not self.notebook_cache:
                self.console.print("No recently viewed notebooks.")
                return
            self.console.print(render.notebooks_table(self.notebook_cache))
        elif sub == "show" and len(rest) == 1:
            nb = self._find_notebook(rest[0])
            self.console.print(render.sources_table(nb))
        elif sub == "create" and rest:
            title = " ".join(rest)
            nb = self._run(lambda: self.notebooks.create_notebook(title), "Creating notebook")
            self.console.print(f"Created {escape(nb.title)}: {nb.notebook_id}")
            self.notebook_cache = []
        elif sub == "add-url" and len(rest) in {2, 3}:
            from nblm import WebSource

            nb_id = self._notebook_id(rest[0])
            source = WebSource(url=rest[1], name=rest[2] if len(rest) == 3 else None)
            resp = self._run(
                lambda: self.notebooks.add_sources(nb_id, web_sources=[source]), "Adding source"
            )
            self.console.print(f"Added {len(resp.sources)} source(s) to {nb_id}.")
            self.notebook_cache = []
        elif sub == "add-file" and len(rest) == 2:
            nb_id = self._notebook_id(rest[0])
            path = Path(rest[1]).expanduser()
            if not path.is_file():
                raise ShellError(f"no such file: {path}")
            resp = self._run(
                lambda: self.notebooks.upload_source_file(nb_id, str(path)), "Uploading file"
            )
            sid = resp.source_id.id if resp.source_id else "?"
            self.console.print(f"Uploaded {path.name} to {nb_id} (source {sid}).")
            self.notebook_cache = []
        elif sub in {"delete", "rm"} and len(rest) == 1:
            nb = self._find_notebook(rest[0], required=False)
            nb_id = nb.notebook_id if nb is not None else rest[0]
            title = f"'{nb.title}' " if nb is not None else ""
            if not self._confirm(f"Delete notebook {title}({nb_id})? This cannot be undone [y/N] "):
                self.console.print("Cancelled.")
                return
            name = (nb.name if nb is not None else None) or self._notebook_name(nb_id)
            resp = self._run(lambda: self.notebooks.delete_notebooks([name]), "Deleting notebook")
            if resp.failed_notebooks:
                raise ShellError(f"delete failed for {', '.join(resp.failed_notebooks)}")
            self.console.print(f"Deleted {nb_id}.")
            self.notebook_cache = []
        else:
            raise ShellError(f"bad /nb usage; see /help ({' '.join(args)})")

    def _notebook_name(self, notebook_id: str) -> str:
        c = self.config
        return f"projects/{c.project_number}/locations/{c.location}/notebooks/{notebook_id}"

    def _notebook_id(self, ref: str) -> str:
        if not ref.isdigit():
            return ref
        if not self.notebook_cache:
            raise ShellError(
                "notebook numbers come from /nb list; run /nb list first or use a full notebook resource name"
            )
        nb = self._find_notebook(ref)
        return str(nb.notebook_id)

    def _find_notebook(self, ref: str, *, required: bool = True) -> Any:
        if not self.notebook_cache:
            resp = self._run(
                lambda: self.notebooks.list_recently_viewed(page_size=100), "Listing notebooks"
            )
            self.notebook_cache = list(resp.notebooks)
        if ref.isdigit() and 1 <= int(ref) <= len(self.notebook_cache):
            return self.notebook_cache[int(ref) - 1]
        for nb in self.notebook_cache:
            if ref in {nb.notebook_id, nb.name}:
                return nb
        if required:
            raise ShellError(f"notebook {ref} not found among recently viewed notebooks")
        return None

    # ----------------------------------------------------------------- helpers
    def _report_error(self, err: BaseException, *, adopt_session: bool = True) -> None:
        session = getattr(err, "session", None)
        if session and adopt_session:
            self.session = session
        self.console.print(Text(f"error: {err}", style="red"))
        if getattr(err, "do_not_retry", False):
            self.console.print(
                Text(
                    "Not retried. Check /history before asking again"
                    + (f" (session {render.short_id(session)})." if session else "."),
                    style="yellow",
                )
            )
        message = str(err).lower()
        if "reauthentication" in message or "gcloud auth login" in message:
            self.console.print(Text("Hint: run `gcloud auth login`, then try again.", style="cyan"))

    def _run_in_thread(self, func: Callable[[], Any], label: str) -> Any:
        """Run a blocking SDK call in a daemon thread so the spinner and Ctrl-C keep working."""
        result: dict[str, Any] = {}

        def target() -> None:
            try:
                result["value"] = func()
            except BaseException as err:  # noqa: BLE001 - re-raised in the caller
                result["error"] = err

        thread = threading.Thread(target=target, name="gesh-call", daemon=True)
        start = time.monotonic()
        thread.start()
        try:
            with self.console.status(f"{label}…") as status:
                while thread.is_alive():
                    thread.join(0.2)
                    status.update(
                        f"{label}… {time.monotonic() - start:.0f}s (Ctrl-C to stop waiting)"
                    )
        except KeyboardInterrupt:
            raise _Abandoned(thread, result) from None
        if "error" in result:
            raise result["error"]
        return result["value"]


class _Abandoned(Exception):
    def __init__(self, thread: threading.Thread, result: dict[str, Any]) -> None:
        super().__init__("abandoned")
        self.thread = thread
        self.result = result
