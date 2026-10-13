use std::io::{BufRead, IsTerminal};

use anyhow::{Context, Result, bail};
use ingfah_client::{ApiKey, Client, Request};
use serde_json::json;

use crate::cli::{AuthCommand, Cli, Format};
use crate::config::{self, ConfigFile};
use crate::output::print_json;

/// The cheapest read every client key with `ai_agents:read` can make.
fn verify_request() -> Request {
    Request::get("/client/ai-agents").query("per_page", "1")
}

async fn verify(client: &Client) -> Result<()> {
    client.get_json(verify_request()).await?;
    Ok(())
}

pub async fn run(cli: &Cli, cmd: &AuthCommand) -> Result<()> {
    match cmd {
        AuthCommand::Login { with_key_stdin } => login(cli, *with_key_stdin).await,
        AuthCommand::Status => status(cli).await,
        AuthCommand::Logout => {
            if config::keyring_delete(&cli.profile)? {
                println!("Removed the stored key for profile `{}`.", cli.profile);
            } else {
                println!("No stored key for profile `{}`.", cli.profile);
            }
            Ok(())
        }
    }
}

fn read_key(with_key_stdin: bool) -> Result<String> {
    if with_key_stdin {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        return Ok(line);
    }
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to prompt on; pipe the key with --with-key-stdin");
    }
    Ok(dialoguer::Password::new()
        .with_prompt("Ingfah API key")
        .interact()?)
}

async fn login(cli: &Cli, with_key_stdin: bool) -> Result<()> {
    let key = ApiKey::new(&read_key(with_key_stdin)?);
    if !key.has_valid_format() {
        bail!(
            "that is not a client API key: expected `sk-ing-v1-` and 64 hex characters, or a legacy `ing_sk_` key"
        );
    }
    let client = crate::build_client(cli, key.clone())?;
    verify(&client)
        .await
        .context("the key did not work; nothing was stored")?;
    config::keyring_set(&cli.profile, &key)?;
    if let Some(base) = &cli.base_url {
        let mut file = ConfigFile::load()?;
        file.profiles
            .entry(cli.profile.clone())
            .or_default()
            .base_url = Some(ingfah_client::client::normalize_base_url(base)?);
        let path = file.save()?;
        println!(
            "Saved base URL for profile `{}` in {}.",
            cli.profile,
            path.display()
        );
    }
    println!(
        "Stored key {} for profile `{}` in the system keyring.",
        key.masked(),
        cli.profile
    );
    Ok(())
}

async fn status(cli: &Cli) -> Result<()> {
    let (key, source) = crate::resolve_key(cli)?;
    let client = crate::build_client(cli, key.clone())?;
    let check = verify(&client).await;
    let check_text = match &check {
        Ok(()) => "ok".to_owned(),
        Err(e) => format!("failed: {e:#}"),
    };
    if cli.format == Some(Format::Json) {
        print_json(&json!({
            "profile": cli.profile,
            "key_source": source.describe(&cli.profile),
            "key": key.masked(),
            "base_url": client.base_url(),
            "check": check_text,
        }))?;
    } else {
        println!("Profile:  {}", cli.profile);
        println!("Key from: {}", source.describe(&cli.profile));
        println!("Key:      {}", key.masked());
        println!("Base URL: {}", client.base_url());
        println!("Check:    GET /client/ai-agents {check_text}");
    }
    check
}
