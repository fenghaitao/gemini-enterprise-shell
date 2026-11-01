//! `gemini-chat`: chat with the official Gemini Enterprise assistant (Discovery Engine v1).
use std::{
    io::{self, Write},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use gemini_chat::{
    AssistRequest, AssistResponse, EnvTokenProvider, FolderAnswer, FolderAnswerRequest,
    GcloudTokenProvider, GeminiEnterpriseClient, GeminiEnterpriseConfig, SearchRequest,
    SearchResults, SharePointFolder, TokenProvider,
};

const TOKEN_ENV: &str = "GEMINI_CHAT_ACCESS_TOKEN";

#[derive(Parser)]
#[command(
    name = "gemini-chat",
    version,
    about = "Chat with the official Gemini Enterprise assistant (OAuth, Discovery Engine v1)"
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct GlobalArgs {
    /// Google Cloud project number (not the project ID).
    #[arg(long, env = "GEMINI_CHAT_PROJECT_NUMBER", global = true)]
    project_number: Option<String>,
    /// Gemini Enterprise location: us, eu or global.
    #[arg(
        long,
        env = "GEMINI_CHAT_LOCATION",
        default_value = "global",
        global = true
    )]
    location: String,
    /// Actual Discovery Engine engine (app) ID, not the web portal CID UUID.
    #[arg(long, env = "GEMINI_CHAT_ENGINE_ID", global = true)]
    engine_id: Option<String>,
    #[arg(
        long,
        env = "GEMINI_CHAT_ASSISTANT_ID",
        default_value = "default_assistant",
        global = true
    )]
    assistant_id: String,
    #[arg(
        long,
        env = "GEMINI_CHAT_COLLECTION_ID",
        default_value = "default_collection",
        global = true
    )]
    collection_id: String,
    /// Token source: gcloud (`gcloud auth print-access-token`) or env (GEMINI_CHAT_ACCESS_TOKEN).
    #[arg(long, value_enum, env = "GEMINI_CHAT_AUTH", default_value_t = Auth::Gcloud, global = true)]
    auth: Auth,
    /// Per-request timeout, e.g. 6m or 90s (default 6m).
    #[arg(long, value_parser = humantime::parse_duration, global = true)]
    timeout: Option<Duration>,
    /// Override the API base (must end in /v1). Intended for tests and proxies.
    #[arg(long, env = "GEMINI_CHAT_BASE_URL", global = true, hide = true)]
    base_url: Option<String>,
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Auth {
    Gcloud,
    Env,
}

#[derive(Subcommand)]
enum Command {
    /// Ask once. Never retries generation or falls back to another model.
    Ask(AskArgs),
    /// Read a saved server session and its service-reported model metadata (no generation).
    /// Prints a summary, or the full session with --json.
    Session(SessionArgs),
    /// Search files (no generation). With --sharepoint-folder, results are restricted to
    /// that folder by SharePoint and re-checked locally.
    Search(SearchArgs),
    /// Answer only from files in one SharePoint folder: a folder-scoped search with
    /// --search-terms, then an answer from just those files (one generation). The CLI
    /// is single-turn; use gesh or the Python client for trusted multi-turn sessions.
    Answer(FolderAnswerArgs),
}

#[derive(Args)]
struct SearchArgs {
    /// Restrict to this folder (recursive); copy the URL from the browser.
    #[arg(long, value_name = "URL")]
    sharepoint_folder: Option<String>,
    /// Datastore to search. Repeatable without a folder; folder scope requires exactly one.
    #[arg(long = "data-store-id", value_name = "ID")]
    data_stores: Vec<String>,
    /// Maximum results, 1-100.
    #[arg(long, default_value_t = 10)]
    page_size: u32,
    /// Keywords. With a folder every word must match, so avoid full sentences.
    #[arg(required = true, num_args = 1..)]
    terms: Vec<String>,
}

#[derive(Args)]
struct FolderAnswerArgs {
    /// Answer only from files in this folder (recursive).
    #[arg(long, value_name = "URL")]
    sharepoint_folder: String,
    /// Keywords that find the relevant files (every word must match), separate from the question.
    #[arg(long, value_name = "KEYWORDS")]
    search_terms: String,
    /// The one SharePoint file datastore to search.
    #[arg(long = "data-store-id", value_name = "ID")]
    data_store: String,
    /// Files to retrieve, 1-100. Only about the top 5 arrive with full text.
    #[arg(long, default_value_t = 10)]
    page_size: u32,
    question: String,
}

#[derive(Args)]
struct AskArgs {
    /// Explicit API model ID, e.g. gemini-4-argon. Omit for the assistant default.
    #[arg(long, env = "GEMINI_CHAT_MODEL")]
    model: Option<String>,
    /// Continue a server session (short ID or full resource). Omit to start a new one.
    #[arg(long, value_name = "ID")]
    session_id: Option<String>,
    /// Select datastores (repeatable; short ID or full resource). Omit for engine defaults.
    /// Prompt folder/URL restrictions are NOT a verified backend isolation boundary.
    #[arg(long = "data-store-id", value_name = "ID")]
    data_stores: Vec<String>,
    /// Opt in to public web grounding (requires assistant/admin support).
    #[arg(long)]
    web_grounding: bool,
    question: String,
}

#[derive(Args)]
struct SessionArgs {
    #[arg(long, value_name = "ID")]
    session_id: String,
    /// Include stored answers and their native metadata, not just queries/configuration.
    #[arg(long)]
    include_answer_details: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        // The reader went away (e.g. `gemini-chat ... | head`): stop quietly, like other CLIs.
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let global = cli.global;
    let project = global
        .project_number
        .context("set --project-number or GEMINI_CHAT_PROJECT_NUMBER")?;
    let engine = global
        .engine_id
        .context("set --engine-id or GEMINI_CHAT_ENGINE_ID")?;
    let mut config = GeminiEnterpriseConfig::new(project, &global.location, engine);
    config.assistant_id = global.assistant_id;
    config.collection_id = global.collection_id;
    if let Some(timeout) = global.timeout {
        config.timeout = timeout;
    }
    let provider: Arc<dyn TokenProvider> = match global.auth {
        Auth::Gcloud => Arc::new(GcloudTokenProvider::default()),
        Auth::Env => Arc::new(EnvTokenProvider::new(TOKEN_ENV)),
    };
    let mut client = GeminiEnterpriseClient::new(provider, config)?;
    if let Some(base) = &global.base_url {
        client = client.with_base_url(base)?;
    }

    match cli.command {
        Command::Ask(args) => {
            let mut request = AssistRequest::new(args.question);
            request.model_id = args.model;
            request.session = args.session_id;
            request.data_stores = args.data_stores;
            request.web_grounding = args.web_grounding;
            let response = client.ask(request).await?;
            emit_answer(&response, global.json)?;
        }
        Command::Search(args) => {
            let mut request = SearchRequest::new(args.terms.join(" "));
            request.folder = args
                .sharepoint_folder
                .as_deref()
                .map(SharePointFolder::parse)
                .transpose()?;
            request.data_stores = args.data_stores;
            request.page_size = args.page_size;
            let results = client.search(request).await?;
            emit_search(&results, global.json)?;
        }
        Command::Answer(args) => {
            let folder = SharePointFolder::parse(&args.sharepoint_folder)?;
            let mut request = FolderAnswerRequest::new(args.question, args.search_terms, folder);
            request.data_stores = vec![args.data_store];
            request.page_size = args.page_size;
            let answer = client.answer_in_folder(request).await?;
            emit_folder_answer(&answer, global.json)?;
        }
        Command::Session(args) => {
            let session = client
                .get_session(&args.session_id, args.include_answer_details)
                .await?;
            let mut out = io::stdout().lock();
            if global.json {
                writeln!(out, "{}", serde_json::to_string_pretty(&session)?)?;
            } else {
                writeln!(out, "Session: {}", session.name)?;
                if let Some(title) = &session.display_name {
                    writeln!(out, "Title: {title}")?;
                }
                for (i, turn) in session.turns.iter().enumerate() {
                    let query = turn.query.as_ref().map_or("", |q| q.text.as_str());
                    let kind = if turn.answer.is_some() || turn.detailed_answer.is_some() {
                        "folder answer".to_string()
                    } else {
                        format!("model {}", turn.model_id().unwrap_or("not reported"))
                    };
                    writeln!(out, "Turn {}: {kind} | {query}", i + 1)?;
                }
                if args.include_answer_details {
                    writeln!(out, "(answer details are included with --json)")?;
                }
            }
            out.flush()?;
        }
    }
    Ok(())
}

fn emit_answer(response: &AssistResponse, json_mode: bool) -> Result<()> {
    let mut out = io::stdout().lock();
    if json_mode {
        writeln!(out, "{}", serde_json::to_string_pretty(response)?)?;
    } else {
        writeln!(out, "{}\n\nSession: {}", response.answer, response.session)?;
        if let Some(model) = &response.requested_model {
            writeln!(out, "Requested model: {model}")?;
        }
        writeln!(
            out,
            "Native grounding references: {}",
            response.native_reference_count()
        )?;
        for (batch, metadata) in response.grounding.iter().enumerate() {
            for (index, reference) in metadata.references.iter().enumerate() {
                let meta = &reference.document_metadata;
                writeln!(
                    out,
                    "  [{batch}:{index}] {} — {}",
                    meta.title.as_deref().unwrap_or("untitled"),
                    meta.uri.as_deref().unwrap_or("URI not provided")
                )?;
            }
        }
    }
    out.flush()?;
    drop(out);
    for warning in &response.warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

fn emit_search(results: &SearchResults, json_mode: bool) -> Result<()> {
    let mut out = io::stdout().lock();
    if json_mode {
        writeln!(out, "{}", serde_json::to_string_pretty(results)?)?;
    } else {
        writeln!(out, "Query: {}", results.query)?;
        writeln!(out, "{} result(s)", results.hits.len())?;
        for (i, hit) in results.hits.iter().enumerate() {
            let text = if hit.content.is_some() {
                "full text"
            } else {
                "snippet"
            };
            writeln!(
                out,
                "  [{}] {} — {} ({text})",
                i + 1,
                hit.title.as_deref().unwrap_or("untitled"),
                hit.uri.as_deref().unwrap_or("URI not provided")
            )?;
        }
    }
    out.flush()?;
    drop(out);
    for warning in &results.warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

fn emit_folder_answer(answer: &FolderAnswer, json_mode: bool) -> Result<()> {
    let mut out = io::stdout().lock();
    if json_mode {
        writeln!(out, "{}", serde_json::to_string_pretty(answer)?)?;
    } else {
        writeln!(out, "{}\n", answer.answer)?;
        writeln!(out, "Session: {}", answer.session)?;
        writeln!(out, "Folder: {}", answer.folder)?;
        let full = answer.sources.iter().filter(|s| s.full_text).count();
        writeln!(
            out,
            "Files used: {} ({full} full text, {} snippet)",
            answer.sources.len(),
            answer.sources.len() - full
        )?;
        writeln!(out, "References: {}", answer.references.len())?;
        for (i, reference) in answer.references.iter().enumerate() {
            writeln!(
                out,
                "  [{i}] {} — {}",
                reference.title.as_deref().unwrap_or("untitled"),
                reference.uri.as_deref().unwrap_or("URI not provided")
            )?;
        }
    }
    out.flush()?;
    drop(out);
    for warning in &answer.warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    })
}
