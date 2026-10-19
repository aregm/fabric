//! The `fabric-demo calendar` command end to end. It keeps its store key in Windows Credential
//! Manager and its calendar encrypted on disk, across restarts. It writes to Credential Manager
//! under the test prefix `fabric-demo-test/` and deletes the entry afterwards, so it is local
//! only:
//!
//!   .\scripts\run.ps1 cargo test -p fabric-demo --test calendar_command -- --ignored

#![cfg(windows)]

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use fabric_demo::calendar::CalendarClient;
use fabric_demo::clock::{Clock, ClockConfig, DemoClock, wall_now_ms};
use fabric_policy::time::parse_utc;
use fabric_policy::{Action, ActionVerdict, CheckKind, CheckReport, Verdict};
use fabric_token::{
    BindingKey, CredentialStore, KeyDirectory, MemoryStore, PersonKeys, SecretStore, hex, random,
};
use serde_json::json;

const PREFIX: &str = "fabric-demo-test";

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

fn permit(action: &Action) -> CheckReport {
    CheckReport {
        kind: CheckKind::Authorize,
        theory: "Check_000001".to_owned(),
        proof_id: "ab".repeat(32),
        policy_sha256: "cd".repeat(32),
        isabelle: "Isabelle2025-2".to_owned(),
        seconds: 1.0,
        verdicts: vec![ActionVerdict {
            action: action.clone(),
            verdict: Verdict::Permit,
            rules: Vec::new(),
        }],
        theory_text: String::new(),
    }
}

/// A running service, killed when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts the command and returns it with the URL it printed.
fn start(config: &Path) -> (Running, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fabric-demo"))
        .args(["calendar", "--config"])
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the command starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.is_empty() {
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        panic!("the command exited: {stderr}");
    }
    let url = line
        .trim()
        .strip_prefix("calendar:alice listening on ")
        .unwrap_or_else(|| panic!("unexpected first line {line:?}"))
        .to_owned();
    (Running(child), url)
}

/// Deletes the test's store key even when an assertion fails.
struct Cleanup(CredentialStore);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.0.delete("alice/store");
    }
}

#[test]
#[ignore = "writes to Windows Credential Manager and runs the command; run with --ignored"]
fn the_calendar_command_keeps_its_store_key_in_credential_manager() {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("calendar-command");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let cleanup = Cleanup(CredentialStore::open(PREFIX).unwrap());
    cleanup.0.delete("alice/store").unwrap();

    // Alice's gate key, published the way the launcher will publish it.
    let alice = PersonKeys::load_or_create(&MemoryStore::new(), "alice").unwrap();
    let mut keys = KeyDirectory::new();
    keys.insert(&alice);
    keys.write(&directory.join("keys.json")).unwrap();

    let clock = ClockConfig {
        start: "2027-01-11T08:00:00Z".to_owned(),
        wall_start_ms: wall_now_ms(),
    };
    let demo = DemoClock::from_config(&clock).unwrap();
    let owner_secret = hex::encode(&random::<32>().unwrap());
    let store = directory.join("data").join("alice.cal");
    let config = json!({
        "person": "alice",
        "peer_name": "Bob",
        "listen": "127.0.0.1:0",
        "store": store,
        "fixtures": Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/fixtures/alice.json"),
        "keys": directory.join("keys.json"),
        "owner_secret": owner_secret,
        "clock": clock,
        "audit_log": directory.join("audit.jsonl"),
        "credential_prefix": PREFIX,
    });
    let config_path = directory.join("calendar.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let agent = BindingKey::generate().unwrap();
    let minter = alice.minter();
    let mint = |action: &Action| {
        minter
            .mint(&permit(action), &agent.verifying_key(), demo.now_ms())
            .unwrap()
            .bytes
    };

    let (running, url) = start(&config_path);
    let client = CalendarClient::new(&url);
    runtime.block_on(async {
        let window = Action::ReadFreeBusy {
            start_ms: at("2027-01-14T13:00Z"),
            end_ms: at("2027-01-14T17:00Z"),
        };
        let busy = client
            .free_busy(&agent, &mint(&window), &window, demo.now_ms())
            .await
            .unwrap();
        eprintln!("free/busy on Thursday afternoon: {busy:?}");
        assert_eq!(busy.len(), 2, "busy until 14:30 and from 15:00");

        let meeting = Action::AddMeeting {
            negotiation: "n1".to_owned(),
            start_ms: at("2027-01-14T14:30Z"),
            length_ms: 30 * 60 * 1_000,
        };
        client
            .add_meeting(&agent, &mint(&meeting), &meeting, demo.now_ms())
            .await
            .unwrap();
        let busy = client
            .free_busy(&agent, &mint(&window), &window, demo.now_ms())
            .await
            .unwrap();
        assert_eq!(busy.len(), 1, "the meeting fills the gap");
    });
    drop(running);
    assert!(
        cleanup.0.get("alice/store").unwrap().is_some(),
        "the store key is in Credential Manager"
    );

    // A restart decrypts the store with the key from Credential Manager.
    let (running, url) = start(&config_path);
    let events = runtime
        .block_on(CalendarClient::new(&url).owner_events(&owner_secret))
        .unwrap();
    drop(running);
    assert!(
        events
            .iter()
            .any(|event| event.title == "Meeting with Bob (Fabric)")
    );

    let bytes = fs::read(&store).unwrap();
    for text in ["Meeting with Bob", "Board meeting prep"] {
        assert!(
            !bytes
                .windows(text.len())
                .any(|window| window == text.as_bytes()),
            "{text} is readable on disk"
        );
    }
    drop(cleanup);
    assert!(
        CredentialStore::open(PREFIX)
            .unwrap()
            .get("alice/store")
            .unwrap()
            .is_none()
    );
}
