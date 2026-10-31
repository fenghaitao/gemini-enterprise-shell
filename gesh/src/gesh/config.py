"""Settings from command-line flags and environment variables.

Each setting is read from ``GESH_<NAME>`` first, then ``NBLM_<NAME>`` (so an existing
nblm environment, e.g. ``source nblm-rs/local/env.sh``, works unchanged).
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field


def env(name: str, default: str | None = None) -> str | None:
    for prefix in ("GESH_", "NBLM_"):
        value = os.environ.get(prefix + name, "").strip()
        if value:
            return value
    return default


def _split(value: str | None) -> list[str]:
    return [part.strip() for part in (value or "").split(",") if part.strip()]


@dataclass
class Config:
    project_number: str | None = None
    location: str = "us"
    endpoint_location: str | None = None  # NotebookLM only; defaults to `location`
    engine_id: str | None = None
    collection_id: str = "default_collection"
    assistant_id: str = "default_assistant"
    model: str | None = None  # None = assistant default
    data_stores: list[str] = field(default_factory=list)
    web_grounding: bool = False
    sharepoint_folder: str | None = None  # folder scope: answers only from this folder
    search_terms: str | None = None  # keywords that find files in the folder
    timeout_secs: float = 360.0
    auth: str = "gcloud"  # "gcloud" or "env" (token in NBLM_ACCESS_TOKEN)

    @classmethod
    def from_env(cls) -> Config:
        location = env("LOCATION", "us") or "us"
        return cls(
            project_number=env("PROJECT_NUMBER"),
            location=location,
            endpoint_location=env("ENDPOINT_LOCATION"),
            engine_id=env("ENGINE_ID"),
            collection_id=env("COLLECTION_ID", "default_collection") or "default_collection",
            assistant_id=env("ASSISTANT_ID", "default_assistant") or "default_assistant",
            model=env("ENTERPRISE_MODEL"),
            data_stores=_split(env("DATA_STORE_IDS") or env("DATA_STORE_ID")),
            sharepoint_folder=env("SHAREPOINT_FOLDER"),
            search_terms=env("SEARCH_TERMS"),
        )

    @property
    def notebook_endpoint_location(self) -> str:
        return self.endpoint_location or self.location
