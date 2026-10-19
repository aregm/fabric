use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use fabric_demo::agent::{self, AgentConfig, BackendConfig};
use fabric_demo::calendar::{self, Calendar, CalendarConfig};
use fabric_demo::host::{self, Host, HostConfig};
use fabric_demo::net::bind_loopback;
use fabric_demo::relay::{self, Relay, RelayConfig};
use fabric_token::SecretStore;

const USAGE: &str = "usage: fabric-demo <calendar | relay | host | agent> --config <file>";
/// How long an agent waits for an outside event before checking again.
const AGENT_WAIT_MS: u64 = 20_000;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fabric-demo: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["calendar", "--config", path] => run_calendar(Path::new(path)),
        ["relay", "--config", path] => run_relay(Path::new(path)),
        ["host", "--config", path] => run_host(Path::new(path)),
        ["agent", "--config", path] => run_agent(Path::new(path)),
        [] | ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("unexpected arguments {arguments:?}\n{USAGE}").into()),
    }
}

async fn ctrl_c() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Runs one person's calendar service until Ctrl+C.
fn run_calendar(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = CalendarConfig::read(path)?;
    let secrets = secret_store(&config.credential_prefix)?;
    let calendar = Calendar::from_config(&config, secrets.as_ref())?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = bind_loopback(config.listen).await?;
        println!(
            "{} listening on http://{}",
            calendar.component(),
            listener.local_addr()?
        );
        calendar::serve(listener, calendar, ctrl_c()).await?;
        Ok::<(), Box<dyn Error>>(())
    })
}

/// Runs the relay until Ctrl+C.
fn run_relay(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = RelayConfig::read(path)?;
    let relay = Relay::from_config(&config)?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = bind_loopback(config.listen).await?;
        println!("relay listening on http://{}", listener.local_addr()?);
        relay::serve(listener, relay, ctrl_c()).await?;
        Ok::<(), Box<dyn Error>>(())
    })
}

/// Runs one person's trusted host until Ctrl+C.
fn run_host(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = HostConfig::read(path)?;
    let secrets = secret_store(&config.credential_prefix)?;
    let host = Host::from_config(&config, secrets.as_ref())?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = bind_loopback(config.listen).await?;
        println!(
            "host:{} listening on http://{}",
            config.person,
            listener.local_addr()?
        );
        tokio::spawn(host.gateway().clone().run_inbound());
        host::serve(listener, host.clone(), ctrl_c()).await?;
        host.stop();
        Ok::<(), Box<dyn Error>>(())
    })
}

/// Runs one person's agent until Ctrl+C.
fn run_agent(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = AgentConfig::read(path)?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let agent = agent::start(&config).await?;
        println!(
            "agent:{} running with the {} planner",
            config.person,
            agent_backend(&config)
        );
        tokio::select! {
            () = agent.run(AGENT_WAIT_MS) => {}
            () = ctrl_c() => {}
        }
        Ok::<(), Box<dyn Error>>(())
    })
}

fn agent_backend(config: &AgentConfig) -> String {
    match &config.backend {
        BackendConfig::Claude => "claude".to_owned(),
        BackendConfig::Ollama { model } => format!("ollama:{model}"),
        BackendConfig::Scripted => "scripted".to_owned(),
    }
}

#[cfg(windows)]
fn secret_store(prefix: &str) -> Result<Box<dyn SecretStore>, Box<dyn Error>> {
    Ok(Box::new(fabric_token::CredentialStore::open(prefix)?))
}

#[cfg(not(windows))]
fn secret_store(_prefix: &str) -> Result<Box<dyn SecretStore>, Box<dyn Error>> {
    Err("the demo runs on Windows only: its keys live in Windows Credential Manager".into())
}
