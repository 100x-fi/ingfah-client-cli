pub mod agents;
pub mod api;
pub mod auth;
pub mod batches;
pub mod calls;
pub mod read;

use anyhow::Result;

use crate::cli::{AgentsCommand, Command};
use crate::ctx::Ctx;

pub async fn dispatch(ctx: &Ctx, command: &Command) -> Result<()> {
    match command {
        Command::Api(args) => api::run(ctx, args).await,
        Command::Teams(cmd) => read::teams(ctx, cmd).await,
        Command::Agents(cmd) => agents_command(ctx, cmd).await,
        Command::Tools(cmd) => read::tools(ctx, cmd).await,
        Command::PhoneTools(cmd) => read::phone_tools(ctx, cmd).await,
        Command::TextChatTools(cmd) => read::text_chat_tools(ctx, cmd).await,
        Command::Voices(cmd) => read::voices(ctx, cmd).await,
        Command::Templates(cmd) => read::templates(ctx, cmd).await,
        Command::Analytics(cmd) => read::analytics(ctx, cmd).await,
        Command::Calls(cmd) => calls::run(ctx, cmd).await,
        Command::Batches(cmd) => batches::run(ctx, cmd).await,
        Command::Auth(_) | Command::Completion { .. } => {
            anyhow::bail!("auth and completion run without an API context")
        }
    }
}

async fn agents_command(ctx: &Ctx, cmd: &AgentsCommand) -> Result<()> {
    match cmd {
        AgentsCommand::List { list, agent_type } => {
            read::agents_list(ctx, list, agent_type.as_deref()).await
        }
        AgentsCommand::Get { slug } => read::get(ctx, format!("/client/agents/{slug}")).await,
        AgentsCommand::Revisions(cmd) => read::revisions(ctx, cmd).await,
        AgentsCommand::Revise(args) => agents::revise(ctx, args).await,
        AgentsCommand::Publish { slug, revision } => agents::publish(ctx, slug, *revision).await,
    }
}
