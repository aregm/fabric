//! The launcher (section 5.7, task T8.3).
//!
//! `fabric-demo up` creates both people's keys on its first run, writes the per-run
//! configuration to `.local/demo/run/`, starts the shared Isabelle server, then the relay, both
//! calendar services, both trusted hosts, the dashboard and both agents, and prints the
//! dashboard URL. It runs until Ctrl+C or `fabric-demo down`, then stops everything.

use std::error::Error;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use fabric_policy::GateConfig;
use fabric_policy::server::{Server, ServerOptions};
use fabric_policy::time::{format_utc, parse_utc};
use fabric_token::{KeyDirectory, PersonKeys, SecretStore, hex, random, sha256};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::clock::wall_now_ms;
use crate::relay::new_mailbox;

/// Where the demo's Isabelle server is registered.
const ISABELLE_SERVER: &str = "fabric-demo";
const START_TIMEOUT: Duration = Duration::from_secs(180);
pub const DEFAULT_START: &str = "2027-01-11T08:00:00Z";
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// What `up` needs to know.
pub struct UpOptions {
    pub repository: PathBuf,
    /// The planners of Alice and Bob: `claude`, `ollama:<model>` or `scripted`.
    pub backends: [String; 2],
    /// The demo clock's start (D22).
    pub start: String,
}

/// What `down` reads to reach a running launcher.
#[derive(Debug, Deserialize, Serialize)]
struct RunFile {
    control: String,
    control_key: String,
    dashboard: String,
}

fn run_dir(repository: &Path) -> PathBuf {
    repository.join(".local").join("demo").join("run")
}

fn secret() -> Result<String, Box<dyn Error>> {
    Ok(hex::encode(&random::<32>()?))
}

/// A free loopback port. Another process could take it before the service binds it; on this
/// one machine that is unlikely, and the service would then fail to start and say so.
fn free_port() -> Result<u16, Box<dyn Error>> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn backend(text: &str) -> Result<Value, Box<dyn Error>> {
    Ok(match text {
        "claude" => json!({ "kind": "claude" }),
        "scripted" => json!({ "kind": "scripted" }),
        _ => match text.strip_prefix("ollama:") {
            Some(model) if !model.is_empty() => json!({ "kind": "ollama", "model": model }),
            _ => {
                return Err(format!(
                    "unknown backend {text:?}; use claude, ollama:<model> or scripted"
                )
                .into());
            }
        },
    })
}

fn write(path: &Path, value: &Value) -> Result<(), Box<dyn Error>> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

/// A child process of the launcher.
struct Running {
    name: String,
    child: Child,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `fabric-demo <command> --config <config>`, waits for its first line, and copies the
/// rest of its output to `<run>/<name>.log`.
fn start(
    exe: &Path,
    run: &Path,
    name: &str,
    command: &str,
    config: &Path,
    repository: &Path,
) -> Result<(Running, String), Box<dyn Error>> {
    let log = run.join(format!("{name}.log"));
    let mut child = Command::new(exe)
        .args([command, "--config"])
        .arg(config)
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(File::create(run.join(format!("{name}.err.log")))?)
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        let _ = sender.send(line);
        if let Ok(mut file) = File::create(log) {
            let _ = std::io::copy(&mut reader, &mut file);
        }
    });
    let running = Running {
        name: name.to_owned(),
        child,
    };
    match receiver.recv_timeout(START_TIMEOUT) {
        Ok(line) if !line.trim().is_empty() => Ok((running, line.trim().to_owned())),
        _ => {
            let errors =
                fs::read_to_string(run.join(format!("{name}.err.log"))).unwrap_or_default();
            Err(format!("{name} did not start: {}", errors.trim()).into())
        }
    }
}

fn url_of(line: &str) -> Result<String, Box<dyn Error>> {
    line.rsplit_once(" listening on ")
        .map(|(_, url)| url.to_owned())
        .ok_or_else(|| format!("no address in {line:?}").into())
}

#[cfg(windows)]
fn secret_store() -> Result<Box<dyn SecretStore>, Box<dyn Error>> {
    Ok(Box::new(fabric_token::CredentialStore::open(
        fabric_token::CredentialStore::DEMO,
    )?))
}

#[cfg(not(windows))]
fn secret_store() -> Result<Box<dyn SecretStore>, Box<dyn Error>> {
    Err("the demo runs on Windows only: its keys live in Windows Credential Manager".into())
}

/// Runs the whole demo until Ctrl+C or `fabric-demo down`.
pub fn up(options: &UpOptions) -> Result<(), Box<dyn Error>> {
    // Not `canonicalize`: on Windows it adds the `\\?\` prefix, which Isabelle's Cygwin paths refuse.
    let repository = std::path::absolute(&options.repository)?;
    for required in ["isabelle/Fabric_Policy/ROOT", "demo/fixtures/alice.json"] {
        if !repository.join(required).exists() {
            return Err(format!(
                "{} is not the repository root: {required} is missing",
                repository.display()
            )
            .into());
        }
    }
    let run = run_dir(&repository);
    if run.join("run.json").exists() {
        return Err(format!(
            "a demo is already running; stop it with `fabric-demo down`, or delete {} if it is not",
            run.join("run.json").display()
        )
        .into());
    }
    let _ = fs::remove_dir_all(&run);
    fs::create_dir_all(&run)?;
    let people = ["alice", "bob"];
    let data = repository.join(".local").join("demo").join("data");
    fs::create_dir_all(&data)?;
    // Every run starts from the fixtures, so the scenario is the same each time (D22). The
    // store stays encrypted at rest for the run.
    for person in people {
        let _ = fs::remove_file(data.join(format!("{person}.cal")));
    }
    let exe = std::env::current_exe()?;

    // Keys: created on the first run, then loaded from Credential Manager.
    let secrets = secret_store()?;
    let mut keys = KeyDirectory::new();
    for person in people {
        keys.insert(&PersonKeys::load_or_create(secrets.as_ref(), person)?);
    }
    keys.write(&run.join("keys.json"))?;

    let start_ms = parse_utc(&options.start).ok_or("the clock start is not a UTC time")?;
    let clock = json!({ "start": options.start, "wall_start_ms": wall_now_ms() });
    let mailboxes = [new_mailbox()?, new_mailbox()?];
    let owner_secrets = [secret()?, secret()?];
    let agent_secrets = [secret()?, secret()?];
    let dashboard_secret = secret()?;
    let control_key = secret()?;

    println!("Starting Isabelle (the first run builds the policy session)...");
    let isabelle = GateConfig::for_repository(&repository);
    let mut server = Server::start(&ServerOptions {
        isabelle_home: &isabelle.isabelle_home,
        user_home: &isabelle.user_home,
        name: ISABELLE_SERVER,
        log_dir: &run,
    })?;
    let result = (|| -> Result<Vec<Running>, Box<dyn Error>> {
        let mut running = Vec::new();
        let relay_config = run.join("relay.json");
        write(
            &relay_config,
            &json!({
                "listen": format!("127.0.0.1:{}", free_port()?),
                "mailboxes": mailboxes,
                "clock": clock,
                "audit_log": run.join("relay.jsonl"),
            }),
        )?;
        let (relay, line) = start(&exe, &run, "relay", "relay", &relay_config, &repository)?;
        let relay_url = url_of(&line)?;
        running.push(relay);

        let displays = ["Alice", "Bob"];
        let mut calendars = Vec::new();
        let mut hosts = Vec::new();
        for (index, person) in people.into_iter().enumerate() {
            let calendar_config = run.join(format!("{person}-calendar.json"));
            write(
                &calendar_config,
                &json!({
                    "person": person,
                    "peer_name": displays[1 - index],
                    "listen": format!("127.0.0.1:{}", free_port()?),
                    "store": data.join(format!("{person}.cal")),
                    "fixtures": repository.join("demo").join("fixtures").join(format!("{person}.json")),
                    "keys": run.join("keys.json"),
                    "owner_secret": owner_secrets[index],
                    "clock": clock,
                    "audit_log": run.join(format!("{person}-calendar.jsonl")),
                }),
            )?;
            let name = format!("{person}-calendar");
            let (calendar, line) =
                start(&exe, &run, &name, "calendar", &calendar_config, &repository)?;
            calendars.push(url_of(&line)?);
            running.push(calendar);
        }
        for (index, person) in people.into_iter().enumerate() {
            let host_config = run.join(format!("{person}-host.json"));
            write(
                &host_config,
                &json!({
                    "person": person,
                    "peer": people[1 - index],
                    "role": if index == 0 { "coordinator" } else { "invitee" },
                    "listen": format!("127.0.0.1:{}", free_port()?),
                    "keys": run.join("keys.json"),
                    "relay": relay_url,
                    "inbox": mailboxes[index],
                    "peer_inbox": mailboxes[1 - index],
                    "agent_secret": agent_secrets[index],
                    "owner_secret": owner_secrets[index],
                    "policy": {
                        "repository": repository,
                        "isabelle": server.address(),
                        "work_dir": run.join(format!("{person}-checks")),
                    },
                    "clock": clock,
                    "audit_log": run.join(format!("{person}-host.jsonl")),
                }),
            )?;
            println!("Starting {person}'s host (it opens a policy session on Isabelle)...");
            let name = format!("{person}-host");
            let (host, line) = start(&exe, &run, &name, "host", &host_config, &repository)?;
            hosts.push(url_of(&line)?);
            running.push(host);
        }

        let window_end = format_utc(start_ms + 7 * DAY_MS);
        let expires = format_utc(start_ms + DAY_MS);
        let preferences = ["Weekday afternoons work best.", "Any weekday works."];
        let dashboard_people: Vec<Value> = people
            .iter()
            .enumerate()
            .map(|(index, person)| {
                json!({
                    "name": person,
                    "display": displays[index],
                    "host": hosts[index],
                    "calendar": calendars[index],
                    "owner_secret": owner_secrets[index],
                    "request": {
                        "peer": people[1 - index],
                        "duration_minutes": 30,
                        "window_start": format_utc(start_ms),
                        "window_end": window_end,
                        "expires_at": expires,
                        "preference": preferences[index],
                    },
                    "mailbox": mailboxes[index],
                    "backend": options.backends[index],
                })
            })
            .collect();
        let mut logs = vec![json!({ "name": "relay", "path": run.join("relay.jsonl") })];
        for person in people {
            for part in ["calendar", "host", "agent"] {
                logs.push(json!({
                    "name": format!("{person}-{part}"),
                    "path": run.join(format!("{person}-{part}.jsonl")),
                }));
            }
        }
        let dashboard_config = run.join("dashboard.json");
        write(
            &dashboard_config,
            &json!({
                "listen": format!("127.0.0.1:{}", free_port()?),
                "secret": dashboard_secret,
                "people": dashboard_people,
                "relay": relay_url,
                "logs": logs,
                "checks": people.map(|person| run.join(format!("{person}-checks")).join("checks.jsonl")),
                "clock": clock,
            }),
        )?;
        let (dashboard, line) = start(
            &exe,
            &run,
            "dashboard",
            "dashboard",
            &dashboard_config,
            &repository,
        )?;
        let dashboard_url = url_of(&line)?;
        running.push(dashboard);

        for (index, person) in people.into_iter().enumerate() {
            let agent_config = run.join(format!("{person}-agent.json"));
            write(
                &agent_config,
                &json!({
                    "person": person,
                    "host": hosts[index],
                    "calendar": calendars[index],
                    "agent_secret": agent_secrets[index],
                    "backend": backend(&options.backends[index])?,
                    "clock": clock,
                    "audit_log": run.join(format!("{person}-agent.jsonl")),
                    "env_file": repository.join(".env"),
                }),
            )?;
            let name = format!("{person}-agent");
            let (agent, line) = start(&exe, &run, &name, "agent", &agent_config, &repository)?;
            println!("{line}");
            running.push(agent);
        }
        println!();
        println!("Dashboard: {dashboard_url}/#key={dashboard_secret}");
        println!("Stop with Ctrl+C, or run `fabric-demo down`.");
        write(
            &run.join("run.json"),
            &json!(RunFile {
                control: String::new(),
                control_key: control_key.clone(),
                dashboard: dashboard_url,
            }),
        )?;
        Ok(running)
    })();

    let outcome = match result {
        Ok(running) => {
            let waited = wait_for_stop(&run, &control_key);
            stop_all(running);
            waited
        }
        Err(error) => Err(error),
    };
    server.shutdown();
    let _ = fs::remove_file(run.join("run.json"));
    println!("The demo has stopped.");
    outcome
}

/// Serves the control endpoint and waits for Ctrl+C or `down`.
fn wait_for_stop(run: &Path, control_key: &str) -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let stop = Arc::new(Notify::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address: SocketAddr = listener.local_addr()?;
        let run_file = run.join("run.json");
        let mut file: RunFile = serde_json::from_slice(&fs::read(&run_file)?)?;
        file.control = format!("http://{address}");
        fs::write(&run_file, serde_json::to_vec_pretty(&file)?)?;
        let expected = sha256(control_key.as_bytes());
        let notify = stop.clone();
        let router = Router::new().route(
            "/v1/stop",
            post(move |headers: HeaderMap| {
                let notify = notify.clone();
                async move {
                    let given = headers
                        .get("x-control-key")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default();
                    if sha256(given.as_bytes()) == expected {
                        notify.notify_one();
                        StatusCode::OK
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }
            }),
        );
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        tokio::select! {
            () = stop.notified() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        server.abort();
        Ok::<(), Box<dyn Error>>(())
    })
}

/// Stops the agents first, so nothing acts while the services go away.
fn stop_all(mut running: Vec<Running>) {
    while let Some(child) = running.pop() {
        println!("Stopping {}...", child.name);
        drop(child);
    }
}

/// Deletes both people's keys and store keys from Credential Manager. The next `up` makes new
/// ones; calendars encrypted with the old store keys can then no longer be opened, so their
/// files are deleted too.
pub fn reset_keys(repository: &Path) -> Result<(), Box<dyn Error>> {
    if run_dir(repository).join("run.json").exists() {
        return Err("stop the demo first with `fabric-demo down`".into());
    }
    let secrets = secret_store()?;
    for person in ["alice", "bob"] {
        PersonKeys::delete(secrets.as_ref(), person)?;
        secrets.delete(&format!("{person}/store"))?;
        let _ = fs::remove_file(
            repository
                .join(".local")
                .join("demo")
                .join("data")
                .join(format!("{person}.cal")),
        );
    }
    println!("Deleted the demo's keys from Credential Manager.");
    Ok(())
}

/// Asks a running launcher to stop, and waits until it has.
pub fn down(repository: &Path) -> Result<(), Box<dyn Error>> {
    let run_file = run_dir(repository).join("run.json");
    let text = fs::read(&run_file).map_err(|_| "no demo is running")?;
    let file: RunFile = serde_json::from_slice(&text)?;
    if file.control.is_empty() {
        return Err("the demo is still starting; try again in a moment".into());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/stop", file.control))
            .header("x-control-key", &file.control_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!("the launcher refused: {}", response.status()).into());
        }
        Ok::<(), Box<dyn Error>>(())
    })?;
    for _ in 0..120 {
        if !run_file.exists() {
            println!("The demo has stopped.");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err("the launcher did not stop within a minute".into())
}
