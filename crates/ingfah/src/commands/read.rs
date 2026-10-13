use anyhow::Result;
use ingfah_client::Request;

use crate::cli::{
    AnalyticsCommand, ListOnly, RevisionsCommand, SearchList, TeamsCommand, TemplatesCommand,
    ToolsCommand,
};
use crate::ctx::Ctx;
use crate::output::{Columns, ListSpec, print_list, print_object, print_rows};

const TEAM_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Name", "/product_name"),
    ("Direction", "/direction"),
    ("Channel", "/channel_type"),
    ("Number / channel", "/channel_identifier"),
    ("Status", "/status"),
    ("Visibility", "/visibility"),
];

const AGENT_COLUMNS: Columns = &[
    ("Slug", "/slug"),
    ("Name", "/name"),
    ("Type", "/type"),
    ("Visibility", "/visibility"),
    ("Has draft", "/has_draft"),
    ("Published", "/has_published"),
];

const REVISION_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Status", "/status"),
    ("Created", "/created_at"),
    ("Created by", "/created_admin/name"),
    ("Published", "/published_at"),
    ("Voice", "/voice/name"),
];

const PLUGIN_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Signature", "/signature"),
    ("Integration", "/integration"),
    ("Method", "/method"),
    ("Global", "/is_global"),
    ("Updated", "/updated_at"),
];

const SIMPLE_TOOL_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Name", "/name"),
    ("Description", "/description"),
];

const VOICE_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Name", "/name"),
    ("Gender", "/gender"),
    ("Description", "/description"),
];

const TEMPLATE_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Name", "/name"),
    ("Fields", "/fields"),
    ("Teams", "/client_products"),
    ("Updated", "/updated_at"),
];

fn spec(path: &str, columns: Columns) -> ListSpec {
    ListSpec {
        path: path.to_owned(),
        rows: "/data",
        columns,
    }
}

async fn search_list(ctx: &Ctx, spec: ListSpec, list: &SearchList) -> Result<()> {
    let req = Request::get(spec.path.as_str()).query_opt("search", list.search.as_deref());
    print_list(ctx, &spec, req, &list.paging).await
}

pub async fn teams(ctx: &Ctx, cmd: &TeamsCommand) -> Result<()> {
    match cmd {
        TeamsCommand::List {
            list,
            direction,
            channel_type,
        } => {
            let spec = spec("/client/ai-agent-teams", TEAM_COLUMNS);
            let req = Request::get(spec.path.as_str())
                .query_opt("search", list.search.as_deref())
                .query_opt("direction", direction.as_deref())
                .query_opt("type", channel_type.as_deref());
            print_list(ctx, &spec, req, &list.paging).await
        }
        TeamsCommand::Get { id } => get(ctx, format!("/client/products/{id}")).await,
    }
}

pub async fn agents_list(ctx: &Ctx, list: &SearchList, agent_type: Option<&str>) -> Result<()> {
    let spec = spec("/client/ai-agents", AGENT_COLUMNS);
    let req = Request::get(spec.path.as_str())
        .query_opt("search", list.search.as_deref())
        .query_opt("type", agent_type);
    print_list(ctx, &spec, req, &list.paging).await
}

pub async fn revisions(ctx: &Ctx, cmd: &RevisionsCommand) -> Result<()> {
    match cmd {
        RevisionsCommand::List { slug } => {
            let envelope = ctx
                .read(Request::get(format!("/client/agents/{slug}/revisions")))
                .await?;
            print_rows(ctx.format, &envelope, "/data/revisions", REVISION_COLUMNS)
        }
        RevisionsCommand::Get { slug, id } => {
            get(ctx, format!("/client/agents/{slug}/revisions/{id}")).await
        }
    }
}

pub async fn tools(ctx: &Ctx, cmd: &ToolsCommand) -> Result<()> {
    match cmd {
        ToolsCommand::List(list) => {
            search_list(ctx, spec("/client/plugin-functions", PLUGIN_COLUMNS), list).await
        }
        ToolsCommand::Get { id } => get(ctx, format!("/client/plugin-functions/{id}")).await,
    }
}

pub async fn phone_tools(ctx: &Ctx, ListOnly::List(list): &ListOnly) -> Result<()> {
    search_list(ctx, spec("/client/phone-tools", SIMPLE_TOOL_COLUMNS), list).await
}

pub async fn text_chat_tools(ctx: &Ctx, ListOnly::List(list): &ListOnly) -> Result<()> {
    search_list(
        ctx,
        spec("/client/text-chat-tools", SIMPLE_TOOL_COLUMNS),
        list,
    )
    .await
}

pub async fn voices(ctx: &Ctx, _: &ListOnly) -> Result<()> {
    let envelope = ctx.read(Request::get("/client/voices")).await?;
    print_rows(ctx.format, &envelope, "/data", VOICE_COLUMNS)
}

pub async fn templates(ctx: &Ctx, cmd: &TemplatesCommand) -> Result<()> {
    match cmd {
        TemplatesCommand::List(list) => {
            search_list(
                ctx,
                spec("/client/outbound/options", TEMPLATE_COLUMNS),
                list,
            )
            .await
        }
        TemplatesCommand::Get { id } => get(ctx, format!("/client/outbound/options/{id}")).await,
    }
}

pub async fn analytics(ctx: &Ctx, cmd: &AnalyticsCommand) -> Result<()> {
    let AnalyticsCommand::Summary {
        from,
        to,
        team,
        direction,
        timezone,
    } = cmd;
    let teams = (!team.is_empty()).then(|| {
        team.iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    });
    let req = Request::get("/client/analytics/summary")
        .query("from", from.as_str())
        .query("to", to.as_str())
        .query_opt("client_product_ids", teams)
        .query_opt("direction", direction.as_deref())
        .query_opt("timezone", timezone.as_deref());
    print_object(ctx.format, &ctx.read(req).await?)
}

pub async fn get(ctx: &Ctx, path: String) -> Result<()> {
    print_object(ctx.format, &ctx.read(Request::get(path)).await?)
}
