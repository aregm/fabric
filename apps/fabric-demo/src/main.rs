use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use fabric_demo::calendar::{Calendar, CalendarConfig, bind, serve};
use fabric_token::SecretStore;

const USAGE: &str = "usage: fabric-demo calendar --config <file>";

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
        ["calendar", "--config", path] => calendar(Path::new(path)),
        [] | ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("unexpected arguments {arguments:?}\n{USAGE}").into()),
    }
}

/// Runs one person's calendar service until Ctrl+C.
fn calendar(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = CalendarConfig::read(path)?;
    let secrets = secret_store(&config.credential_prefix)?;
    let calendar = Calendar::from_config(&config, secrets.as_ref())?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let listener = bind(config.listen).await?;
        println!(
            "{} listening on http://{}",
            calendar.component(),
            listener.local_addr()?
        );
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        serve(listener, calendar, shutdown).await?;
        Ok::<(), Box<dyn Error>>(())
    })
}

#[cfg(windows)]
fn secret_store(prefix: &str) -> Result<Box<dyn SecretStore>, Box<dyn Error>> {
    Ok(Box::new(fabric_token::CredentialStore::open(prefix)?))
}

#[cfg(not(windows))]
fn secret_store(_prefix: &str) -> Result<Box<dyn SecretStore>, Box<dyn Error>> {
    Err("the demo runs on Windows only: its keys live in Windows Credential Manager".into())
}
