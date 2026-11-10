"""`gesh` entry point: flags, client construction and the input loop."""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path
from typing import TYPE_CHECKING

from rich.console import Console

if TYPE_CHECKING:
    from prompt_toolkit.history import History

from . import __version__
from .config import Config
from .shell import Shell


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="gesh",
        description="Line-based shell for Gemini Enterprise: chat, Deep Research and NotebookLM "
        "Enterprise notebooks. Settings default to GESH_* then NBLM_* environment variables.",
    )
    p.add_argument("--project-number", help="Google Cloud project number [PROJECT_NUMBER]")
    p.add_argument("--location", help="Location, e.g. us, eu, global [LOCATION]")
    p.add_argument("--endpoint-location", help="NotebookLM endpoint location [ENDPOINT_LOCATION]")
    p.add_argument("--engine-id", help="Gemini Enterprise engine (app) ID [ENGINE_ID]")
    p.add_argument("--model", help="Model ID, e.g. gemini-4-argon [ENTERPRISE_MODEL]")
    p.add_argument(
        "--data-store-id",
        action="append",
        dest="data_stores",
        help="Retrieval datastore; repeatable [DATA_STORE_ID or comma-separated DATA_STORE_IDS]",
    )
    p.add_argument("--web-grounding", action="store_true", help="Enable public web grounding")
    p.add_argument(
        "--sharepoint-folder",
        help="Answer only from files in this SharePoint folder [SHAREPOINT_FOLDER]",
    )
    p.add_argument("--search-terms", help="Keywords that find files in the folder [SEARCH_TERMS]")
    p.add_argument("--session-id", help="Continue an existing assistant session")
    p.add_argument(
        "--auth",
        choices=["gcloud", "env"],
        default="gcloud",
        help="gcloud (default) or env (GEMINI_CHAT_ACCESS_TOKEN for chat, NBLM_ACCESS_TOKEN "
        "for notebooks)",
    )
    p.add_argument("--timeout", type=float, default=360.0, help="Ask timeout in seconds")
    p.add_argument("-c", "--command", action="append", help="Run a line and exit; repeatable")
    p.add_argument(
        "--no-history",
        action="store_true",
        help="Do not persist interactive prompt history",
    )
    p.add_argument("--version", action="version", version=f"gesh {__version__}")
    return p.parse_args(argv)


def build_config(args: argparse.Namespace) -> Config:
    config = Config.from_env()
    for attr in (
        "project_number",
        "location",
        "endpoint_location",
        "engine_id",
        "model",
        "sharepoint_folder",
        "search_terms",
    ):
        value = getattr(args, attr)
        if value:
            setattr(config, attr, value)
    if args.data_stores:
        config.data_stores = list(dict.fromkeys(args.data_stores))
    config.web_grounding = args.web_grounding
    config.timeout_secs = args.timeout
    config.auth = args.auth
    return config


def build_shell(config: Config, console: Console) -> Shell:
    # Two independent SDKs: gemini_chat for the assistant, nblm for NotebookLM notebooks.
    import gemini_chat
    import nblm

    # Native subprocesses do not apply Windows PATHEXT lookup like a shell does.
    gcloud_binary = (shutil.which("gcloud") or "gcloud") if config.auth == "gcloud" else "gcloud"

    def chat() -> object:
        tokens = (
            gemini_chat.EnvTokenProvider()  # GEMINI_CHAT_ACCESS_TOKEN
            if config.auth == "env"
            else gemini_chat.GcloudTokenProvider(binary=gcloud_binary)
        )
        return gemini_chat.GeminiEnterpriseClient(
            tokens,
            str(config.project_number),
            str(config.engine_id),
            config.location,
            collection_id=config.collection_id,
            assistant_id=config.assistant_id,
            timeout_secs=config.timeout_secs,
        )

    def notebooks() -> object:
        tokens = (
            nblm.EnvTokenProvider()  # NBLM_ACCESS_TOKEN
            if config.auth == "env"
            else nblm.GcloudTokenProvider(binary=gcloud_binary)
        )
        return nblm.NblmClient(
            tokens,
            str(config.project_number),
            config.location,
            config.notebook_endpoint_location,
        )

    return Shell(
        config,
        console,
        chat_factory=chat,
        notebook_factory=notebooks,
        normalize_folder=gemini_chat.normalize_sharepoint_folder,
    )


def _prompt_history(enabled: bool) -> History:
    from prompt_toolkit.history import FileHistory, InMemoryHistory

    if not enabled:
        return InMemoryHistory()

    state = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "gesh"
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    history_path = state / "history"
    history_path.touch(mode=0o600, exist_ok=True)
    # Do not rely on the caller's umask for Enterprise prompts. chmod is meaningful
    # on POSIX; other platforms keep their native ACL semantics.
    if os.name == "posix":
        state.chmod(0o700)
        history_path.chmod(0o600)
    return FileHistory(str(history_path))


def _interactive_loop(shell: Shell, *, history_enabled: bool = True) -> None:
    from prompt_toolkit import PromptSession
    from prompt_toolkit.key_binding import KeyBindings

    bindings = KeyBindings()

    @bindings.add("escape", "enter")
    def _(event: object) -> None:  # Esc+Enter (or Alt+Enter) inserts a newline
        event.current_buffer.insert_text("\n")  # type: ignore[attr-defined]

    session: PromptSession[str] = PromptSession(
        history=_prompt_history(history_enabled),
        key_bindings=bindings,
        bottom_toolbar=shell.toolbar,
    )
    shell.console.print(
        f"gesh {__version__} — Gemini Enterprise chat, research and notebooks. /help for commands, "
        "Ctrl-D to quit."
    )
    while True:
        shell.poll_pending()
        try:
            line = session.prompt(shell.prompt_label())
        except KeyboardInterrupt:
            continue
        except EOFError:
            break
        if not shell.handle(line):
            break


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    console = Console(highlight=False)
    config = build_config(args)
    shell = build_shell(config, console)
    if args.session_id:
        shell.session = args.session_id

    if args.command:
        for line in args.command:
            keep_running = shell.handle(line)
            if shell.last_command_failed:
                return 1
            if not keep_running:
                break
        return 0
    if sys.stdin.isatty() and sys.stdout.isatty():
        _interactive_loop(shell, history_enabled=not args.no_history)
    else:  # scripted: one line per command/question
        for line in sys.stdin:
            keep_running = shell.handle(line.rstrip("\n"))
            if shell.last_command_failed:
                return 1
            if not keep_running:
                break
    if shell.pending is not None:
        console.print("An abandoned question is still running; exiting without waiting.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
