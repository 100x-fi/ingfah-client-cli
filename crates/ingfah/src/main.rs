mod cli;
mod commands;
mod config;
mod ctx;
mod output;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use ingfah_client::{ApiKey, Client, ClientConfig, DEFAULT_BASE_URL};

use crate::cli::{Cli, Command, Format};
use crate::config::{ConfigFile, KeySource};
use crate::ctx::Ctx;

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 3;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match run(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(report(&e)),
    }
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "ingfah=debug,ingfah_client=debug",
        _ => "ingfah=trace,ingfah_client=trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(level))
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();
}

fn report(e: &anyhow::Error) -> u8 {
    let Some(client_err) = e.downcast_ref::<ingfah_client::Error>() else {
        eprintln!("error: {e:#}");
        return EXIT_ERROR;
    };
    if let ingfah_client::Error::Api(api) = client_err {
        eprintln!("error: {api}");
        if let Some(hint) = &api.hint {
            eprintln!("hint: {hint}");
        }
        for (label, extra) in [("errors", &api.errors), ("data", &api.data)] {
            if let Some(v) = extra {
                let pretty = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
                eprintln!("{label}: {pretty}");
            }
        }
    } else {
        eprintln!("error: {e:#}");
    }
    if client_err.is_refusal() {
        EXIT_REFUSED
    } else {
        EXIT_ERROR
    }
}

async fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::Completion { shell } => {
            clap_complete::generate(
                *shell,
                &mut Cli::command(),
                "ingfah",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Command::Auth(cmd) => commands::auth::run(cli, cmd).await,
        command => {
            let ctx = build_ctx(cli)?;
            commands::dispatch(&ctx, command).await
        }
    }
}

/// `--base-url` / `INGFAH_API_BASE_URL`, then the profile's base_url, then production.
pub fn base_url(cli: &Cli, config: &ConfigFile) -> String {
    cli.base_url
        .clone()
        .or_else(|| config.base_url(&cli.profile).map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned())
}

pub fn resolve_key(cli: &Cli) -> Result<(ApiKey, KeySource)> {
    let env = std::env::var("INGFAH_API_KEY").ok();
    config::resolve_key(cli.api_key.as_deref(), env.as_deref(), || {
        config::keyring_get(&cli.profile)
    })?
    .with_context(|| {
        format!(
            "no API key for profile `{}`: run `ingfah auth login`, set INGFAH_API_KEY, or pass --api-key",
            cli.profile
        )
    })
}

pub fn build_client(cli: &Cli, key: ApiKey) -> Result<Client> {
    let config = ConfigFile::load()?;
    let mut client_config = ClientConfig::new(base_url(cli, &config), key);
    client_config.timeout = Duration::from_secs(cli.timeout);
    Ok(Client::new(client_config)?)
}

fn build_ctx(cli: &Cli) -> Result<Ctx> {
    let (key, _) = resolve_key(cli)?;
    let format = cli.format.unwrap_or(if std::io::stdout().is_terminal() {
        Format::Table
    } else {
        Format::Json
    });
    Ok(Ctx {
        client: build_client(cli, key)?,
        format,
        yes: cli.yes,
        dry_run: cli.dry_run,
    })
}
