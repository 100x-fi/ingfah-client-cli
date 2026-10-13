pub mod api;
pub mod auth;
pub mod read;

use anyhow::Result;

use crate::cli::{AgentsCommand, Command};
use crate::ctx::Ctx;

pub async fn dispatch(ctx: &Ctx, command: &Command) -> Result<()> {
    match command {
        Command::Api(args) => api::run(ctx, args).await,
        Command::Teams(cmd) => read::teams(ctx, cmd).await,
        Command::Agents(cmd) => agents(ctx, cmd).await,
        Command::Tools(cmd) => read::tools(ctx, cmd).await,
        Command::PhoneTools(cmd) => read::phone_tools(ctx, cmd).await,
        Command::TextChatTools(cmd) => read::text_chat_tools(ctx, cmd).await,
        Command::Voices(cmd) => read::voices(ctx, cmd).await,
        Command::Templates(cmd) => read::templates(ctx, cmd).await,
        Command::Analytics(cmd) => read::analytics(ctx, cmd).await,
        Command::Calls(_) | Command::Batches(_) => anyhow::bail!("not implemented yet"),
        Command::Auth(_) | Command::Completion { .. } => {
            unreachable!("handled before a context is built")
        }
    }
}

async fn agents(ctx: &Ctx, cmd: &AgentsCommand) -> Result<()> {
    match cmd {
        AgentsCommand::List { list, agent_type } => {
            read::agents_list(ctx, list, agent_type.as_deref()).await
        }
        AgentsCommand::Get { slug } => read::get(ctx, format!("/client/agents/{slug}")).await,
        AgentsCommand::Revisions(cmd) => read::revisions(ctx, cmd).await,
        AgentsCommand::Revise(_) | AgentsCommand::Publish { .. } => {
            anyhow::bail!("not implemented yet")
        }
    }
}
