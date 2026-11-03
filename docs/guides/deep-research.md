# Deep Research — official Gemini Enterprise API

Deep Research is available through this repo's **Gemini Enterprise** client, not
NotebookLM's notebook/source API. It invokes the built-in `deep_research` agent
through Discovery Engine **v1 `streamAssist`**, using OAuth. Browser cookies and
private widget endpoints are not needed by the implementation.

Google documents API access as generally available **with allowlisting**. Your
app/account must have Deep Research enabled and permission to invoke the assistant.
An HTTP 200 by itself is not proof of agent execution: some tenants may silently
ignore an unknown agent. The built-in ID was verified in an authorized tenant;
do not substitute a numeric custom-agent ID or a web-portal CID.

See the [chat guide](gemini-enterprise-chat.md) for authentication, engine discovery,
location, CA bundles and proxy configuration. The client never enables APIs,
changes IAM, or changes app/connector settings.

## CLI: plan, review, explicitly start

Build the updated binary with `cargo make build` (then `target/debug/gemini-chat`),
or install it with `make install-chat`.

```sh
export GEMINI_CHAT_PROJECT_NUMBER=YOUR_PROJECT_NUMBER
export GEMINI_CHAT_LOCATION=us
export GEMINI_CHAT_ENGINE_ID=YOUR_ENGINE_ID
# gcloud auth login, if needed

# One generation request for a new plan; no automatic Start Research request.
gemini-chat --json research plan --data-store-id YOUR_DATASTORE_ID \
  'Compare Rust and Python for a CLI API client. Return a plan for review only.' \
  > plan.json

# Read the plan's contents and warnings. Copy its session resource.
# Only after review, explicitly authorize one research generation:
gemini-chat --json research start --session-id SESSION_FROM_PLAN \
  --data-store-id YOUR_DATASTORE_ID > research.json

# Read-only recovery after a timeout/rejected response; never generates:
gemini-chat --json research read --session-id SESSION_FROM_PLAN > recovered.json
# Full saved session for inspection:
gemini-chat --json session --session-id SESSION_FROM_PLAN --include-answer-details
```

Keep `plan.json`/`research.json` private (e.g. `umask 077`) and out of Git.
Datastore flags are repeatable and accept short IDs or canonical resources in the
configured project/location/collection. Omit them for engine defaults. Keep the
**same sources and `--web-grounding` setting on both turns**. The CLI and SDK do not
persist a trusted source binding between separate invocations. Public web grounding
is opt-in and requires tenant support.

Deep Research **does not enforce SharePoint folder/file isolation**. Do not infer
such isolation from a URL in the prompt or from citations. Use the separate
folder-scoped Search → Answer feature when that boundary is required. There is no
research `--model`; `GEMINI_CHAT_MODEL` applies only to `ask`. Research uses the
managed agent's model configuration, without fallback.

### Timeout and uncertain delivery

Research requests (including planning) default to **30 minutes**; ordinary chat
remains at 6 minutes. Override for the CLI with `--timeout 40m`. A verified research
run took over 15 minutes, with no headers/body visible until completion. The client
buffers responses to EOF; it does not promise progressive UI updates.

There are **no automatic generation retries**, including on authentication errors,
429/5xx, redirects, broken streams or timeouts. One command sends at most one POST.
After a dispatched failure, the server may still complete or record that turn.
Inspect the saved session and the research UI before deciding whether another
manually authorized attempt is appropriate. Ctrl-C stops waiting, not necessarily
server-side work. There is no public async-operation polling, cancellation,
idempotency key, or durable cross-process replay guard in this implementation.

## Result format and the missing-stage limitation

`--json`, Rust `ResearchResponse`, and Python's returned dictionary expose:

- `session`, `answer_resource`, `requested_agent`, `requested_phase` (`plan`/`start`/`read`).
  These are **requested** settings; the response must have a named terminal
  `SUCCEEDED` answer, but this alone does not attest research routing/completion.
- `contents`: ordered visible blocks, each with optional `text`, `kind`, `agent`,
  `file`, native `grounding` batches, and `unattributed_segments`. Intermediate research questions/answers
  are **not concatenated into a supposed final report**. Hidden thought blocks
  are excluded.
- `report_indices`: only nonempty text blocks explicitly labelled `RESEARCH_REPORT`
  by the service. Optional stage labels are preserved if the service returns them.
- `final_text_candidate_index`: the last nonempty text block **only when all stage
  labels are absent**. This is a convenience candidate, **not an attested report**.
- `warnings`, `invocation_tools`.

**Verified limitation (2026-11-02):** public v1 responses and session reads omitted
`contentMetadata`/`agent`, even for a run independently confirmed as Deep Research
in the signed-in research UI. Consequently an empty `report_indices` does not mean
research failed. Conversely, never promote arbitrary answer text to a completed
report merely because the request specified `deep_research`. Inspect the saved
research UI when stage attestation matters. The CLI and gesh prominently warn when
stages are absent; retain the complete JSON rather than just the candidate.

Grounding segment offsets are UTF-8 bytes **within each content block**, and
reference indices remain local to each metadata batch. The terminal snapshot may
repeat all earlier grounding; exact duplicates are retained only once per block.
Invalid offsets, out-of-range references, ambiguous unattached metadata,
conflicting labels, mismatched sessions, malformed/truncated responses, non-success
terminal states and over-limit responses fail closed. A verified service exception
is the native `referenceIndices: [-1]` marker: those segments still require valid
byte offsets, but are quarantined in `unattributed_segments` with their original
values and batch index. They are **not usable citations** and are excluded from the
validated grounding segments. Warnings count them; repeated terminal snapshots do
not double-count them. Other negative/out-of-range reference indices still fail. Native reference **entries**
are not necessarily distinct sources and do not establish citation truthfulness.
The existing limits remain 16 MiB and 10,000 frames.

Audio summaries, when returned, retain `fileId` and `mimeType` as **metadata only**.
No guessed download URL or unsupported download API is exposed. Private
`assistToken`, cookies and browser storage are not included in research results.

## Python

Rebuild with `cargo make chat-py-build` after Rust changes:

```python
from gemini_chat import GeminiEnterpriseClient, GcloudTokenProvider

client = GeminiEnterpriseClient(
    GcloudTokenProvider(), "123456789012", "your-engine-id", "us",
    research_timeout_secs=1800,  # separate from chat's timeout_secs=360
)
sources = {"data_store_ids": ["your-datastore"], "web_grounding": False}
plan = client.plan_research("Research CLI API testing. Return a plan only.", **sources)
print(plan["contents"])
# Deliberate user approval after review, not an automatic retry/start:
result = client.start_research(plan["session"], **sources)
print(result["report_indices"], result["final_text_candidate_index"], result["warnings"])
saved = client.get_session(result["session"], include_answer_details=True)
# Recovery/inspection without dispatching another generation:
recovered = client.get_research(plan["session"])
```

A dispatched failure raises `GeminiChatError` with `do_not_retry=True` and `session`
when known. The SDK releases the GIL while waiting. Callers must maintain their own
exclusive attempt records if building durable automation.

## Rust

`ResearchPlanRequest::new(topic)` and `ResearchStartRequest::new(session)` have
`data_stores`/`web_grounding` fields and are passed to
`GeminiEnterpriseClient::plan_research` / `start_research`.
`GeminiEnterpriseConfig::research_timeout` defaults to 1800 seconds, independent of
`timeout`. Both methods return `ResearchResponse`. `get_research(session)` reads
and validates the latest persisted assistant answer without generating. Recovery
sets `requested_phase` to `read` and `requested_agent` to null because it sends no
agent invocation; it does not infer a served agent from the session ID.

The request includes `agentsSpec.agentSpecs: [{"agentId":"deep_research"}]` and,
when selected, `toolsSpec.vertexAiSearchSpec.dataStoreSpecs: [{"dataStore":…}]`.
Both fields are **arrays**, even where prose examples in Google's how-to use
objects. Continuation sends `query.text = "Start Research"` in the plan's session.

## gesh

Research sessions are separate from chat and folder-answer sessions:

```text
/research plan Compare strategies for testing a CLI API client; plan only.
/research start
/research show
/research history
/research read SAVED_SESSION_ID
/research save research.json
```

`start` uses the successful plan's captured datastore/web settings even if `/ds`
or `/web` changed afterwards. It allows only **one start attempt per plan per
process**, including after failure or Ctrl-C; use read-only recovery or inspect
history/UI instead of resending. This guard is not durable across processes. Research generation commands reject
active folder mode (`/folder off` first). An abandoned generation blocks further
generations until its result arrives and does not hijack the chat session. Saved
JSON includes every visible block/grounding, uses private POSIX permissions, and
refuses to overwrite existing files. `/save` still saves chat transcripts; use
`/research save` for research.

## Validation

Deterministic tests cover exact requests, explicit start, session/resource validation,
research-specific deadlines, no retries, block separation, optional stage labels,
hidden thoughts, repeated terminal metadata, citations, Python bindings, CLI and
shell state. An ignored local replay test can validate a private capture without
committing it:

```sh
# Also set project/location/engine and CA variables for the capture's tenant.
GEMINI_CHAT_RESEARCH_CAPTURE=/private/path/response.json \
  cargo test -p gemini-chat replay_private_research_capture --lib -- --ignored
```

Never commit raw tenant captures, credentials, reports or browser state. Synthetic
fixtures are used in automated tests; live generation is deliberate and opt-in.

### Live verification, 2026-11-02

A fresh topic → plan → one explicit Start Research call was run in an authorized
`us` tenant. The server produced four research question/answer pairs, a 23,496-byte
report, and MP3 summary metadata in about 13.5 minutes. The first client parse
safely rejected three `-1` reference markers. After adding quarantine support,
`research read` recovered the same completed session **without another generation**.
The report's SHA-256 matched the signed-in research UI's explicitly labelled
`RESEARCH_REPORT` text exactly; the UI also confirmed `agent=deep_research`.
The recovered report retained 738 native reference entries (not necessarily distinct
sources), 108 validated segments and three quarantined unattributed segments.
No tenant content, identifiers, credentials or browser state were added to Git.

References:

- [Google Deep Research documentation](https://docs.cloud.google.com/gemini/enterprise/docs/research-assistant)
- [Invoke agents with streamAssist](https://docs.cloud.google.com/gemini/enterprise/docs/invoke-agent-streamassist)
- [Official v1 streamAssist reference](https://docs.cloud.google.com/gemini/enterprise/docs/reference/rest/v1/projects.locations.collections.engines.assistants/streamAssist)
