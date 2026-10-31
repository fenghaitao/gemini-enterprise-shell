# Gemini Enterprise chat — official public API

`gemini-chat` talks to Gemini Enterprise's **official Discovery Engine v1
assistant API**. It is independent of NotebookLM: the `gemini-chat` library crate
and the `gemini-chat` command (crate `gemini-chat-cli`) do not depend on
`nblm-core` or `nblm`. The Python bindings ship in the `nblm` package as
`GeminiEnterpriseClient`, so one SDK covers both notebooks and chat.

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

The endpoint is derived from `--location`, e.g.
`https://us-discoveryengine.googleapis.com/v1/`. A hidden `--base-url`
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
| `--location` / `GEMINI_CHAT_LOCATION` | `global` (or `us`, `eu`) |
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

Rebuild the editable extension after Rust changes (`make py-build`):

```python
from nblm import GeminiEnterpriseClient, GcloudTokenProvider

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
  `NblmError` exposes `session` (when known) and `do_not_retry=True`.

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
