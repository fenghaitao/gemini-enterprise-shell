# gesh — Gemini Enterprise shell

A line-based terminal shell for Gemini Enterprise features available through
official Google APIs:

- **Chat** with the Gemini Enterprise assistant (Discovery Engine v1 `streamAssist`):
  explicit model, datastore-scoped retrieval (e.g. SharePoint), server sessions,
  native citations.
- **Deep Research** through the built-in official agent: plan, explicitly start,
  inspect and save results in a separate research session.
- **Notebooks** in NotebookLM Enterprise: list, inspect sources, create, add web/file
  sources, delete.

It is a thin UI over this repository's two independent Python SDKs: `gemini_chat`
(`GeminiEnterpriseClient`, from `python/gemini-chat`) for chat and `nblm` (`NblmClient`,
from `python/`) for notebooks. Auth is OAuth via `gcloud` (or access tokens in
environment variables); no browser cookies.

## Install

Requirements: Python 3.14, [uv](https://docs.astral.sh/uv/), Rust (to build the SDKs),
and `gcloud` logged in (`gcloud auth login`).

```bash
cd gesh                  # in this repository
uv sync                  # builds both SDKs: ../python and ../python/gemini-chat
uv run gesh --help       # or, from anywhere: uv run --project /path/to/repo/gesh gesh
```

Optional, to get a `gesh` command on `PATH`:

```bash
uv tool install --editable .     # or: uv run gesh ...
```

After changing Rust code, rebuild the SDKs with
`uv sync --reinstall-package nblm --reinstall-package gemini-chat`.

## Configure

Each setting comes from a flag, else `GESH_<NAME>`, else `NBLM_<NAME>`, so an existing
nblm environment works as is.

| Flag | Env name (`GESH_`/`NBLM_` prefix) | Meaning |
|---|---|---|
| `--project-number` | `PROJECT_NUMBER` | Google Cloud project **number** |
| `--location` | `LOCATION` | `us`, `eu` or `global` (default `us`) |
| `--endpoint-location` | `ENDPOINT_LOCATION` | NotebookLM endpoint location (default: `--location`) |
| `--engine-id` | `ENGINE_ID` | Gemini Enterprise app/engine ID (not the portal CID UUID) |
| `--model` | `ENTERPRISE_MODEL` | e.g. `gemini-4-argon`; default = assistant default |
| `--data-store-id` (repeatable) | `DATA_STORE_ID` or comma-separated `DATA_STORE_IDS` | Retrieval datastores |
| `--web-grounding` | | Public web grounding (needs assistant support) |
| `--session-id` | | Continue an existing session |
| `--auth gcloud\|env` | `env`: `GEMINI_CHAT_ACCESS_TOKEN` (chat), `NBLM_ACCESS_TOKEN` (notebooks) | Token source (default `gcloud`) |
| `--no-history` | | Do not persist interactive prompt history |

Behind an HTTPS-inspecting proxy, the SDKs trust the CA bundle in `SSL_CERT_FILE` or
`REQUESTS_CA_BUNDLE` (or `NBLM_CA_BUNDLE` / `GEMINI_CHAT_CA_BUNDLE` per SDK).

## Use

```text
$ uv run gesh
gesh[new]> Find H-0001.txt and explain the CBB role in COR-AP HWSync.
⠋ Asking Gemini Enterprise… 12s (Ctrl-C to stop waiting)
<Markdown answer>

Sources (1)
 [1] H-0001.txt
     https://…/H-0001.txt
session 16059694688619672760 · model gemini-4-argon
gesh[16059694688619672760]> In one sentence: which die is the CBB parent?
…
```

The bottom bar shows engine, model, datastores and session. Enter sends; Esc+Enter (or
Alt+Enter) inserts a newline. By default, history is kept in
`~/.local/state/gesh/history`; on POSIX the directory/file are forced to `0700`/`0600`.
Use `--no-history` when prompts should not be persisted.

### Chat commands

| Command | |
|---|---|
| *question* | Ask; continues the current session |
| `/new` | Next question starts a new session |
| `/session [ID]` | Show current session, or switch to (and read back) an existing one |
| `/history` | Read back the session's questions and answers |
| `/model [ID\|default]` | Show/set the model |
| `/ds`, `/ds add ID…`, `/ds rm ID…`, `/ds clear` | Retrieval datastores |
| `/web [on\|off]` | Web grounding |
| `/refs` | Last answer's sources with snippets |
| `/save FILE` | Save this shell's transcript as Markdown |

### Folder-scoped answers (SharePoint)

Normal chat cannot be restricted to a folder. Folder mode answers only from files in one
SharePoint folder, enforced by SharePoint: a folder-scoped search with your keywords,
then an answer from just those files. Folder mode has its own server session, separate
from the chat session: follow-up questions continue it (each turn searches the folder
again, with the current `/terms`), `/new` starts a fresh one, `/history` reads it back,
and changing the folder starts a new one.

```text
gesh[new]> /folder https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs
gesh[folder:Specs]> /terms sync handshake        # keywords: every word must match
gesh[folder:Specs]> /search                       # list matching files (no generation)
gesh[folder:Specs]> What does the spec say the handshake does?
gesh[folder:Specs]> /terms handshake root die     # keywords for the follow-up
gesh[folder:Specs]> Shorten that, then say which die drives the ack.
gesh[folder:Specs]> /folder off                   # back to normal chat (its session is kept)
```

| Command | |
|---|---|
| `/folder URL`, `/folder off`, `/folder` | Set, clear or show the folder |
| `/terms KEYWORDS`, `/terms off` | Keywords that find files in the folder |
| `/search [TERMS]` | Matching files (in the folder if set), full text or snippet |

In folder mode, `/session` shows the current folder session. `/session ID` is
intentionally refused: folder-session provenance is trusted only for sessions created
by the current gesh process, so an arbitrary server session cannot import history from
another folder.

Also `--sharepoint-folder` / `--search-terms` (or `GESH_SHAREPOINT_FOLDER` /
`GESH_SEARCH_TERMS`). At most 100 files per search; only about 5 arrive with full text,
which the answer's warnings report. Folder mode needs **exactly one** datastore, the
SharePoint file datastore (`/ds clear`, then `/ds add ID`); see the guide's "End to
end" steps for finding its ID.

### Deep Research

```text
/research plan Compare strategies for testing a CLI API client; plan only.
/research start
/research show
/research history
/research read SAVED_SESSION_ID
/research save research.json
```

Review before `start`; it is never sent automatically. Research has a separate
30-minute SDK timeout, does not use `/model`, and pins the plan's datastore/web
settings. One start attempt per plan is allowed, including after uncertainty.
Ctrl-C stops waiting, not server work; wait for the late result, use read-only
`/research read ID`, or inspect the UI. A successful read clears any pending
plan/start authorization and its pinned sources, even for the same session.
Late plans from before that read remain visible/savable but cannot restore start
authorization. Create and review a new plan before starting again. If a late plan
first appears while processing `/research start`, review it and issue a fresh
`/research start` command; that same input cannot both reveal and start the plan.
Research sessions/results do not replace chat or folder sessions. Active folder
mode is rejected for generation because research has **no folder isolation**.

Public v1 may omit some or all research-stage labels. Without a labelled report,
the last nonempty unlabelled text block (or last labelled text if none) is displayed
as an **unverified candidate**, not as a guaranteed report; all content and native
grounding can be saved as JSON. `/research save` creates a private new file and
refuses overwrites. Audio is file metadata only. See the
[Deep Research guide](../docs/guides/deep-research.md) for details.

### Notebook commands

| Command | |
|---|---|
| `/nb list [N]` | Recently viewed notebooks, numbered |
| `/nb show NB` | Sources of a notebook (`NB` = number from the list, or notebook ID) |
| `/nb create TITLE` | Create a notebook |
| `/nb add-url NB URL [NAME]` | Add a web source |
| `/nb add-file NB PATH` | Upload a local file |
| `/nb delete NB` | Delete (asks for confirmation) |

Also `/config`, `/help`, `/quit` (or Ctrl-D).

### Scripting

Non-interactive input is read line by line, and `-c` runs given lines. The first failed
command stops the script and returns a nonzero exit status:

```bash
printf '/nb list 5\n/quit\n' | uv run gesh
uv run gesh -c '/config' -c 'What is H-0001.txt about?'
```

## Behavior worth knowing

- **No automatic retries.** A question is sent once. If it fails, gesh shows the error
  and the session (if known) and does not resend; check `/history` before asking again.
- **Ctrl-C while waiting** stops waiting but cannot cancel the request; the server may
  still record the turn. A late answer is shown when it arrives, and new questions are
  refused until then so turns do not interleave.
- Model, datastores and web grounding are **sent on every turn**; a session keeps the
  conversation, not the tool selection.
- Answers without native references are flagged with a warning. Citations written in
  the answer text are not structured citations, and folder names in a question only
  steer retrieval; they do not restrict it.

## Develop

```bash
uv run pytest
uv run ruff check src tests && uv run ruff format --check src tests
uv run mypy
```

Renaming: the distribution/command name is set in `pyproject.toml` (`name`,
`[project.scripts]`) and the package directory `src/gesh`.
