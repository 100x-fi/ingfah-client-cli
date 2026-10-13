use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use ingfah_client::agent::ToolRef;

const AFTER_HELP: &str = "\
Exit codes: 0 ok, 1 API or runtime error, 2 usage error, 3 refused by a guard
(unknown route, tool drop, missing confirmation).

Writes ask for confirmation on a terminal; pass --yes to skip it, or --dry-run
to print the request without sending it.";

/// Command-line client for the Ingfah client API (API-key auth).
#[derive(Debug, Parser)]
#[command(name = "ingfah", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Profile: picks the stored key and the profile's base URL.
    #[arg(long, env = "INGFAH_PROFILE", default_value = "default", global = true)]
    pub profile: String,

    /// API key; overrides INGFAH_API_KEY and the stored key.
    #[arg(long, global = true, hide_env_values = true)]
    pub api_key: Option<String>,

    /// API base URL [default: https://api.ingfah.ai].
    #[arg(long, env = "INGFAH_API_BASE_URL", global = true)]
    pub base_url: Option<String>,

    /// Output format [default: table on a terminal, json otherwise].
    #[arg(long, value_enum, global = true)]
    pub format: Option<Format>,

    /// Request timeout in seconds (downloads always get 120).
    #[arg(long, default_value_t = 30, global = true)]
    pub timeout: u64,

    /// Skip confirmation prompts for writes.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Print the write request that would be sent, and send nothing.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// More logging on stderr: -v requests, -vv headers (key redacted).
    #[arg(long, short = 'v', action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Store, inspect or remove the API key for a profile.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Call any allowed route directly and print the JSON.
    Api(ApiArgs),
    /// AI agent teams (products).
    #[command(subcommand)]
    Teams(TeamsCommand),
    /// AI agents, their revisions, and publishing.
    #[command(subcommand)]
    Agents(AgentsCommand),
    /// Plugin functions (Tools).
    #[command(subcommand)]
    Tools(ToolsCommand),
    /// Phone tools (Tools in the Calling category).
    #[command(subcommand)]
    PhoneTools(ListOnly),
    /// Text chat tools.
    #[command(subcommand)]
    TextChatTools(ListOnly),
    /// Voices an agent can speak with.
    #[command(subcommand)]
    Voices(ListOnly),
    /// Voice calls: list, transcript, recording.
    #[command(subcommand)]
    Calls(CallsCommand),
    /// Customer templates (outbound options).
    #[command(subcommand)]
    Templates(TemplatesCommand),
    /// Outbound call batches.
    #[command(subcommand)]
    Batches(BatchesCommand),
    /// Call analytics.
    #[command(subcommand)]
    Analytics(AnalyticsCommand),
    /// Print a shell completion script.
    Completion {
        /// Target shell.
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Verify a key and store it in the system keyring.
    Login {
        /// Read the key from stdin instead of prompting.
        #[arg(long)]
        with_key_stdin: bool,
    },
    /// Show where the key comes from, the masked key, the base URL, and whether it works.
    Status,
    /// Delete the profile's key from the keyring.
    Logout,
}

#[derive(Debug, Args)]
pub struct ApiArgs {
    /// HTTP method.
    pub method: String,
    /// Path such as /client/agents/mali; no query string (use -q).
    pub path: String,
    /// Query parameter; repeat for several, or for a repeated key.
    #[arg(short = 'q', long = "query", value_name = "KEY=VALUE", value_parser = parse_key_value)]
    pub query: Vec<(String, String)>,
    /// JSON body: inline, @FILE, or - for stdin.
    #[arg(long)]
    pub body: Option<String>,
    /// Write the response body to FILE (never overwrites).
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// Allow a revision body that drops tools bound to the agent.
    #[arg(long)]
    pub allow_tool_drop: bool,
}

#[derive(Debug, Args)]
pub struct Paging {
    /// Page number, from 1.
    #[arg(long, default_value_t = 1)]
    pub page: u32,
    /// Rows per page.
    #[arg(long)]
    pub per_page: Option<u32>,
    /// Walk every page.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Subcommand)]
pub enum ListOnly {
    /// List them.
    List(SearchList),
}

#[derive(Debug, Args)]
pub struct SearchList {
    /// Search text.
    #[arg(long)]
    pub search: Option<String>,
    #[command(flatten)]
    pub paging: Paging,
}

#[derive(Debug, Subcommand)]
pub enum TeamsCommand {
    /// List teams.
    List {
        #[command(flatten)]
        list: SearchList,
        /// inbound or outbound.
        #[arg(long)]
        direction: Option<String>,
        /// audio or text.
        #[arg(long = "type")]
        channel_type: Option<String>,
    },
    /// Show one team.
    Get {
        /// Team (product) id.
        id: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentsCommand {
    /// List agents.
    List {
        #[command(flatten)]
        list: SearchList,
        /// audio or text.
        #[arg(long = "type")]
        agent_type: Option<String>,
    },
    /// Show one agent, with the published prompt and bound tools.
    Get {
        /// Agent slug.
        slug: String,
    },
    /// List or show revisions.
    #[command(subcommand)]
    Revisions(RevisionsCommand),
    /// Create a new draft revision from the newest one, changing only what you name.
    Revise(ReviseArgs),
    /// Put a revision live, checking that tools and voice come out as expected.
    Publish {
        /// Agent slug.
        slug: String,
        /// Revision id; must be the newest revision.
        #[arg(long)]
        revision: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum RevisionsCommand {
    /// List an agent's revisions, newest first.
    List {
        /// Agent slug.
        slug: String,
    },
    /// Show one revision in full.
    Get {
        /// Agent slug.
        slug: String,
        /// Revision id.
        id: u64,
    },
}

const REVISE_AFTER_HELP: &str = "\
Fields: name, vocal_name, ai_greeting_message, ai_instruction_identity,
ai_instruction_task, ai_instructions, ai_instruction_flow (JSON),
ai_instruction_variables (JSON), voice_id (JSON), voice_speed (JSON),
prompt_engine_version.

Tool kinds: plugin, phone (audio agents), text (text agents).

Example:
  ingfah agents revise mali --set-file ai_instruction_task=task.md --add-tool plugin:12
  ingfah agents publish mali --revision 18";

#[derive(Debug, Args)]
#[command(after_help = REVISE_AFTER_HELP)]
pub struct ReviseArgs {
    /// Agent slug.
    pub slug: String,
    /// Set a field: FIELD=VALUE.
    #[arg(long = "set", value_name = "FIELD=VALUE", value_parser = parse_key_value)]
    pub set: Vec<(String, String)>,
    /// Set a field from a file's contents: FIELD=PATH.
    #[arg(long = "set-file", value_name = "FIELD=PATH", value_parser = parse_key_value)]
    pub set_file: Vec<(String, String)>,
    /// JSON object of field values to apply.
    #[arg(long, value_name = "FILE")]
    pub from_file: Option<PathBuf>,
    /// Link a tool: KIND:ID.
    #[arg(long, value_name = "KIND:ID")]
    pub add_tool: Vec<ToolRef>,
    /// Unlink a tool: KIND:ID.
    #[arg(long, value_name = "KIND:ID")]
    pub remove_tool: Vec<ToolRef>,
    /// Allow a body that drops bound tools without naming them in --remove-tool.
    #[arg(long)]
    pub allow_tool_drop: bool,
}

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    /// List plugin functions.
    List(SearchList),
    /// Show one plugin function.
    Get {
        /// Plugin function id.
        id: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum CallsCommand {
    /// List calls.
    List(CallsList),
    /// Show one call; table format prints the transcript.
    Get {
        /// Call (chat session) uuid.
        uuid: String,
    },
    /// Recording checksum, presigned URL, or download.
    Recording {
        /// Call (chat session) uuid.
        uuid: String,
        /// Download the audio to FILE (never overwrites).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Print the presigned URL, valid for 15 minutes.
        #[arg(long)]
        show_url: bool,
    },
}

#[derive(Debug, Args)]
pub struct CallsList {
    /// Calls starting at or after this RFC 3339 time.
    #[arg(long)]
    pub from: Option<String>,
    /// Calls starting before this RFC 3339 time.
    #[arg(long)]
    pub to: Option<String>,
    /// inbound or outbound.
    #[arg(long)]
    pub direction: Option<String>,
    /// Team (product) id; repeat for several.
    #[arg(long)]
    pub team: Vec<u64>,
    /// Disposition outcome; repeat for several.
    #[arg(long)]
    pub outcome: Vec<String>,
    /// Client phone number; repeat for several.
    #[arg(long)]
    pub phone: Vec<String>,
    /// Search text.
    #[arg(long)]
    pub search: Option<String>,
    #[command(flatten)]
    pub paging: Paging,
}

#[derive(Debug, Subcommand)]
pub enum TemplatesCommand {
    /// List customer templates.
    List(SearchList),
    /// Show one template.
    Get {
        /// Template (outbound option) id.
        id: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum BatchesCommand {
    /// Show one batch with its progress.
    Get {
        /// Batch id (client_batch_id).
        id: u64,
    },
    /// List a batch's records.
    Records {
        /// Batch id (client_batch_id).
        id: u64,
        /// Record status.
        #[arg(long)]
        status: Option<String>,
        /// Search text.
        #[arg(long)]
        search: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Create a batch from a JSON file. Places real calls.
    Create {
        /// The request body as JSON.
        #[arg(long)]
        file: PathBuf,
    },
    /// Stop dialing a batch until resumed.
    Pause {
        /// Batch id.
        id: u64,
    },
    /// Resume dialing a paused batch.
    Resume {
        /// Batch id.
        id: u64,
    },
    /// Cancel a batch for good.
    Cancel {
        /// Batch id.
        id: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum AnalyticsCommand {
    /// Call totals for a window of at most 93 days.
    Summary {
        /// Window start, RFC 3339.
        #[arg(long)]
        from: String,
        /// Window end (exclusive), RFC 3339.
        #[arg(long)]
        to: String,
        /// Team (product) id; repeat for several.
        #[arg(long)]
        team: Vec<u64>,
        /// inbound or outbound.
        #[arg(long)]
        direction: Option<String>,
        /// IANA time zone [server default: Asia/Bangkok].
        #[arg(long)]
        timezone: Option<String>,
    },
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| format!("expected KEY=VALUE, got `{s}`"))
}
