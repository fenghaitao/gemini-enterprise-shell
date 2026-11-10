//! OAuth access-token sources.

use async_trait::async_trait;
use tokio::process::Command;

use crate::error::{Error, Result};

/// Supplies an OAuth 2.0 access token with a Gemini Enterprise-capable scope
/// (for example `cloud-platform`).
#[async_trait]
pub trait TokenProvider: Send + Sync {
    async fn access_token(&self) -> Result<String>;
}

/// Runs `gcloud auth print-access-token`.
#[derive(Debug, Clone)]
pub struct GcloudTokenProvider {
    binary: String,
}

impl GcloudTokenProvider {
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

impl Default for GcloudTokenProvider {
    fn default() -> Self {
        Self::new("gcloud")
    }
}

#[async_trait]
impl TokenProvider for GcloudTokenProvider {
    async fn access_token(&self) -> Result<String> {
        let output = Command::new(&self.binary)
            .args(["auth", "print-access-token"])
            .output()
            .await
            .map_err(|err| {
                Error::TokenProvider(format!(
                    "failed to run gcloud; make sure the gcloud CLI is installed and on PATH: {err}"
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::TokenProvider(format!(
                "failed to get an access token from gcloud; run 'gcloud auth login'.\nIf browser login gets stuck, run 'gcloud auth login --force --no-launch-browser'. Open the printed URL, sign in, and paste the authorization code back into that terminal.\n{}",
                stderr.trim()
            )));
        }
        let token = String::from_utf8(output.stdout)
            .map_err(|_| Error::TokenProvider("gcloud returned a non-UTF-8 token".into()))?;
        Ok(token.trim().to_owned())
    }
}

/// Reads the token from an environment variable.
#[derive(Debug, Clone)]
pub struct EnvTokenProvider {
    key: String,
}

impl EnvTokenProvider {
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }
}

#[async_trait]
impl TokenProvider for EnvTokenProvider {
    async fn access_token(&self) -> Result<String> {
        std::env::var(&self.key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                Error::TokenProvider(format!("environment variable {} is missing", self.key))
            })
    }
}

/// A fixed token (tests, or tokens obtained elsewhere).
#[derive(Clone)]
pub struct StaticTokenProvider {
    token: String,
}

// Never print the bearer token.
impl std::fmt::Debug for StaticTokenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticTokenProvider")
            .field("token", &"<redacted>")
            .finish()
    }
}

impl StaticTokenProvider {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl TokenProvider for StaticTokenProvider {
    async fn access_token(&self) -> Result<String> {
        Ok(self.token.clone())
    }
}
