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
