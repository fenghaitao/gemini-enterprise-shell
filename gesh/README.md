# gesh — Gemini Enterprise shell

A line-based terminal shell for the two Gemini Enterprise features available through
official Google APIs:

- **Chat** with the Gemini Enterprise assistant (Discovery Engine v1 `streamAssist`):
  explicit model, datastore-scoped retrieval (e.g. SharePoint), server sessions,
  native citations.
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
Alt+Enter) inserts a newline. History is kept in `~/.local/state/gesh/history`.

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

Non-interactive input is read line by line, and `-c` runs given lines:

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
