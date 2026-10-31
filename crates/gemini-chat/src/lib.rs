//! Client for the official Gemini Enterprise assistant API.
//!
//! Talks to Discovery Engine v1 `streamAssist` (generation) and `sessions.get`
//! (readback) with an OAuth access token. Independent of NotebookLM.

mod auth;
mod client;
mod error;
mod models;
mod tls;

pub use auth::{EnvTokenProvider, GcloudTokenProvider, StaticTokenProvider, TokenProvider};
pub use client::{GeminiEnterpriseClient, GeminiEnterpriseConfig};
pub use error::{Error, Result};
pub use models::*;
