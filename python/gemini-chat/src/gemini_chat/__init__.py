"""Client for the official Gemini Enterprise assistant API (independent of NotebookLM)."""

from ._native import (
    EnvTokenProvider,
    GcloudTokenProvider,
    GeminiAssistResponse,
    GeminiChatError,
    GeminiEnterpriseClient,
    normalize_sharepoint_folder,
)

__all__ = [
    "EnvTokenProvider",
    "GcloudTokenProvider",
    "GeminiAssistResponse",
    "GeminiChatError",
    "GeminiEnterpriseClient",
    "normalize_sharepoint_folder",
]
