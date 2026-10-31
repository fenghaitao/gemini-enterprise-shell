//! `gemini-chat`: chat with the official Gemini Enterprise assistant (Discovery Engine v1).
use std::{process::ExitCode, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use gemini_chat::{
    AssistRequest, AssistResponse, EnvTokenProvider, GcloudTokenProvider, GeminiEnterpriseClient,
    GeminiEnterpriseConfig, TokenProvider,
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
    Session(SessionArgs),
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
        Command::Session(args) => {
            let session = client
                .get_session(&args.session_id, args.include_answer_details)
                .await?;
            if global.json {
                println!("{}", serde_json::to_string_pretty(&session)?);
            } else {
                println!("Session: {}", session.name);
                for (i, turn) in session.turns.iter().enumerate() {
                    println!(
                        "Turn {} model: {}",
                        i + 1,
                        turn.model_id().unwrap_or("not reported")
                    );
                }
                println!("{}", serde_json::to_string_pretty(&session)?);
            }
        }
    }
    Ok(())
}

fn emit_answer(response: &AssistResponse, json_mode: bool) -> Result<()> {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(response)?);
    } else {
        println!("{}\n\nSession: {}", response.answer, response.session);
        if let Some(model) = &response.requested_model {
            println!("Requested model: {model}");
        }
        println!(
            "Native grounding references: {}",
            response.native_reference_count()
        );
        for (batch, metadata) in response.grounding.iter().enumerate() {
            for (index, reference) in metadata.references.iter().enumerate() {
                let meta = &reference.document_metadata;
                println!(
                    "  [{batch}:{index}] {} — {}",
                    meta.title.as_deref().unwrap_or("untitled"),
                    meta.uri.as_deref().unwrap_or("URI not provided")
                );
            }
        }
    }
    for warning in &response.warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}
