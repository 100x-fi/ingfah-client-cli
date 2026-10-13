use std::io::IsTerminal;

use anyhow::Result;
use ingfah_client::{Client, Effect, Error, Request, Response};
use serde_json::{Value, json};

use crate::cli::Format;

const PROMPT_BODY_LINES: usize = 40;

/// Everything a command needs: the client and the global safety switches.
pub struct Ctx {
    pub client: Client,
    pub format: Format,
    pub yes: bool,
    pub dry_run: bool,
}

/// What `write` did.
pub enum Sent {
    Sent(Response),
    DryRun,
}

impl Ctx {
    /// Sends a read. Reads are never gated, even under `--dry-run`.
    pub async fn read(&self, req: Request) -> Result<Value> {
        Ok(self.client.get_json(req).await?)
    }

    /// Sends a request through the confirmation gate. `note` says in plain words what
    /// happens; Live and Destructive routes get a default when it is absent.
    pub async fn write(&self, req: Request, note: Option<&str>) -> Result<Sent> {
        let route = req.route()?;
        if route.effect == Effect::Read {
            return Ok(Sent::Sent(self.client.send(&req).await?));
        }
        if self.dry_run {
            let plan = json!({
                "dry_run": true,
                "method": req.method.as_str(),
                "path": req.path,
                "query": req.query.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                "effect": route.effect.as_str(),
                "body": req.body,
            });
            println!("{}", serde_json::to_string_pretty(&plan)?);
            return Ok(Sent::DryRun);
        }
        if !self.yes {
            confirm(&req, route.effect, note)?;
        }
        Ok(Sent::Sent(self.client.send(&req).await?))
    }
}

fn default_note(effect: Effect) -> Option<&'static str> {
    match effect {
        Effect::Live => Some("This changes what real customers experience right away."),
        Effect::Destructive => Some("This cannot be undone."),
        Effect::Read | Effect::Write => None,
    }
}

fn confirm(req: &Request, effect: Effect, note: Option<&str>) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(Error::Refused(format!(
            "{} {} is a {} call and needs confirmation; rerun with --yes to send it or --dry-run to preview it",
            req.method,
            req.path,
            effect.as_str()
        ))
        .into());
    }
    eprintln!("{} {}  [{}]", req.method, req.path, effect.as_str());
    if let Some(note) = note.or(default_note(effect)) {
        eprintln!("{note}");
    }
    if let Some(body) = &req.body {
        let pretty = serde_json::to_string_pretty(body)?;
        let total = pretty.lines().count();
        for line in pretty.lines().take(PROMPT_BODY_LINES) {
            eprintln!("  {line}");
        }
        if total > PROMPT_BODY_LINES {
            eprintln!("  … {} more lines", total - PROMPT_BODY_LINES);
        }
    }
    let go = dialoguer::Confirm::new()
        .with_prompt("Send it?")
        .default(false)
        .interact()?;
    if go {
        Ok(())
    } else {
        Err(Error::Refused("not confirmed; nothing was sent".to_owned()).into())
    }
}

/// Refuses early, before a download is spent, when the target already exists.
pub fn refuse_existing(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        return Err(overwrite_refusal(path));
    }
    Ok(())
}

fn overwrite_refusal(path: &std::path::Path) -> anyhow::Error {
    Error::Refused(format!(
        "{} already exists; refusing to overwrite it",
        path.display()
    ))
    .into()
}

/// Writes `bytes` to a new file; refuses to overwrite.
pub fn write_new_file(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(overwrite_refusal(path));
        }
        Err(e) => return Err(e.into()),
    };
    file.write_all(bytes)?;
    Ok(())
}
