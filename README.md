<div align="center">

  <img width="256" alt="logo" src="https://github.com/user-attachments/assets/37f0f882-65ca-436e-8053-3db8c18cac59" />

  # _gemini-enterprise-shell_

  **Terminal-first Gemini Enterprise chat + NotebookLM Enterprise toolkit**

  💬 **gesh**: Interactive Gemini Enterprise shell with chat, citations, SharePoint folder scope, and notebook commands <br/>
  🔌 **gemini-chat**: Official Discovery Engine v1 assistant client for Rust, CLI, and Python <br/>
  📚 **nblm**: NotebookLM Enterprise CLI + Python SDK retained from the nblm-rs foundation

  <sub>NotebookLM foundation package badges:</sub><br/>

  [![Crates.io](https://img.shields.io/crates/v/nblm-cli.svg)](https://crates.io/crates/nblm-cli)
  [![Crates.io](https://img.shields.io/crates/d/nblm-cli.svg?color=orange&label=downloads)](https://crates.io/crates/nblm-cli)
  <br/>
  [![PyPI](https://img.shields.io/pypi/v/nblm.svg?color=blue)](https://pypi.org/project/nblm/)
  [![PyPI Downloads](https://static.pepy.tech/personalized-badge/nblm?period=total&units=INTERNATIONAL_SYSTEM&left_color=GRAY&right_color=BLUE&left_text=downloads)](https://pepy.tech/projects/nblm)
  [![Python versions](https://img.shields.io/pypi/pyversions/nblm.svg)](https://pypi.org/project/nblm/)
  [![uv](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/uv/main/assets/badge/v0.json)](https://github.com/astral-sh/uv)
  [![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
  [![mypy](https://img.shields.io/badge/mypy-checked-blue.svg)](https://mypy-lang.org/)
  <br/>
  <sub>Upstream nblm-rs references:</sub><br/>
  [![DeepWiki](https://img.shields.io/badge/DeepWiki-K--dash%2Fnblm--rs-blue.svg?logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACwAAAAyCAYAAAAnWDnqAAAAAXNSR0IArs4c6QAAA05JREFUaEPtmUtyEzEQhtWTQyQLHNak2AB7ZnyXZMEjXMGeK/AIi+QuHrMnbChYY7MIh8g01fJoopFb0uhhEqqcbWTp06/uv1saEDv4O3n3dV60RfP947Mm9/SQc0ICFQgzfc4CYZoTPAswgSJCCUJUnAAoRHOAUOcATwbmVLWdGoH//PB8mnKqScAhsD0kYP3j/Yt5LPQe2KvcXmGvRHcDnpxfL2zOYJ1mFwrryWTz0advv1Ut4CJgf5uhDuDj5eUcAUoahrdY/56ebRWeraTjMt/00Sh3UDtjgHtQNHwcRGOC98BJEAEymycmYcWwOprTgcB6VZ5JK5TAJ+fXGLBm3FDAmn6oPPjR4rKCAoJCal2eAiQp2x0vxTPB3ALO2CRkwmDy5WohzBDwSEFKRwPbknEggCPB/imwrycgxX2NzoMCHhPkDwqYMr9tRcP5qNrMZHkVnOjRMWwLCcr8ohBVb1OMjxLwGCvjTikrsBOiA6fNyCrm8V1rP93iVPpwaE+gO0SsWmPiXB+jikdf6SizrT5qKasx5j8ABbHpFTx+vFXp9EnYQmLx02h1QTTrl6eDqxLnGjporxl3NL3agEvXdT0WmEost648sQOYAeJS9Q7bfUVoMGnjo4AZdUMQku50McDcMWcBPvr0SzbTAFDfvJqwLzgxwATnCgnp4wDl6Aa+Ax283gghmj+vj7feE2KBBRMW3FzOpLOADl0Isb5587h/U4gGvkt5v60Z1VLG8BhYjbzRwyQZemwAd6cCR5/XFWLYZRIMpX39AR0tjaGGiGzLVyhse5C9RKC6ai42ppWPKiBagOvaYk8lO7DajerabOZP46Lby5wKjw1HCRx7p9sVMOWGzb/vA1hwiWc6jm3MvQDTogQkiqIhJV0nBQBTU+3okKCFDy9WwferkHjtxib7t3xIUQtHxnIwtx4mpg26/HfwVNVDb4oI9RHmx5WGelRVlrtiw43zboCLaxv46AZeB3IlTkwouebTr1y2NjSpHz68WNFjHvupy3q8TFn3Hos2IAk4Ju5dCo8B3wP7VPr/FGaKiG+T+v+TQqIrOqMTL1VdWV1DdmcbO8KXBz6esmYWYKPwDL5b5FA1a0hwapHiom0r/cKaoqr+27/XcrS5UwSMbQAAAABJRU5ErkJggg==)](https://deepwiki.com/K-dash/nblm-rs)
  [![codecov](https://codecov.io/gh/K-dash/nblm-rs/graph/badge.svg?token=OhxeTdnxTw)](https://codecov.io/gh/K-dash/nblm-rs)


  
</div>

> [!IMPORTANT]
> This repository combines two independent Google enterprise surfaces:
> **Gemini Enterprise assistant/search** through the official Discovery Engine v1 APIs,
> and **NotebookLM Enterprise** through the retained `nblm` client stack.
> The NotebookLM code remains an important foundation of the project; the newer
> Gemini Enterprise client and `gesh` shell are built alongside it, not as replacements.
> Consumer NotebookLM and general Google Workspace NotebookLM are not the target APIs.

## What this repository provides

| Surface | What it is | Highlights |
| --- | --- | --- |
| **`gesh`** | Interactive terminal shell | Gemini Enterprise chat, Markdown answers, native references, server sessions, SharePoint folder-scoped Q&A, and NotebookLM commands in one shell |
| **`gemini-chat`** | Independent Rust/CLI/Python client | Official Discovery Engine v1 `streamAssist`, session readback, datastore selection, structured grounding, web grounding, Search/Answer, and fail-closed SharePoint folder scope |
| **`nblm`** | NotebookLM Enterprise foundation | Original Rust CLI and Python SDK for notebooks, sources, audio overviews, sharing, and automation |

The APIs are deliberately kept separate internally. The Gemini Enterprise chat client
does not depend on the NotebookLM client, while `gesh` composes both Python SDKs into
one operator-friendly terminal experience.

## Gemini Enterprise Shell quick start

Authenticate with the Google account that has access to your Gemini Enterprise app:

```bash
gcloud auth login

cd gesh
uv sync

uv run gesh \
  --project-number YOUR_PROJECT_NUMBER \
  --location us \
  --engine-id YOUR_ENGINE_ID \
  --data-store-id YOUR_DATASTORE_ID
```

Then chat normally:

```text
gesh[new]> Find the relevant design document and summarize the decision.
gesh[16059694688619672760]> What evidence supports that?
```

For a stronger SharePoint folder boundary, use folder mode. Retrieval is constrained to
one explicit SharePoint datastore and every returned file/reference is checked locally:

```text
gesh[new]> /folder https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs
gesh[folder:Specs]> /terms sync handshake
gesh[folder:Specs]> /search
gesh[folder:Specs]> What does the spec say the handshake does?
```

NotebookLM remains available in the same shell:

```text
gesh[new]> /nb list 5
gesh[new]> /nb show 1
gesh[new]> /nb create Architecture Notes
```

See [`gesh/README.md`](gesh/README.md) for the interactive shell and
[the Gemini Enterprise guide](docs/guides/gemini-enterprise-chat.md) for the API,
authentication, grounding, session, and folder-isolation details.

## Foundation and lineage

This repository is built on **[K-dash/nblm-rs](https://github.com/K-dash/nblm-rs)**.
The original NotebookLM Enterprise Rust core, CLI, Python SDK, packaging, and much of
the NotebookLM documentation are intentionally retained here. They remain the foundation
for the notebook side of `gesh` and for users who want the standalone `nblm` tooling.

The Gemini Enterprise work adds a separate official assistant client (`gemini-chat`)
and the `gesh` shell around that foundation. Keeping both layers visible in this README
is intentional: this repo is now broader than nblm-rs, but it still inherits and values
that project as its NotebookLM base.

## NotebookLM foundation: motivation

In September 2025, Google released the [NotebookLM Enterprise API](https://cloud.google.com/gemini/enterprise/notebooklm-enterprise/docs/overview), enabling programmatic access to NotebookLM features for the first time.

While you can interact with the API using simple `curl` commands, this approach has several limitations that this project addresses:

### Challenges with Direct API Calls

- **Authentication complexity**

  - **Problem**: Managing OAuth tokens, handling token refresh, and ensuring secure credential storage
  - **Solution**: Seamless `gcloud` CLI integration with automatic token caching and refresh

- **Manual request construction**

  - **Problem**: Writing JSON payloads by hand, managing resource names, and handling API versioning
  - **Solution**: Type-safe CLI flags and Python SDK with intelligent defaults and validation

- **Error handling**

  - **Problem**: Cryptic HTTP error codes without context or recovery suggestions
  - **Solution**: Clear, actionable error messages with automatic retries for transient failures

- **Repeated operations**

  - **Problem**: Writing boilerplate loops for fetch/add/delete sequences
  - **Solution**: Higher-level client helpers and CLI flags that wrap single API calls (with retries built in) so scripts stay concise

- **Output parsing**
  - **Problem**: Manual JSON parsing and extracting specific fields from responses
  - **Solution**: Structured response objects in the Python SDK and `--json` output in the CLI for easy integration with tools like `jq`

### NotebookLM foundation goals

The retained nblm-rs foundation provides production-ready tools that make the NotebookLM API accessible and reliable:

- **Rust CLI**: Fast, cross-platform binary for shell scripting and automation
- **Python SDK**: Idiomatic Python bindings for application integration
- **Type safety**: Compile-time checks prevent common API usage errors
- **Developer experience**: Intuitive commands and clear documentation

## Installation

### CLI

```bash
# macOS
brew tap k-dash/nblm https://github.com/K-dash/homebrew-nblm
brew install k-dash/nblm/nblm

# Linux (prebuilt binaries)
# Download from Releases page: https://github.com/K-dash/nblm-rs/releases

# From source
cargo install nblm-cli
```

### Build this checkout

With Rust, GNU Make, and [cargo-make](https://github.com/sagiegurari/cargo-make)
installed (`cargo install cargo-make`), run from the repository root:

```bash
make                         # Build the release CLI
make install                 # Install/update the nblm command from this checkout
make install-chat            # Install/update the gemini-chat command
make gesh                    # Interactive shell for chat + notebooks (gesh/, needs uv)
make run ARGS="--help"        # Run from source, rebuilding changes automatically
make help                    # List available targets
```

`make install` uses `cargo install --path crates/nblm-cli --force --locked`.
The binary goes into Cargo's bin directory (normally `~/.cargo/bin`); ensure it is
on `PATH`. No `sudo` is needed. The installed binary is not editable: run
`make install` again after changes, or use `make run` during development.
`make` only builds; it does not update the installed command. All builds include
local changes, whether committed or not.

### Python SDK

```bash
pip install nblm
# or
uv add nblm
```

> Prerequisite: a Google Cloud project with the NotebookLM Enterprise API enabled and either `gcloud auth login` or an OAuth token ready for `NBLM_ACCESS_TOKEN`.

For detailed installation instructions and troubleshooting, see the [Installation Guide](https://k-dash.github.io/nblm-rs/getting-started/installation/).

## API quick start

### CLI

```bash
# 1. Authenticate
gcloud auth login

# 2. Set environment variables
export NBLM_PROJECT_NUMBER="123456789012"  # Get from GCP console
export NBLM_LOCATION="global"
export NBLM_ENDPOINT_LOCATION="global"

# 3. Create a notebook
nblm notebooks create --title "My Notebook"

# 4. Add a source
nblm sources add \
  --notebook-id YOUR_NOTEBOOK_ID \
  --web-url "https://example.com" \
  --web-name "Example"
```

### Gemini Enterprise chat (official API)

Uses the official Discovery Engine v1 assistant API with the same OAuth token as
the rest of the CLI:

A separate `gemini-chat` command and crate, independent of nblm
(`make install-chat` installs it):

```sh
gemini-chat --project-number YOUR_PROJECT_NUMBER --location us --json \
  --engine-id YOUR_ENGINE_ID ask \
  --model gemini-4-argon --data-store-id YOUR_SHAREPOINT_DATASTORE_ID \
  'Find the relevant document and cite its supporting evidence.'
```

Supports explicit models, server-session continuation/readback, and native
structured grounding. No generation retries or model fallback. Answers without
native references are flagged; citations and quotes still require review.
See the [Gemini Enterprise guide](docs/guides/gemini-enterprise-chat.md) for CLI,
Rust/Python examples, authentication, and tool boundaries.

### Python

```python
from nblm import NblmClient, GcloudTokenProvider, WebSource

# Initialize client
client = NblmClient(
    token_provider=GcloudTokenProvider(),
    project_number="123456789012"
)

# Create a notebook
notebook = client.create_notebook(title="My Notebook")

# Add sources
response = client.add_sources(
    notebook_id=notebook.notebook_id,
    web_sources=[WebSource(url="https://example.com", name="Example")]
)
```

## Features

### Gemini Enterprise

- **Interactive shell**: `gesh` provides Markdown terminal chat with session history,
  references, scripting, configuration commands, and NotebookLM operations.
- **Official assistant API**: `gemini-chat` uses Discovery Engine v1 `streamAssist`
  with explicit model/datastore selection and server-session continuation/readback.
- **Structured grounding**: Native references and grounding metadata are preserved rather
  than inferred from answer text.
- **SharePoint folder-scoped answers**: Search + Answer with exactly one datastore,
  exact-result validation, local folder checks, and fail-closed session provenance.
- **Rust and Python**: The Gemini Enterprise client is available as Rust crates, a CLI,
  and the independent `gemini_chat` Python package.
- **No hidden retries**: Generation is sent once; ambiguous failures are surfaced so the
  caller can inspect the session before deciding what to do.

### NotebookLM Enterprise foundation

> [!NOTE]
> The NotebookLM API is currently in **alpha**. Some features may not work as documented due to API limitations. See the [complete upstream feature list](https://k-dash.github.io/nblm-rs/#features).

The retained nblm-rs stack supports the following NotebookLM API operations:

- **Notebooks**: Create, list, and delete notebooks
- **Sources**: Add web URLs, text, videos (YouTube), Google Drive files, and upload files
- **Audio Overview**: Create and delete audio overviews
- **Sharing**: Share notebooks with users (CLI only, untested)

For detailed NotebookLM feature status and limitations, see the
[upstream nblm-rs feature documentation](https://k-dash.github.io/nblm-rs/#features).

## Documentation

### Gemini Enterprise Shell

- [gesh interactive shell](gesh/README.md) - Commands, configuration, folder mode, scripting, and behavior
- [Gemini Enterprise guide](docs/guides/gemini-enterprise-chat.md) - Official API usage, authentication, sessions, grounding, and SharePoint folder isolation
- [gemini-chat Python package](python/gemini-chat/README.md) - Python examples for chat, Search/Answer, and trusted folder-session continuation
- [Project documentation site](https://fenghaitao.github.io/gemini-enterprise-shell/) - Generated project documentation

### NotebookLM Enterprise foundation

The original nblm-rs documentation remains the most complete reference for the retained
NotebookLM CLI and Python SDK:

- [nblm-rs documentation](https://k-dash.github.io/nblm-rs/) - Guides, API references, and examples
- [Getting Started](https://k-dash.github.io/nblm-rs/getting-started/installation/) - Installation, authentication, configuration
- [CLI Reference](https://k-dash.github.io/nblm-rs/cli/) - All commands, options, and examples
- [Python SDK Reference](https://k-dash.github.io/nblm-rs/python/) - API reference and usage patterns

## Known API Issues

> [!NOTE]
> The NotebookLM API is currently in **alpha** and has several known limitations. See [API Limitations](https://k-dash.github.io/nblm-rs/api/limitations/) for details.

## Related Resources

- [K-dash/nblm-rs](https://github.com/K-dash/nblm-rs) - Upstream foundation for the NotebookLM Enterprise CLI and Python SDK
- [Gemini Enterprise StreamAssist reference](https://docs.cloud.google.com/gemini/enterprise/docs/reference/rest/v1/projects.locations.collections.engines.assistants/streamAssist) - Official assistant API
- [Gemini Enterprise invocation guide](https://docs.cloud.google.com/gemini/enterprise/docs/invoke-agent-streamassist) - Official behavior and limitations
- [NotebookLM API Documentation](https://cloud.google.com/gemini/enterprise/notebooklm-enterprise/docs/overview) - Official NotebookLM Enterprise documentation
- [NotebookLM API Reference](https://cloud.google.com/gemini/enterprise/notebooklm-enterprise/docs/api-notebooks) - NotebookLM Enterprise API reference

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

MIT
