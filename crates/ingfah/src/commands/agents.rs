use anyhow::{Context, Result, anyhow, bail};
use ingfah_client::agent::{
    self, AgentType, BodyKind, FieldChange, ToolIds, check_revision_body, diff_bodies,
    newest_revision, unwrap_data,
};
use ingfah_client::{Error, Request};
use serde_json::{Map, Value, json};
use similar::TextDiff;

use crate::cli::{AgentTypeArg, CreateArgs, FieldArgs, Format, ReviseArgs, Visibility};
use crate::ctx::{Ctx, Sent};
use crate::output::print_json;

struct AgentState {
    detail: Value,
    agent_type: AgentType,
}

async fn load_agent(ctx: &Ctx, slug: &str) -> Result<AgentState> {
    let envelope = ctx
        .read(Request::get(format!("/client/agents/{slug}")))
        .await?;
    let detail = unwrap_data(&envelope, "agent").clone();
    let agent_type = AgentType::of(&detail);
    Ok(AgentState { detail, agent_type })
}

async fn load_revision(ctx: &Ctx, slug: &str, id: u64) -> Result<Value> {
    let envelope = ctx
        .read(Request::get(format!(
            "/client/agents/{slug}/revisions/{id}"
        )))
        .await?;
    Ok(unwrap_data(&envelope, "revision").clone())
}

async fn newest(ctx: &Ctx, slug: &str) -> Result<agent::RevisionSummary> {
    let list = ctx
        .read(Request::get(format!("/client/agents/{slug}/revisions")))
        .await?;
    newest_revision(&list).with_context(|| format!("agent `{slug}` has no revisions"))
}

fn refused(message: String) -> anyhow::Error {
    Error::Refused(message).into()
}

pub async fn revise(ctx: &Ctx, args: &ReviseArgs) -> Result<()> {
    let slug = &args.slug;
    let state = load_agent(ctx, slug).await?;
    let base = newest(ctx, slug).await?;
    if base.status == "draft" {
        eprintln!(
            "warning: building on draft #{}, which is not published yet",
            base.id
        );
    }
    let revision = load_revision(ctx, slug, base.id).await?;
    let preserved = agent::tools_to_preserve(&state.detail, base.id);
    let old = agent::base_body(&revision, &preserved, state.agent_type);

    let mut new = old.clone();
    apply_edits(
        &mut new,
        BodyKind::CreateRevision,
        state.agent_type,
        &args.fields,
    )?;
    let removed = agent::apply_tool_edits(
        &mut new,
        state.agent_type,
        &args.fields.add_tool,
        &args.remove_tool,
    )
    .map_err(refused)?;

    let mut must_keep = preserved;
    for tool in removed {
        must_keep.remove(tool);
    }
    let violations = check_revision_body(state.agent_type, &must_keep, &Value::Object(new.clone()));
    if !violations.is_empty() && !args.allow_tool_drop {
        let lines: Vec<String> = violations.iter().map(|v| format!("  - {v}")).collect();
        return Err(refused(format!(
            "refusing to create a revision that drops tools:\n{}\nName a tool with --remove-tool to unlink it, or pass --allow-tool-drop.",
            lines.join("\n")
        )));
    }

    let changes = diff_bodies(&old, &new);
    if changes.is_empty() {
        eprintln!("No changes from revision #{}; nothing sent.", base.id);
        return Ok(());
    }
    eprintln!("Changes from revision #{}:", base.id);
    for change in &changes {
        eprint!("{}", render_change(change));
    }

    let req =
        Request::post(format!("/client/agents/{slug}/revisions")).json(Value::Object(new.clone()));
    let note = "Creates a draft revision. Nothing goes live until you publish it.";
    let created_envelope = match ctx.write(req, Some(note)).await? {
        Sent::Sent(resp) => resp.body.json().cloned().unwrap_or(Value::Null),
        Sent::DryRun => return Ok(()),
    };
    let created = unwrap_data(&created_envelope, "revision");
    let new_id = created
        .get("id")
        .and_then(Value::as_u64)
        .context("the create response carried no revision id")?;

    let stored = load_revision(ctx, slug, new_id).await?;
    let mismatches = agent::readback_mismatches(&new, &stored, created, state.agent_type);
    if !mismatches.is_empty() {
        bail!(
            "revision #{new_id} was created but what was stored differs from what was sent:\n  - {}\nDo not publish it; inspect it with `ingfah agents revisions get {slug} {new_id}`.",
            mismatches.join("\n  - ")
        );
    }

    let publish = format!("ingfah agents publish {slug} --revision {new_id}");
    match ctx.format {
        Format::Json => print_json(&json!({"revision_id": new_id, "publish_command": publish}))?,
        Format::Table => {
            println!("Created draft revision #{new_id}; the stored values match what was sent.");
            println!("Publish it with:\n  {publish}");
        }
    }
    Ok(())
}

pub async fn create(ctx: &Ctx, args: &CreateArgs) -> Result<()> {
    let (agent_type, type_name) = match args.agent_type {
        AgentTypeArg::Audio => (AgentType::Audio, "audio"),
        AgentTypeArg::Text => (AgentType::Text, "text"),
    };
    let visibility = match args.visibility {
        Visibility::Private => "private",
        Visibility::Public => "public",
    };
    let mut body = Map::new();
    body.insert("type".to_owned(), json!(type_name));
    body.insert("visibility".to_owned(), json!(visibility));
    apply_edits(&mut body, BodyKind::CreateAgent, agent_type, &args.fields)?;
    agent::apply_tool_edits(&mut body, agent_type, &args.fields.add_tool, &[]).map_err(refused)?;

    let missing = agent::missing_create_fields(&body, agent_type);
    if !missing.is_empty() {
        return Err(refused(format!(
            "refusing to create an agent without its required fields: {}\nSet each with --set FIELD=VALUE, --set-file FIELD=PATH or --from-file.",
            missing.join(", ")
        )));
    }
    let violations = check_revision_body(
        agent_type,
        &ToolIds::default(),
        &Value::Object(body.clone()),
    );
    if !violations.is_empty() {
        let lines: Vec<String> = violations.iter().map(|v| format!("  - {v}")).collect();
        return Err(refused(format!(
            "refusing to create an agent whose flow uses tools it does not link:\n{}\nLink each tool with --add-tool KIND:ID.",
            lines.join("\n")
        )));
    }

    let req = Request::post("/client/agents").json(Value::Object(body.clone()));
    let note = "Creates an unpublished agent with a draft revision. Nothing goes live until you publish it.";
    let created_envelope = match ctx.write(req, Some(note)).await? {
        Sent::Sent(resp) => resp.body.json().cloned().unwrap_or(Value::Null),
        Sent::DryRun => return Ok(()),
    };
    let created = unwrap_data(&created_envelope, "agent");
    let slug = created
        .get("slug")
        .and_then(Value::as_str)
        .context("the create response carried no agent slug")?;
    let agent_id = created
        .get("id")
        .and_then(Value::as_u64)
        .context("the create response carried no agent id")?;

    let stored = load_agent(ctx, slug).await?;
    let draft = stored
        .detail
        .get("ai_agent_instruction_revision")
        .filter(|d| d.is_object())
        .with_context(|| {
            format!("agent `{slug}` was created, but reading it back shows no draft revision; inspect it with `ingfah agents get {slug}`")
        })?;
    let revision_id = draft
        .get("id")
        .and_then(Value::as_u64)
        .with_context(|| format!("the draft revision of `{slug}` has no id"))?;
    let mismatches = agent::readback_mismatches(&body, draft, draft, agent_type);
    if !mismatches.is_empty() {
        bail!(
            "agent `{slug}` was created but its draft revision #{revision_id} differs from what was sent:\n  - {}\nDo not publish it; inspect it with `ingfah agents get {slug}`.",
            mismatches.join("\n  - ")
        );
    }

    let publish = format!("ingfah agents publish {slug} --revision {revision_id}");
    match ctx.format {
        Format::Json => print_json(&json!({
            "slug": slug,
            "agent_id": agent_id,
            "revision_id": revision_id,
            "publish_command": publish,
        }))?,
        Format::Table => {
            println!("Created agent `{slug}` (id {agent_id}).");
            println!("Draft revision #{revision_id}; the stored values match what was sent.");
            println!("Publish it with:\n  {publish}");
        }
    }
    Ok(())
}

fn apply_edits(
    body: &mut Map<String, Value>,
    target: BodyKind,
    agent_type: AgentType,
    args: &FieldArgs,
) -> Result<()> {
    if let Some(path) = &args.from_file {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let edits: Map<String, Value> = serde_json::from_str(&text)
            .with_context(|| format!("{} must hold a JSON object of fields", path.display()))?;
        for (field, value) in edits {
            agent::set_field(body, target, agent_type, &field, value).map_err(|e| anyhow!(e))?;
        }
    }
    for (field, path) in &args.set_file {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        let value = agent::parse_field_value(target, field, &text).map_err(|e| anyhow!(e))?;
        agent::set_field(body, target, agent_type, field, value).map_err(|e| anyhow!(e))?;
    }
    for (field, raw) in &args.set {
        let value = agent::parse_field_value(target, field, raw).map_err(|e| anyhow!(e))?;
        agent::set_field(body, target, agent_type, field, value).map_err(|e| anyhow!(e))?;
    }
    Ok(())
}

fn render_change(change: &FieldChange) -> String {
    let as_text = |v: &Option<Value>| match v {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => serde_json::to_string_pretty(other).ok(),
    };
    let (old, new) = (as_text(&change.old), as_text(&change.new));
    let multiline = [&old, &new].iter().any(|s| {
        s.as_deref()
            .is_some_and(|s| s.contains('\n') || s.len() > 60)
    });
    if multiline {
        let old = old.unwrap_or_default();
        let new = new.unwrap_or_default();
        let diff = TextDiff::from_lines(&old, &new);
        let body = diff
            .unified_diff()
            .context_radius(2)
            .header(
                &format!("{} (before)", change.key),
                &format!("{} (after)", change.key),
            )
            .to_string();
        format!("{body}\n")
    } else {
        format!(
            "{}: {} -> {}\n",
            change.key,
            old.unwrap_or_else(|| "(absent)".to_owned()),
            new.unwrap_or_else(|| "(absent)".to_owned())
        )
    }
}

fn voice_id(v: &Value) -> Option<u64> {
    v.pointer("/voice/id").and_then(Value::as_u64)
}

pub async fn publish(ctx: &Ctx, slug: &str, revision_id: u64, allow_tool_drop: bool) -> Result<()> {
    let newest = newest(ctx, slug).await?;
    if newest.id != revision_id {
        return Err(refused(format!(
            "revision #{revision_id} is not the newest revision of `{slug}`; #{} is. Review #{} before publishing anything.",
            newest.id, newest.id
        )));
    }
    if newest.status == "published" {
        return Err(refused(format!(
            "revision #{revision_id} is already published"
        )));
    }
    let before = load_agent(ctx, slug).await?;
    let agent_type = before.agent_type;
    let tools_before = ToolIds::from_detail(&before.detail).applicable(agent_type);
    let expected_tools = agent::draft_tools(&before.detail, revision_id)
        .map(|t| t.applicable(agent_type))
        .unwrap_or_else(|| tools_before.clone());
    let revision = load_revision(ctx, slug, revision_id).await?;
    let check_voice = agent_type == AgentType::Audio;
    let voice_before = voice_id(&before.detail);
    let expected_voice = voice_id(&revision);

    let mut note =
        format!("Puts revision #{revision_id} live: every new call or chat with `{slug}` uses it.");
    let tool_changes = tools_before.changes_to(&expected_tools);
    let dropped: Vec<&String> = tool_changes.iter().filter(|c| c.starts_with('-')).collect();
    if !dropped.is_empty() && !allow_tool_drop {
        let dropped: Vec<&str> = dropped.iter().map(|c| c.as_str()).collect();
        return Err(refused(format!(
            "refusing to publish revision #{revision_id}: it unbinds tools that are bound now: {}\nPass --allow-tool-drop if that is intended.",
            dropped.join(" ")
        )));
    }
    if !tool_changes.is_empty() {
        note.push_str(&format!("\nTool links change: {}", tool_changes.join(" ")));
    }
    if check_voice && voice_before != expected_voice {
        note.push_str(&format!(
            "\nVoice changes: {} -> {}",
            show_id(voice_before),
            show_id(expected_voice)
        ));
    }
    let req = Request::post(format!(
        "/client/agents/{slug}/revisions/{revision_id}/publish"
    ));
    match ctx.write(req, Some(&note)).await? {
        Sent::Sent(_) => {}
        Sent::DryRun => return Ok(()),
    }

    let after = load_agent(ctx, slug).await?;
    let tools_after = ToolIds::from_detail(&after.detail).applicable(agent_type);
    let voice_after = voice_id(&after.detail);
    let mut problems = Vec::new();
    let unexpected = expected_tools.changes_to(&tools_after);
    if !unexpected.is_empty() {
        problems.push(format!(
            "bound tools differ from the revision's: {}",
            unexpected.join(" ")
        ));
    }
    if check_voice && voice_after != expected_voice {
        problems.push(format!(
            "voice is {} but the revision has {}",
            show_id(voice_after),
            show_id(expected_voice)
        ));
    }
    if !problems.is_empty() {
        bail!(
            "revision #{revision_id} was published, but the live agent does not match it:\n  - {}\nCheck `ingfah agents get {slug}` now.",
            problems.join("\n  - ")
        );
    }
    let tools: Vec<String> = tools_after.iter().map(|t| t.to_string()).collect();
    match ctx.format {
        Format::Json => print_json(&json!({
            "published_revision_id": revision_id,
            "tools": tools,
            "voice_id": voice_after,
        }))?,
        Format::Table => {
            println!("Published revision #{revision_id} of `{slug}`.");
            println!(
                "Bound tools: {}",
                if tools.is_empty() {
                    "none".to_owned()
                } else {
                    tools.join(" ")
                }
            );
            if check_voice {
                println!("Voice: {}", show_id(voice_after));
            }
        }
    }
    Ok(())
}

fn show_id(id: Option<u64>) -> String {
    id.map_or_else(|| "none".to_owned(), |id| format!("#{id}"))
}
