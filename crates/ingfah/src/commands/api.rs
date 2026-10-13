use std::io::{IsTerminal, Read, Write};

use anyhow::{Context, Result, bail};
use ingfah_client::agent::{self, AgentType, ToolIds};
use ingfah_client::{Body, Effect, Error, Method, Request};
use serde_json::Value;

use crate::cli::ApiArgs;
use crate::ctx::{Ctx, Sent, refuse_existing, write_new_file};
use crate::output::print_json;

const REVISIONS_ROUTE: &str = "/client/agents/{slug}/revisions";

pub async fn run(ctx: &Ctx, args: &ApiArgs) -> Result<()> {
    let method = Method::parse(&args.method)
        .with_context(|| format!("unknown HTTP method `{}`", args.method))?;
    if !args.path.starts_with('/') {
        bail!("the path must start with `/`");
    }
    if args.path.contains('?') {
        bail!("put query parameters in -q KEY=VALUE, not in the path");
    }
    let mut req = Request::new(method, args.path.as_str());
    req.query.clone_from(&args.query);
    if let Some(body) = &args.body {
        req.body = Some(read_body(body)?);
    }
    if let Some(out) = &args.out {
        refuse_existing(out)?;
        req = req.raw();
    }
    let route = req.route()?;
    if method == Method::Post && route.path == REVISIONS_ROUTE && !args.allow_tool_drop {
        guard_revision(ctx, &req).await?;
    }
    let resp = if route.effect == Effect::Read {
        ctx.client.send(&req).await?
    } else {
        match ctx.write(req, None).await? {
            Sent::Sent(resp) => resp,
            Sent::DryRun => return Ok(()),
        }
    };
    if let Some(out) = &args.out {
        let bytes = match resp.body {
            Body::Bytes(b) => b,
            Body::Text(t) => t.into_bytes(),
            Body::Json(v) => serde_json::to_vec_pretty(&v)?,
        };
        write_new_file(out, &bytes)?;
        eprintln!("Wrote {} bytes to {}.", bytes.len(), out.display());
        return Ok(());
    }
    match resp.body {
        Body::Json(v) => print_json(&v),
        Body::Text(t) => {
            print!("{t}");
            Ok(())
        }
        Body::Bytes(b) => {
            if std::io::stdout().is_terminal() {
                return Err(Error::Refused(
                    "the response is binary; pass --out FILE to save it".to_owned(),
                )
                .into());
            }
            std::io::stdout().write_all(&b)?;
            Ok(())
        }
    }
}

fn read_body(arg: &str) -> Result<Value> {
    let text = if arg == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else if let Some(path) = arg.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?
    } else {
        arg.to_owned()
    };
    serde_json::from_str(&text).context("the body is not valid JSON")
}

/// Refuses a raw revision body that would unbind the agent's tools on publish.
async fn guard_revision(ctx: &Ctx, req: &Request) -> Result<()> {
    let slug = req.path.split('/').nth(3).unwrap_or_default();
    let envelope = ctx
        .read(Request::get(format!("/client/agents/{slug}")))
        .await?;
    let detail = agent::unwrap_data(&envelope, "agent");
    let empty = Value::Object(serde_json::Map::new());
    let body = req.body.as_ref().unwrap_or(&empty);
    let violations =
        agent::check_revision_body(AgentType::of(detail), &ToolIds::from_detail(detail), body);
    if violations.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = violations.iter().map(|v| format!("  - {v}")).collect();
    Err(Error::Refused(format!(
        "refusing to create a revision that drops tools:\n{}\nA revision is a full snapshot: send every tool id, or pass --allow-tool-drop if the drop is intended.",
        lines.join("\n")
    ))
    .into())
}
