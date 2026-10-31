//! Client for the official Gemini Enterprise assistant API.
//!
//! Talks to Discovery Engine v1 `streamAssist` (generation) and `sessions.get`
//! (readback) with an OAuth access token, plus SharePoint folder-scoped Search and
//! Answer (see [`GeminiEnterpriseClient::answer_in_folder`]). Independent of NotebookLM.

mod auth;
mod client;
mod error;
mod models;
mod sharepoint;
mod tls;

pub use auth::{EnvTokenProvider, GcloudTokenProvider, StaticTokenProvider, TokenProvider};
pub use client::{
    AnswerCitation, AnswerReference, AnswerSource, FolderAnswer, FolderAnswerRequest,
    GeminiEnterpriseClient, GeminiEnterpriseConfig, SearchHit, SearchRequest, SearchResults,
    MAX_SEARCH_RESULTS,
};
pub use error::{Error, Result};
pub use models::*;
pub use sharepoint::SharePointFolder;
