use std::path::Path;

use anyhow::{Context, Result, bail};
use ingfah_client::{Error, Request};
use serde_json::Value;

use crate::cli::{BatchesCommand, Paging};
use crate::commands::read;
use crate::ctx::{Ctx, Sent};
use crate::output::{Columns, ListSpec, print_list, print_object, render_rows};

const RECORD_COLUMNS: Columns = &[
    ("ID", "/id"),
    ("Row", "/row_number"),
    ("Status", "/status"),
    ("Name", "/customer_name"),
    ("Phone", "/phone_number"),
    ("Calls", "/call_attempts"),
    (
        "Outcome",
        "/latest_chat_session/disposition_outcome/outcome",
    ),
    ("Last call", "/latest_chat_session/start_time"),
];

const INVALID_COLUMNS: Columns = &[
    ("Error", "/error"),
    ("Column", "/column"),
    ("Rows", "/row_numbers"),
];

const ALL_HOURS: &str = r#""time_slots": [{"from": "00:00", "to": "23:59"}]"#;

pub async fn run(ctx: &Ctx, cmd: &BatchesCommand) -> Result<()> {
    match cmd {
        BatchesCommand::Get { id } => {
            read::get(ctx, format!("/client/outbound/batches/{id}")).await
        }
        BatchesCommand::Records {
            id,
            status,
            search,
            paging,
        } => records(ctx, *id, status.as_deref(), search.as_deref(), paging).await,
        BatchesCommand::Create { file } => create(ctx, file).await,
        BatchesCommand::Pause { id } => {
            let note = format!("Stops dialing batch {id} until it is resumed.");
            action(ctx, *id, "pause", &note).await
        }
        BatchesCommand::Resume { id } => {
            let note = format!("Batch {id} starts calling real customers again.");
            action(ctx, *id, "resume", &note).await
        }
        BatchesCommand::Cancel { id } => {
            let note = format!(
                "Cancels batch {id} for good; it cannot be resumed. Calls in progress run to their end."
            );
            action(ctx, *id, "cancel", &note).await
        }
    }
}

async fn records(
    ctx: &Ctx,
    id: u64,
    status: Option<&str>,
    search: Option<&str>,
    paging: &Paging,
) -> Result<()> {
    let spec = ListSpec {
        path: format!("/client/outbound/batches/{id}/records"),
        rows: "/data",
        columns: RECORD_COLUMNS,
    };
    let req = Request::get(spec.path.as_str())
        .query_opt("status", status)
        .query_opt("search", search);
    print_list(ctx, &spec, req, paging).await
}

async fn action(ctx: &Ctx, id: u64, verb: &str, note: &str) -> Result<()> {
    let req = Request::post(format!("/client/outbound/batches/{id}/{verb}"));
    match ctx.write(req, Some(note)).await? {
        Sent::Sent(resp) => print_object(ctx.format, resp.body.json().unwrap_or(&Value::Null)),
        Sent::DryRun => Ok(()),
    }
}

/// Counts records and refuses schedules without `time_slots`, which the API answers with a 500.
fn check_batch(body: &Value) -> Result<(usize, usize)> {
    let schedules = body
        .get("schedules")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
        .context("the batch needs a non-empty `schedules` array")?;
    let mut records = 0;
    for (i, schedule) in schedules.iter().enumerate() {
        let has_slots = schedule
            .get("time_slots")
            .and_then(Value::as_array)
            .is_some_and(|s| !s.is_empty());
        if !has_slots {
            return Err(Error::Refused(format!(
                "schedule {} has no time_slots; the API fails without them. For all hours add {ALL_HOURS}",
                i + 1
            ))
            .into());
        }
        records += schedule
            .get("records")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
    }
    Ok((records, schedules.len()))
}

async fn create(ctx: &Ctx, file: &Path) -> Result<()> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let body: Value =
        serde_json::from_str(&text).with_context(|| format!("{} is not JSON", file.display()))?;
    let (records, schedules) = check_batch(&body)?;
    let note = format!(
        "This places real calls to {records} numbers across {schedules} schedule(s), within each schedule's time slots."
    );
    let req = Request::post("/client/outbound/batches").json(body);
    let resp = match ctx.write(req, Some(&note)).await? {
        Sent::Sent(resp) => resp,
        Sent::DryRun => return Ok(()),
    };
    let envelope = resp.body.json().cloned().unwrap_or(Value::Null);
    let invalids = envelope
        .pointer("/data/invalids")
        .and_then(Value::as_array)
        .filter(|i| !i.is_empty());
    if let Some(invalids) = invalids {
        eprintln!("{}", render_rows(invalids, INVALID_COLUMNS));
        bail!(
            "the batch was not created: {} problem(s) in the records",
            invalids.len()
        );
    }
    print_object(ctx.format, &envelope)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn check_batch_counts_records_across_schedules() {
        let body = json!({"schedules": [
            {"time_slots": [{"from": "09:00", "to": "17:00"}], "records": [{}, {}]},
            {"time_slots": [{"from": "09:00", "to": "17:00"}], "records": [{}]}
        ]});
        assert_eq!(check_batch(&body).ok(), Some((3, 2)));
    }

    #[test]
    fn check_batch_refuses_schedule_without_time_slots() {
        let body = json!({"schedules": [{"records": [{}]}]});
        assert!(check_batch(&body).is_err());
    }
}
