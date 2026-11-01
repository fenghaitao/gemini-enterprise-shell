# Gemini Enterprise chat — official public API

`gemini-chat` talks to Gemini Enterprise's **official Discovery Engine v1
assistant API**. It is independent of NotebookLM: the `gemini-chat` library crate
and the `gemini-chat` command (crate `gemini-chat-cli`) do not depend on
`nblm-core` or `nblm`. The Python bindings are a separate `gemini_chat` package (`python/gemini-chat`,
built from `crates/gemini-chat-python`); it does not need `nblm`.

Install the command from this checkout with `make install-chat`
(`cargo install --path crates/gemini-chat-cli`).

## Prerequisites and resource discovery

- An existing Gemini Enterprise app/engine and a licensed/authorized account.
- OAuth credentials with a supported scope, such as `cloud-platform` (`gcloud
  auth login`, or an access token in `GEMINI_CHAT_ACCESS_TOKEN` with `--auth env`).
- Invocation permission `discoveryengine.assistants.assist`. Reading saved
  sessions also needs the applicable session-read permission. Broad admin access
  is **not** required merely to invoke an already known assistant.
- The real project number, location, engine ID, and relevant datastore IDs.
  The web portal's CID UUID is **not necessarily the engine ID**. Use known
  deployment configuration, your administrator, or authorized resource discovery;
  never guess based on a portal UUID.

The client sends `x-goog-user-project` with the configured project number, uses
TLS verification, and never needs browser cookies. It does not enable APIs,
change IAM, authorize connectors, or change billing configuration.

Behind an HTTPS-inspecting proxy, add the proxy's CA: the client reads a PEM bundle
from `GEMINI_CHAT_CA_BUNDLE`, then `SSL_CERT_FILE`, then `REQUESTS_CA_BUNDLE` (first
nonempty wins; a bad selected bundle fails closed). Built-in roots stay trusted.
Through the Python SDK, the same variables apply.

`--location` must be `global`, `us` or `eu`, and must be the **engine's own
location**: it is part of every resource path, so an engine created in `us` is not
found under `global` (HTTP 404). The endpoint follows the location: `global` uses
`https://discoveryengine.googleapis.com/v1/`; `us` and `eu` use
`https://us-discoveryengine.googleapis.com/v1/` and
`https://eu-discoveryengine.googleapis.com/v1/`. A hidden `--base-url`
(`GEMINI_CHAT_BASE_URL`) is a test/proxy override and must include `/v1` (local
HTTP only for tests).

## CLI: ask with an explicit model and datastore

```sh
export GEMINI_CHAT_PROJECT_NUMBER=YOUR_PROJECT_NUMBER
export GEMINI_CHAT_LOCATION=us
export GEMINI_CHAT_ENGINE_ID=YOUR_ENGINE_ID

gcloud auth login

gemini-chat --json ask \
  --model gemini-4-argon \
  --data-store-id YOUR_SHAREPOINT_FILE_DATASTORE_ID \
  'Find the relevant HWRS document and answer with native source citations.'
```

Alternatively pass `--project-number`, `--location` and `--engine-id`.
`--data-store-id` is repeatable and accepts short IDs or full datastore resources
in the configured project/location/collection. With none supplied, engine defaults
apply; this is **not** SharePoint-only or notebook-only grounding.

Optional configuration:

| Flag/environment | Default/meaning |
| --- | --- |
| `--project-number` / `GEMINI_CHAT_PROJECT_NUMBER` | Required project **number** |
| `--location` / `GEMINI_CHAT_LOCATION` | `global` (default), `us` or `eu`; must match the engine |
| `--engine-id` / `GEMINI_CHAT_ENGINE_ID` | Required real engine ID |
| `--assistant-id` / `GEMINI_CHAT_ASSISTANT_ID` | `default_assistant` |
| `--collection-id` / `GEMINI_CHAT_COLLECTION_ID` | `default_collection` |
| `ask --model` / `GEMINI_CHAT_MODEL` | Omit for assistant default |
| `ask --session-id` | Continue a saved session; otherwise start a new one |
| `ask --data-store-id` | Repeatable datastore selection |
| `ask --web-grounding` | Explicit public-web grounding opt-in; needs assistant support |
| `--timeout 6m` | Default 360 seconds |
| `--auth env` / `GEMINI_CHAT_AUTH` | Use `GEMINI_CHAT_ACCESS_TOKEN` instead of gcloud |
| `--json` | JSON output |

An explicit model is sent as `generationSpec.modelId`; no fallback model is
substituted on error. IDs are tenant/service-dependent. `gemini-4-argon` was
observed in a real tenant's model configuration and accepted live on 2026-10-30;
availability in other tenants is not guaranteed. Human UI names are not sufficient
to infer an API model ID.

## Continue and inspect a server conversation

Copy the returned **full `session` resource**, or its last ID segment:

```sh
gemini-chat --json ask \
  --model gemini-4-argon \
  --session-id YOUR_SESSION_ID \
  --data-store-id YOUR_SHAREPOINT_FILE_DATASTORE_ID \
  'Explain the second point in your previous answer.'

# Read-only; does not generate another answer.
gemini-chat --json session --session-id YOUR_SESSION_ID
gemini-chat --json session --session-id YOUR_SESSION_ID --include-answer-details
```

Re-send the intended model/datastore options on each turn. The server session
provides prior conversational context; it is not a guarantee of current tool
selection or folder isolation. Stored turns' `queryConfig` may report
`google.discoveryengine.googleapis.com.Assistant.model_id`. This is a
service-reported persisted selection, unlike the response's `requested_model`.

Cross-UI visibility and continuity with Google's browser chat are **not**
verified; returning a server session is not proof of that.

## Answers limited to one SharePoint folder

`ask` (streamAssist) cannot be restricted to a folder: a folder named in the question
only steers retrieval, and the assistant may search elsewhere or answer from memory.
For a server-enforced scope, use the two-step commands instead.

### End to end

**1. Log in and configure** (once per shell; the login expires on the organization's
reauthentication schedule, and the tools then report "run `gcloud auth login`"):

```sh
gcloud auth login                       # in the browser profile of your work account
make install-chat                       # or: cargo install --path crates/gemini-chat-cli
export GEMINI_CHAT_PROJECT_NUMBER=YOUR_PROJECT_NUMBER
export GEMINI_CHAT_LOCATION=us           # the engine's own location: global, us or eu
export GEMINI_CHAT_ENGINE_ID=YOUR_ENGINE_ID
# Behind a TLS-inspecting proxy: export GEMINI_CHAT_CA_BUNDLE=/path/to/ca-bundle.pem
```

**2. Find the SharePoint file datastore ID.** Reading the engine works even without
permission to list engines or datastores; it returns the attached datastore IDs. The
SharePoint connector has one per entity type; use the one ending in `_file`:

```sh
curl -sS -H "Authorization: Bearer $(gcloud auth print-access-token)" \
  -H "x-goog-user-project: $GEMINI_CHAT_PROJECT_NUMBER" \
  "https://$GEMINI_CHAT_LOCATION-discoveryengine.googleapis.com/v1/projects/$GEMINI_CHAT_PROJECT_NUMBER/locations/$GEMINI_CHAT_LOCATION/collections/default_collection/engines/$GEMINI_CHAT_ENGINE_ID" \
  | python3 -c 'import json,sys; print("\n".join(json.load(sys.stdin)["dataStoreIds"]))'
# e.g. sharepoint_connector_ab12cd_file   <- use this one
export DS=sharepoint_connector_ab12cd_file
```

(For `global`, the host is `discoveryengine.googleapis.com` without a prefix.)

**3. Copy the folder URL** from the browser address bar while viewing the folder in
SharePoint, for example
`https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs`. Library views
(`…/Forms/AllItems.aspx?id=…`) also work; sharing links (`…/:f:/…`) do not.

**4. Search and answer from the CLI:**

```sh
F='https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs'

# a) Which files match? (no generation; every word must match, so use keywords)
gemini-chat search --sharepoint-folder "$F" --data-store-id "$DS" sync handshake

# b) Answer only from the matching files in that folder (one generation).
#    Prints the answer, `Session: …`, the files used and the references.
gemini-chat answer --sharepoint-folder "$F" --data-store-id "$DS" \
  --search-terms 'sync handshake' \
  'In two sentences, what does the spec say the handshake does?'

# c) Read the server session back (no generation).
gemini-chat session --session-id SESSION_ID
```

Add `--json` to any command for machine-readable output. Folder-answer continuation
is intentionally **not** exposed by the one-shot CLI: a fresh process cannot prove how
an arbitrary server session was originally scoped. For strict multi-turn folder chat,
use one long-lived Python `GeminiEnterpriseClient` instance
(`.answer_in_folder(..., session_id=...)`) or gesh (`/folder`, `/terms`, then plain
questions). Both keep trusted session-to-folder provenance in the client process.

What to check in the output: `References` (every one is inside the folder and was
supplied in that turn, or the command fails) and the warnings: "more files may match"
(narrow the keywords or raise `--page-size`) and "N of M files were sent as short
snippets only" (only about 5 files arrive with full text).

How it works:

1. **Search** sends `(<search terms>) path:"<folder>"`. With a federated SharePoint
   datastore, SharePoint evaluates `path:` itself: recursive, with your own SharePoint
   permissions. Spell correction is suggestion-only, because automatic correction
   rewrites the folder URL.
2. **Answer** receives only those files (`searchSpec.searchResultList`), so no other
   server-side search runs; its citations can only point at supplied files. Folder-scoped
   Search/Answer require exactly **one** datastore, preventing the query from being fanned
   out to unrelated connectors before local URL checks run.
3. Every search result and every answer reference is re-checked locally against the
   folder (host, then path, case-insensitive, ignoring `?web=1`). Anything outside
   fails the call instead of being dropped silently.
4. Each answer reference must also be one of the files sent **in this turn** (same
   document resource, or the same normalized URL when the reference has no resource).
   A citation of any other file, even one inside the folder, fails the call.

The contract per turn: retrieval and references are limited to this turn's search
results. For multi-turn use, the client records the session's normalized folder and
datastore when it creates the session. A continuation is accepted only by that same
client instance and only for the same folder and datastore. This request provenance,
not the presence of citations, is the trust boundary, so a successful scoped answer
with zero structured references can still be continued safely.

Folder URLs: paste the browser address of the folder, with or without a trailing slash,
with literal or `%20` spaces, or the library view (`…/Forms/AllItems.aspx?id=/sites/…`).
Sharing links (`…/:f:/…`) cannot be resolved locally and are rejected. Search terms must
not contain `path:` and must have balanced quotes and parentheses.

Limits:

- Keywords, not a sentence: with a folder restriction SharePoint requires every word to
  match, which is why `answer` takes `--search-terms` separately from the question.
- At most 100 files per search (`--page-size`, default 10), with no further pages; a
  warning says when the limit was reached.
- Search returns full text for only about the top 5 files; the rest are sent as short
  snippets, and a warning reports how many.
- Sessions: the first folder `answer` creates a server session. The service does
  **not** persist an attested folder scope on that session, so saved-session readback is
  not used as proof. The client instead binds sessions it creates to the normalized
  folder and datastore in memory; clones share that registry. The registry keeps the
  most recent 1024 folder sessions; older entries are evicted and then fail closed like
  sessions from a fresh client. A continuation from a fresh client/process, a different
  folder, or a different datastore is rejected before network I/O. Each accepted turn
  then runs its own folder-scoped search and sends only
  those files. Give each follow-up its own search terms (shorthand questions like
  "which die?" make poor keywords). Server readback remains available with
  `gemini-chat session`, but is informational rather than an isolation proof.
- `answer` has no `--model` option. A question the Answer API declines (for example
  small talk) is reported as skipped; if no file matches, the Answer API is not called
  at all.
- This relies on the SharePoint connector being in **federated** (live search) mode. It
  needs the `discoveryengine.servingConfigs.search` and `.answer` permissions.
- The strongest alternative is admin-side: a dedicated SharePoint connector whose Path
  filter is the folder. Then plain `--data-store-id` is scoped on every call, including `ask`.

In Rust: `GeminiEnterpriseClient::search(SearchRequest)` and
`GeminiEnterpriseClient::answer_in_folder(FolderAnswerRequest)` with a
`SharePointFolder::parse(url)`.

## Rust

Crate `gemini-chat` (path `crates/gemini-chat`), no NotebookLM dependency:

```rust
use std::sync::Arc;
use gemini_chat::{AssistRequest, GcloudTokenProvider,
    GeminiEnterpriseClient, GeminiEnterpriseConfig};

let client = GeminiEnterpriseClient::new(
    Arc::new(GcloudTokenProvider::default()),
    GeminiEnterpriseConfig::new("123456789012", "us", "your-engine-id"),
)?;
let mut request = AssistRequest::new("Find the relevant source and cite it.");
request.model_id = Some("gemini-4-argon".into());
request.data_stores = vec!["your-sharepoint-file-datastore".into()];
let result = client.ask(request).await?;
let session = client.get_session(&result.session, false).await?;
```

## Python

Build it from `python/gemini-chat` with `uv run --extra dev maturin develop`:

```python
from gemini_chat import GeminiEnterpriseClient, GcloudTokenProvider

client = GeminiEnterpriseClient(
    GcloudTokenProvider(),
    project_number="123456789012",
    engine_id="your-engine-id",
    location="us",
)
answer = client.ask(
    "Find the relevant source and cite it.",
    model_id="gemini-4-argon",
    data_store_ids=["your-sharepoint-file-datastore"],
)
print(answer.answer)
print(answer.session)
print(answer.grounding)
print(answer.warnings)

saved = client.get_session(answer.session)
for turn in saved["turns"]:
    print(turn["queryConfig"].get(
        "google.discoveryengine.googleapis.com.Assistant.model_id"
    ))
# A deliberate new turn, not an automatic retry:
# client.ask("Follow-up", session_id=answer.session,
#            model_id="gemini-4-argon", data_store_ids=[...])
```

`GeminiAssistResponse` exposes `answer`, `session`, `answer_resource`,
`requested_model`, `grounding`, `native_reference_count`, `invocation_tools`,
`warnings`, and `to_dict()`. `get_session()` returns a dictionary matching the public session
shape. No network request happens during client construction.

## Grounding, size limits and failure safety

- `grounding` contains only native `textGroundingMetadata`, preserving each batch's
  own reference indices, segments, extracts, and document metadata. Different
  source URIs may share a generic connector document resource; do not identify
  documents solely by that resource suffix.
- A model-written “Native Citation” label does **not** create native references.
  Zero native references produce an explicit warning. That warning does not mean
  the quotation is false or that retrieval was not performed; the trace is absent.
- Source quotations still need independent verification. In initial live testing,
  default-model answers had native references but some non-verbatim quotations;
  the explicitly selected Argon answer had a matching quotation but no native
  references or tool-call names. Model choice alone does not ensure citation quality.
- Datastore selection is a server request field. Folder/file scope expressed only
  in the prompt is **not** a verified isolation boundary or complete coverage check.
- Responses are buffered, at most **16 MiB / 10,000 frames**, and must reach EOF
  with a named terminal `SUCCEEDED` answer. Partial, malformed, failed, skipped,
  cancelled, empty-text, or connector-auth-error responses are not returned as
  successful textual answers. Native segment reference indices/UTF-8 offsets are
  checked, without claiming semantic validation of citations.
- There are **no automatic generation retries**, including on 401, 429, server
  errors, broken streams or timeouts; redirects and transport-level retries are
  disabled too. An ambiguous failure may have recorded the turn. Inspect the
  session before any manual retry; never retry the question just to recover a
  missing response. There is no Google idempotency key or durable local replay
  guard in the client. Batch scripts should maintain their own exclusive attempt
  records.
- No prompt, response body, thoughts, token, or cookie is logged by this client. JSON results and stored session details are sensitive
  content: keep them private and out of version control. Python post-dispatch
  `GeminiChatError` exposes `session` (when known) and `do_not_retry=True`.

## Tool boundaries

This version exposes enterprise search and optional web grounding, **not mutative
connector actions, arbitrary shell/Python function calls, media generation or
agent/workflow execution**. The wider public API also defines other managed tool
specifications and calls to certain registered agents, but these were not added
just because the schema mentions them.

Google's current StreamAssist guide explicitly says programmatic mutative tools
(email drafting, calendar event creation, chat messaging, etc.) are unsupported
and may silently fail or loop. Registered A2A/ADK agents are not supported through
this endpoint. For custom function-calling, use a separately supported API/agent
integration with appropriate authorization; a Gemini Enterprise license does not
establish access/billing/availability for another API.

References:

- [StreamAssist reference](https://docs.cloud.google.com/gemini/enterprise/docs/reference/rest/v1/projects.locations.collections.engines.assistants/streamAssist)
- [Official invocation guide and limitations](https://docs.cloud.google.com/gemini/enterprise/docs/invoke-agent-streamassist)
- [Discovery schema](https://discoveryengine.googleapis.com/$discovery/rest?version=v1)
