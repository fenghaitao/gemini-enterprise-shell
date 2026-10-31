"""Rich rendering for answers, citations, sessions and notebooks."""

from __future__ import annotations

import re
from typing import Any

from rich.console import Console
from rich.markdown import Markdown
from rich.table import Table
from rich.text import Text

_HIGHLIGHT = re.compile(r"</?c\d+>|<ddd/>")
MODEL_KEY = "google.discoveryengine.googleapis.com.Assistant.model_id"
TOOLS_KEY = "google.discoveryengine.googleapis.com.Assistant.tools_spec"


def clean_snippet(text: str) -> str:
    """Drop the search highlight markup (<c0>…</c0>, <ddd/>) from reference snippets."""
    return " ".join(_HIGHLIGHT.sub("", text).split())


def short_id(resource: str | None) -> str:
    return resource.rsplit("/", 1)[-1] if resource else "-"


def citations(grounding: list[dict[str, Any]]) -> list[dict[str, str]]:
    """Unique native references (by URI, else document) in first-seen order."""
    seen: set[str] = set()
    out: list[dict[str, str]] = []
    for block in grounding:
        for ref in block.get("references", []) or []:
            meta = ref.get("documentMetadata") or {}
            key = meta.get("uri") or meta.get("document") or ""
            if not key or key in seen:
                continue
            seen.add(key)
            out.append(
                {
                    "title": meta.get("title") or short_id(meta.get("document")) or "(untitled)",
                    "uri": meta.get("uri") or "",
                    "snippet": clean_snippet(ref.get("content") or ""),
                }
            )
    return out


def render_answer(console: Console, response: Any, *, snippets: bool = False) -> None:
    console.print(Markdown(response.answer or "_(empty answer)_"))
    refs = citations(response.grounding)
    if refs:
        console.print()
        console.print(Text(f"Sources ({len(refs)})", style="bold"))
        for i, ref in enumerate(refs, 1):
            line = Text(f" [{i}] ", style="cyan")
            line.append(ref["title"], style="bold")
            if ref["uri"]:
                line.append("\n     ")
                line.append(ref["uri"], style=f"link {ref['uri']} dim")
            console.print(line)
            if snippets and ref["snippet"]:
                console.print(Text("     " + ref["snippet"][:400], style="italic dim"))
    for warning in response.warnings:
        console.print(Text(f"! {warning}", style="yellow"))
    meta = f"session {short_id(response.session)}"
    if response.requested_model:
        meta += f" · model {response.requested_model}"
    if response.invocation_tools:
        meta += f" · tools {', '.join(response.invocation_tools)}"
    console.print(Text(meta, style="dim"))


def render_session(console: Console, session: dict[str, Any]) -> None:
    turns = session.get("turns") or []
    title = session.get("displayName") or short_id(session.get("name"))
    console.print(
        Text(f"Session {short_id(session.get('name'))}: {title} ({len(turns)} turns)", style="bold")
    )
    for i, turn in enumerate(turns, 1):
        query = (turn.get("query") or {}).get("text") or ""
        config = turn.get("queryConfig") or {}
        model = config.get(MODEL_KEY) or "default"
        console.print(Text(f"\n[{i}] you: ", style="bold cyan") + Text(query))
        console.print(Text(f"    model {model}", style="dim"))
        answer = turn.get("detailedAssistAnswer") or {}
        text = _answer_text(answer)
        if text:
            console.print(Markdown(text))


def _answer_text(answer: dict[str, Any]) -> str:
    parts: list[str] = []
    for reply in answer.get("replies") or []:
        content = ((reply.get("groundedContent") or {}).get("content")) or {}
        if content.get("thought"):
            continue
        if content.get("text"):
            parts.append(content["text"])
    return "".join(parts)


def notebooks_table(notebooks: list[Any]) -> Table:
    table = Table(show_lines=False, header_style="bold")
    table.add_column("#", justify="right", style="cyan")
    table.add_column("Title")
    table.add_column("Notebook ID", style="dim", no_wrap=True, min_width=36)
    table.add_column("Sources", justify="right")
    table.add_column("Last viewed", style="dim")
    for i, nb in enumerate(notebooks, 1):
        meta = nb.metadata
        table.add_row(
            str(i),
            f"{nb.emoji or ''} {nb.title}".strip(),
            nb.notebook_id or short_id(nb.name),
            str(len(nb.sources)),
            (meta.last_viewed or "")[:19].replace("T", " ") if meta else "",
        )
    return table


def sources_table(notebook: Any) -> Table:
    table = Table(header_style="bold", title=f"{notebook.title} ({len(notebook.sources)} sources)")
    table.add_column("#", justify="right", style="cyan")
    table.add_column("Title")
    table.add_column("Status")
    table.add_column("Words", justify="right")
    table.add_column("Source ID", style="dim", no_wrap=True, min_width=36)
    for i, src in enumerate(notebook.sources, 1):
        status = src.settings.status if src.settings else None
        words = src.metadata.word_count if src.metadata else None
        table.add_row(
            str(i),
            src.title or "",
            (status or "").removeprefix("SOURCE_STATUS_"),
            "" if words is None else str(words),
            short_id(src.name),
        )
    return table


def folder_name(url: str) -> str:
    """Last path segment of a folder URL, decoded, for prompts and the toolbar."""
    from urllib.parse import unquote, urlsplit

    path = unquote(urlsplit(url.strip()).path).rstrip("/")
    return path.rsplit("/", 1)[-1] or url


def folder_sources(answer: dict[str, Any]) -> list[dict[str, str]]:
    """Unique referenced files of a folder-scoped answer, in citation order."""
    seen: set[str] = set()
    out: list[dict[str, str]] = []
    for ref in answer.get("references") or []:
        key = ref.get("uri") or ref.get("document") or ""
        if not key or key in seen:
            continue
        seen.add(key)
        out.append(
            {
                "title": ref.get("title") or "(untitled)",
                "uri": ref.get("uri") or "",
                "snippet": clean_snippet(ref.get("content") or ""),
            }
        )
    return out


def render_folder_answer(console: Console, answer: dict[str, Any]) -> None:
    console.print(Markdown(answer.get("answer") or "_(empty answer)_"))
    refs = folder_sources(answer)
    if refs:
        console.print()
        console.print(Text(f"Sources ({len(refs)})", style="bold"))
        for i, ref in enumerate(refs, 1):
            line = Text(f" [{i}] ", style="cyan")
            line.append(ref["title"], style="bold")
            if ref["uri"]:
                line.append("\n     ")
                line.append(ref["uri"], style=f"link {ref['uri']} dim")
            console.print(line)
    for warning in answer.get("warnings") or []:
        console.print(Text(f"! {warning}", style="yellow"))
    sources = answer.get("sources") or []
    full = sum(1 for s in sources if s.get("full_text"))
    console.print(
        Text(
            f"folder-scoped · {len(sources)} files ({full} full text) · no session",
            style="dim",
        )
    )


def render_folder_references(console: Console, answer: dict[str, Any]) -> None:
    refs = folder_sources(answer)
    if not refs:
        console.print("The last answer has no references.")
        return
    for i, ref in enumerate(refs, 1):
        console.print(Text(f"[{i}] ", style="cyan") + Text(ref["title"], style="bold"))
        if ref["uri"]:
            console.print(Text("    " + ref["uri"], style="dim"))
        if ref["snippet"]:
            console.print(Text("    " + ref["snippet"][:400], style="italic"))


def render_search(console: Console, results: dict[str, Any]) -> None:
    hits = results.get("hits") or []
    table = Table(header_style="bold", title=f"{len(hits)} file(s)")
    table.add_column("#", justify="right", style="cyan")
    table.add_column("Title")
    table.add_column("Text")
    table.add_column("URL", style="dim", overflow="fold")
    for i, hit in enumerate(hits, 1):
        table.add_row(
            str(i),
            hit.get("title") or "",
            "full" if hit.get("content") else "snippet",
            hit.get("uri") or "",
        )
    console.print(table)
    for warning in results.get("warnings") or []:
        console.print(Text(f"! {warning}", style="yellow"))
