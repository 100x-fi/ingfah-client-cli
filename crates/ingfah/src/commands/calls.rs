use std::path::Path;

use anyhow::Result;
use ingfah_client::{Body, Request};
use serde_json::{Value, json};

use crate::cli::{CallsCommand, CallsList, Format};
use crate::ctx::{Ctx, refuse_existing, write_new_file};
use crate::output::{Columns, ListSpec, print_json, print_list};

const CALL_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Start", "/start_time"),
    ("Direction", "/direction"),
    ("Customer", "/customer_phone"),
    ("Seconds", "/duration_sec"),
    ("Team", "/team/product_name"),
    ("Outcome", "/disposition_outcome/outcome"),
    ("Title", "/title"),
];

pub async fn run(ctx: &Ctx, cmd: &CallsCommand) -> Result<()> {
    match cmd {
        CallsCommand::List(args) => list(ctx, args).await,
        CallsCommand::Get { uuid } => get(ctx, uuid).await,
        CallsCommand::Recording {
            uuid,
            out,
            show_url,
        } => match out {
            Some(out) => download(ctx, uuid, out).await,
            None => recording_info(ctx, uuid, *show_url).await,
        },
    }
}

async fn list(ctx: &Ctx, args: &CallsList) -> Result<()> {
    let spec = ListSpec {
        path: "/client/chat-sessions-list".to_owned(),
        rows: "/data/rows",
        columns: CALL_COLUMNS,
    };
    let mut req = Request::get(spec.path.as_str())
        .query_opt("start_time", args.from.as_deref())
        .query_opt("end_time", args.to.as_deref())
        .query_opt("direction", args.direction.as_deref())
        .query_opt("search", args.search.as_deref());
    for team in &args.team {
        req = req.query("product_id[]", team.to_string());
    }
    for outcome in &args.outcome {
        req = req.query("outcome[]", outcome.as_str());
    }
    for phone in &args.phone {
        req = req.query("client_phone[]", phone.as_str());
    }
    print_list(ctx, &spec, req, &args.paging).await
}

async fn get(ctx: &Ctx, uuid: &str) -> Result<()> {
    let envelope = ctx
        .read(Request::get(format!("/client/chat-sessions/{uuid}")))
        .await?;
    let call = envelope.get("data").unwrap_or(&envelope);
    match ctx.format {
        Format::Json => print_json(call),
        Format::Table => {
            print!("{}", transcript(call));
            Ok(())
        }
    }
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => other.to_string(),
    }
}

fn clock(timestamp: &str) -> &str {
    timestamp.get(11..19).unwrap_or(timestamp)
}

/// Header lines, then one line per message. Older sessions carry `chat_logs` instead of `messages`.
fn transcript(call: &Value) -> String {
    let mut out = String::new();
    for (label, value) in [
        ("Title", text(call, "title")),
        ("Start", text(call, "start_time")),
        ("End", text(call, "end_time")),
        ("Direction", text(call, "direction")),
        ("Customer", text(call, "customer_phone")),
        (
            "Outcome",
            call.pointer("/disposition_outcome/outcome")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        ("Summary", text(call, "summary")),
    ] {
        if !value.is_empty() {
            out.push_str(&format!("{label:<10} {value}\n"));
        }
    }
    out.push('\n');
    let empty = Vec::new();
    let messages = call
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    for m in messages {
        let at = clock(
            m.get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .to_owned();
        let line = match m.get("role").and_then(Value::as_str) {
            Some("tool") => {
                let params = m
                    .get("parameters")
                    .filter(|p| !p.is_null())
                    .map(|p| format!(" {p}"))
                    .unwrap_or_default();
                format!(
                    "tool: {} ({}){params}",
                    text(m, "tool_name"),
                    text(m, "status")
                )
            }
            Some("assistant") => format!("Agent: {}", text(m, "content")),
            Some("user") => format!("Customer: {}", text(m, "content")),
            other => format!("{}: {}", other.unwrap_or("?"), text(m, "content")),
        };
        out.push_str(&format!("[{at}] {line}\n"));
    }
    if messages.is_empty() {
        let logs = call
            .get("chat_logs")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        for log in logs {
            let who = if log
                .get("is_agent")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "Agent"
            } else {
                "Customer"
            };
            out.push_str(&format!("{who}: {}\n", text(log, "chat_log")));
        }
    }
    out
}

async fn recording_info(ctx: &Ctx, uuid: &str, show_url: bool) -> Result<()> {
    let envelope = ctx
        .read(Request::get(format!("/client/chat-sessions/{uuid}/record")))
        .await?;
    let data = envelope.get("data").unwrap_or(&envelope);
    let checksum = text(data, "checksum");
    let url = show_url.then(|| text(data, "audio_record_url"));
    match ctx.format {
        Format::Json => {
            let mut out = json!({"uuid": uuid, "checksum": checksum});
            if let Some(url) = url {
                out["audio_record_url"] = json!(url);
                out["url_valid_minutes"] = json!(15);
            }
            print_json(&out)
        }
        Format::Table => {
            println!("Checksum: {checksum}");
            match url {
                Some(url) => println!("URL (valid for 15 minutes, every open is logged):\n{url}"),
                None => eprintln!(
                    "Pass --out FILE to download the audio, or --show-url for a link valid 15 minutes."
                ),
            }
            Ok(())
        }
    }
}

async fn download(ctx: &Ctx, uuid: &str, out: &Path) -> Result<()> {
    refuse_existing(out)?;
    let req = Request::get(format!("/client/chat-sessions/{uuid}/record/download")).raw();
    let bytes = match ctx.client.send(&req).await?.body {
        Body::Bytes(b) => b,
        Body::Text(t) => t.into_bytes(),
        Body::Json(v) => serde_json::to_vec(&v)?,
    };
    write_new_file(out, &bytes)?;
    eprintln!("Wrote {} bytes to {}.", bytes.len(), out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_labels_speakers_and_tools() {
        let call = json!({"title": "Policy", "messages": [
            {"role": "assistant", "content": "สวัสดีค่ะ", "timestamp": "2026-10-01T03:00:01Z"},
            {"role": "user", "content": "Hi", "timestamp": "2026-10-01T03:00:04Z"},
            {"role": "tool", "tool_name": "lookup_policy", "status": "success", "timestamp": "2026-10-01T03:00:05Z"}
        ]});
        let lines: Vec<String> = transcript(&call)
            .lines()
            .skip(2)
            .map(str::to_owned)
            .collect();
        assert_eq!(
            lines,
            [
                "[03:00:01] Agent: สวัสดีค่ะ",
                "[03:00:04] Customer: Hi",
                "[03:00:05] tool: lookup_policy (success)"
            ]
        );
    }

    #[test]
    fn transcript_falls_back_to_chat_logs() {
        let call = json!({"chat_logs": [{"is_agent": true, "chat_log": "Hello"}]});
        assert!(transcript(&call).contains("Agent: Hello"));
    }
}
