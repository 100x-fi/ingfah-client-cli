use anyhow::Result;
use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};
use ingfah_client::Request;
use serde_json::Value;

use crate::cli::{Format, Paging};
use crate::ctx::Ctx;

/// Table columns: header and the JSON pointer of the cell within a row.
pub type Columns = &'static [(&'static str, &'static str)];

const CELL_MAX_CHARS: usize = 80;
const ALL_PAGES_PER_PAGE: u32 = 100;

/// A list endpoint and how to show it.
pub struct ListSpec {
    pub path: String,
    /// JSON pointer to the row array in the envelope.
    pub rows: &'static str,
    pub columns: Columns,
}

pub fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn cell(value: Option<&Value>) -> String {
    let text = match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) if items.iter().all(is_scalar) => items
            .iter()
            .map(|v| cell(Some(v)))
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::Array(items)) => format!("[{} items]", items.len()),
        Some(v) => v.to_string(),
    };
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(CELL_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn is_scalar(v: &Value) -> bool {
    !(v.is_array() || v.is_object())
}

fn new_table() -> Table {
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

pub fn render_rows(rows: &[Value], columns: Columns) -> Table {
    let mut table = new_table();
    table.set_header(columns.iter().map(|(h, _)| *h));
    for row in rows {
        table.add_row(columns.iter().map(|(_, ptr)| cell(row.pointer(ptr))));
    }
    table
}

/// A two-column field/value table of an object's top-level fields.
pub fn render_object(object: &Value) -> Table {
    let mut table = new_table();
    table.set_header(["Field", "Value"]);
    if let Some(map) = object.as_object() {
        for (k, v) in map {
            table.add_row([k.clone(), cell(Some(v))]);
        }
    }
    table
}

pub fn print_object(format: Format, envelope: &Value) -> Result<()> {
    let data = envelope.get("data").unwrap_or(envelope);
    match format {
        Format::Json => print_json(data),
        Format::Table => {
            println!("{}", render_object(data));
            Ok(())
        }
    }
}

fn pagination(envelope: &Value) -> Option<&Value> {
    envelope
        .get("pagination")
        .or_else(|| envelope.pointer("/data/pagination"))
}

/// Fetches one page, or every page with `--all`, and prints the rows.
pub async fn print_list(ctx: &Ctx, spec: &ListSpec, base: Request, paging: &Paging) -> Result<()> {
    let per_page = paging.per_page.or(paging.all.then_some(ALL_PAGES_PER_PAGE));
    let mut page = paging.page;
    let mut rows = Vec::new();
    let mut more_hint = None;
    loop {
        let req = base
            .clone()
            .query("page", page.to_string())
            .query_opt("per_page", per_page);
        let envelope = ctx.read(req).await?;
        let batch = envelope
            .pointer(spec.rows)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let fetched = batch.len();
        rows.extend(batch);
        let p = pagination(&envelope);
        let total_pages = p.and_then(|p| p.get("total_pages")).and_then(Value::as_u64);
        let has_next = p
            .and_then(|p| p.get("has_next"))
            .and_then(Value::as_bool)
            .unwrap_or_else(|| total_pages.is_some_and(|t| u64::from(page) < t));
        let last = fetched == 0
            || !has_next
            || total_pages.is_some_and(|t| u64::from(page) >= t)
            || p.is_none();
        if !paging.all || last {
            if !paging.all && !last {
                more_hint = Some(format!(
                    "page {page} of {}; use --page {} or --all for more",
                    total_pages.map_or_else(|| "?".to_owned(), |t| t.to_string()),
                    page + 1
                ));
            }
            break;
        }
        page += 1;
    }
    match ctx.format {
        Format::Json => print_json(&Value::Array(rows))?,
        Format::Table => println!("{}", render_rows(&rows, spec.columns)),
    }
    if let Some(hint) = more_hint {
        eprintln!("{hint}");
    }
    Ok(())
}

/// Prints every row of an unpaged list.
pub fn print_rows(
    format: Format,
    envelope: &Value,
    rows_ptr: &str,
    columns: Columns,
) -> Result<()> {
    let rows = envelope
        .pointer(rows_ptr)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match format {
        Format::Json => print_json(&Value::Array(rows)),
        Format::Table => {
            println!("{}", render_rows(&rows, columns));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn cell_reads_nested_pointer() {
        let row = json!({"team": {"product_name": "Support"}});
        assert_eq!(cell(row.pointer("/team/product_name")), "Support");
    }

    #[test]
    fn cell_renders_null_as_empty() {
        assert_eq!(cell(Some(&Value::Null)), "");
    }

    #[test]
    fn cell_counts_object_arrays() {
        assert_eq!(cell(Some(&json!([{"a": 1}, {"a": 2}]))), "[2 items]");
    }

    #[test]
    fn cell_truncates_long_text() {
        assert_eq!(cell(Some(&json!("x".repeat(100)))).chars().count(), 81);
    }

    #[test]
    fn render_rows_has_one_line_per_row() {
        let rows = vec![json!({"id": 1}), json!({"id": 2})];
        let table = render_rows(&rows, &[("ID", "/id")]);
        assert_eq!(table.row_count(), 2);
    }
}
