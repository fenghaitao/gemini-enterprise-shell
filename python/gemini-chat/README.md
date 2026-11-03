# gemini-chat (Python)

Python client for the official **Gemini Enterprise assistant API** (Discovery Engine v1
`streamAssist` and `sessions.get`). Independent of NotebookLM: built from the
`gemini-chat` Rust crate via `crates/gemini-chat-python`; it does not need the `nblm`
package.

```python
from gemini_chat import GeminiEnterpriseClient, GcloudTokenProvider

client = GeminiEnterpriseClient(
    GcloudTokenProvider(), "123456789012", "your-engine-id", "us"
)
answer = client.ask(
    "Find the relevant source and cite it.",
    model_id="gemini-4-argon",
    data_store_ids=["your-sharepoint-file-datastore"],
)
print(answer.answer, answer.session, answer.warnings)
```

Answers limited to one SharePoint folder (folder-scoped Search, then the Answer API
with only those files):

```python
folder = "https://example.sharepoint.com/sites/eng/Shared Documents/Team/Specs"
hits = client.search("sync handshake", sharepoint_folder=folder,
                     data_store_ids=["your-sharepoint-file-datastore"])
scoped = client.answer_in_folder(
    "What does the spec say the handshake does?",
    search_terms="sync handshake",          # keywords: every word must match
    sharepoint_folder=folder,
    data_store_ids=["your-sharepoint-file-datastore"],
)
print(scoped["answer"], [r["uri"] for r in scoped["references"]], scoped["warnings"])
follow_up = client.answer_in_folder(
    "Shorten that.", search_terms="sync handshake", sharepoint_folder=folder,
    data_store_ids=["your-sharepoint-file-datastore"], session_id=scoped["session"],
)   # trusted because the same client created the session; searches the folder again
```

Folder-session provenance is intentionally client-local: only a session created earlier
by the same `GeminiEnterpriseClient` instance can be continued. A fresh process/client
cannot safely attest the original folder scope, so importing an arbitrary session ID is
rejected.

Token providers: `GcloudTokenProvider()` (`gcloud auth print-access-token`) or
`EnvTokenProvider()` (reads `GEMINI_CHAT_ACCESS_TOKEN`). Failures raise
`GeminiChatError` with `session`, `do_not_retry` and `skipped_reasons`.
Behind an HTTPS-inspecting proxy, set `GEMINI_CHAT_CA_BUNDLE` (or `SSL_CERT_FILE` /
`REQUESTS_CA_BUNDLE`).

Develop from this directory:

```bash
uv run --extra dev maturin develop
uv run --extra dev pytest
```

See `docs/guides/gemini-enterprise-chat.md` in the repository for details.


## Deep Research

The same client exposes `plan_research(topic, *, data_store_ids=None,
web_grounding=False)` and `start_research(session_id, *, data_store_ids=None,
web_grounding=False)`, returning dictionaries. Review the plan before explicitly
starting; keep identical source settings. Research uses a separate
`research_timeout_secs=1800` constructor option (chat stays at 360 seconds).
No model override or automatic generation retry is performed.
`get_research(session_id)` recovers the latest saved result without generating.

Results preserve ordered `contents` with per-block grounding and optional audio
file metadata. `report_indices` contains only service-labelled reports; when
public v1 omits stages, `final_text_candidate_index` is unverified and warnings
explain the limitation. Native `-1` reference markers are quarantined in
`unattributed_segments`, not treated as valid citations. Datastore selection is
not folder isolation. See the
[full guide](../../docs/guides/deep-research.md) for examples and recovery safety.
