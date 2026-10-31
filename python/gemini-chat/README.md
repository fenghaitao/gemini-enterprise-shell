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
