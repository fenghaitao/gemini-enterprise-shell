use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Gemini Enterprise API read failed: {0}")]
    Api(String),
    #[error("Gemini Enterprise generation failed: {reason}; session={session:?}. The server may have recorded this turn; do not automatically repeat the question.")]
    Assist {
        reason: String,
        session: Option<String>,
    },
    /// The assistant declined to answer (e.g. greetings); nothing was generated.
    #[error("Gemini Enterprise skipped this question ({}): {}", reasons.join(", "), skip_hint(reasons))]
    Skipped {
        reasons: Vec<String>,
        session: Option<String>,
    },
    #[error("token provider error: {0}")]
    TokenProvider(String),
    #[error("request error: {0}")]
    Request(#[from] reqwest::Error),
    #[error("url parse error: {0}")]
    Url(#[from] url::ParseError),
    #[error("validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn skip_hint(reasons: &[String]) -> &'static str {
    if reasons
        .iter()
        .any(|r| r == "NON_ASSIST_SEEKING_QUERY_IGNORED")
    {
        "the assistant ignores greetings and small talk; ask a concrete question"
    } else {
        "no answer was generated; rephrase the question"
    }
}

impl Error {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}
