//! Process tests of the built `fabric-broker` executable, with plain `std::process` (design 9.4).
//!
//! The broker under test is `FABRIC_BROKER_UNDER_TEST` when it is set (an absolute path, for
//! example the broker inside a packaged app) and otherwise the binary Cargo built. Every line
//! the broker writes must decode as an owner response or an owner notice; the helper fails the
//! test otherwise.
//!
//! Every start removes each inherited variable whose name starts with `FABRIC_BROKER_`, then
//! names its own new data directory under the system's temporary directory and the public test
//! key. No broker here opens a store with the Keychain: the only starts without
//! `FABRIC_BROKER_KEY_SOURCE=test` are the configuration refusals, whose key source is unset,
//! unknown, or `keychain` on a platform that has no Keychain.
//!
//! Bare numbers and the test ids p01 to p08 cite the CD-W04 design
//! (`docs/plans/calendar-desktop-w04-design.md`); "PW" numbers, such as PW p40 to PW p48,
//! cite the prover wiring's design (`docs/plans/calendar-desktop-prover-wiring-design.md`).

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use fabric_desktop_contracts::common::UndecidedReason;
use fabric_desktop_contracts::error::{ErrorCode, Fault};
use fabric_desktop_contracts::owner::{
    Approval, Change, Grant, GrantEndReason, GrantTerm, OwnerNotice, OwnerResponse, OwnerResult,
    ProverHealth, Request,
};
use fabric_desktop_contracts::{
    Entry, Frame, MAX_FRAME_BYTES, RequestState, canonical, decode, first_invalid, parse_strict,
};
use serde_json::{Value, json};

/// How long to wait for a frame that is due at once. The first answer of a broker also waits
/// for its launch, which for a newly signed executable on macOS can take seconds.
const PROMPT: Duration = Duration::from_secs(10);
/// How long to wait for a step of the synthetic agent, beyond its nominal time.
const STEP: Duration = Duration::from_secs(6);
/// The bound on stopping after end of input, for a broker that has already answered.
const STOP_BOUND: Duration = Duration::from_millis(1000);
/// How long a broker may take to exit when nothing showed that it had started yet.
const LAUNCH_AND_EXIT: Duration = Duration::from_secs(10);

/// The variables the broker reads.
const DATA_DIR: &str = "FABRIC_BROKER_DATA_DIR";
const KEY_SOURCE: &str = "FABRIC_BROKER_KEY_SOURCE";
const SYNTHETIC_AGENT: &str = "FABRIC_BROKER_SYNTHETIC_AGENT";
const ISABELLE_HOME: &str = "FABRIC_BROKER_ISABELLE_HOME";
const ISABELLE_USER_HOME: &str = "FABRIC_BROKER_ISABELLE_USER_HOME";
const RULES_DIR: &str = "FABRIC_BROKER_RULES_DIR";

/// The title of every event the synthetic agent proposes (`fabric_broker::synthetic::TITLE`;
/// the synthetic test checks they agree). It must never appear on stdout outside a full-detail
/// read, nor on stderr.
const SYNTHETIC_TITLE: &str = "Synthetic planning block (test build, marker 5f1c)";

/// Held across every spawn of a broker in this file, and released as soon as the spawn
/// returns. On macOS there is no `pipe2`, so `std` makes a child's pipes and marks them
/// close-on-exec in two steps; a broker spawned by another test thread in between inherits
/// the write end of this broker's stdin (or stdout), and this broker then sees end of input
/// only when that other broker exits.
static SPAWN: Mutex<()> = Mutex::new(());

/// Spawns `command` while no other test thread is spawning.
fn spawn(command: &mut Command) -> Child {
    let _spawning = SPAWN.lock().unwrap_or_else(PoisonError::into_inner);
    command.spawn().expect("the broker starts")
}

fn broker_path() -> PathBuf {
    match std::env::var_os("FABRIC_BROKER_UNDER_TEST") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env!("CARGO_BIN_EXE_fabric-broker")),
    }
}

fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/desktop/v1")
}

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// A fresh directory under the system's temporary directory, removed on drop. The broker's
/// data directory is its child `data`, which does not exist until the store creates it.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> TempDir {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fabric-broker-process-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a fresh temporary directory");
        TempDir(path)
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The broker with no inherited `FABRIC_BROKER_*` variable and none of its own.
fn bare_command() -> Command {
    let mut command = Command::new(broker_path());
    for (name, _) in std::env::vars_os() {
        // In any case: Windows compares variable names without regard to case, as the broker does.
        if name
            .as_encoded_bytes()
            .to_ascii_uppercase()
            .starts_with(b"FABRIC_BROKER_")
        {
            command.env_remove(name);
        }
    }
    command
}

/// The broker on `data_dir` with the test key, and the synthetic agent when asked.
fn command(data_dir: &Path, synthetic: bool) -> Command {
    let mut command = bare_command();
    command.env(DATA_DIR, data_dir).env(KEY_SOURCE, "test");
    if synthetic {
        command.env(SYNTHETIC_AGENT, "1");
    }
    command
}

/// One frame the broker wrote: its exact line and what it decodes to.
#[derive(Clone, Debug)]
struct Written {
    line: Vec<u8>,
    frame: Frame,
}

/// Decodes one line of stdout, with its line feed, or fails the test.
fn decode_line(line: &[u8]) -> Frame {
    let text = String::from_utf8_lossy(line);
    assert!(line.ends_with(b"\n"), "a line without a line feed: {text}");
    let bytes = &line[..line.len() - 1];
    let value = parse_strict(bytes).unwrap_or_else(|e| panic!("not strict JSON ({e}): {text}"));
    let entry = if value.get("sequence").is_some() {
        Entry::OwnerNotice
    } else {
        Entry::OwnerResponse
    };
    let frame = decode(entry, bytes).unwrap_or_else(|r| panic!("not a frame ({r}): {text}"));
    assert_eq!(
        frame.to_line().as_bytes(),
        line,
        "the line is not canonical"
    );
    assert_cross_member_rules(&frame, &text);
    frame
}

/// PW p48: the two rules between members of contract schema version 2 (README section 7),
/// on every owner line: a health names a problem only while the prover is unavailable, and
/// request details name an `undecided_reason` only while the request is `undecided`.
fn assert_cross_member_rules(frame: &Frame, text: &str) {
    let health = match frame {
        Frame::OwnerResponse(OwnerResponse::Result {
            result: OwnerResult::GetSnapshot(snapshot),
            ..
        }) => Some(&snapshot.health),
        Frame::OwnerNotice(OwnerNotice {
            change: Change::Health(health),
            ..
        }) => Some(health),
        Frame::OwnerResponse(OwnerResponse::Result {
            result: OwnerResult::GetRequestDetails(details),
            ..
        }) => {
            assert!(
                details.request.status.state() == RequestState::Undecided
                    || details.undecided_reason.is_none(),
                "a reason on a request that is not undecided: {text}"
            );
            None
        }
        _ => None,
    };
    if let Some(health) = health {
        assert!(
            health.prover == ProverHealth::Unavailable || health.prover_problem.is_none(),
            "a problem while the prover is not unavailable: {text}"
        );
    }
}

struct Broker {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Vec<u8>>,
    /// Every line read so far.
    seen: Vec<Vec<u8>>,
    /// Notices read while waiting for a response.
    notices: Vec<OwnerNotice>,
    /// stderr, when the test reads it.
    stderr: Option<thread::JoinHandle<String>>,
    /// The data directory, when the broker owns it; removed after the broker is gone.
    _dir: Option<TempDir>,
}

impl Broker {
    /// A broker on a new data directory of its own, with the test key.
    fn start() -> Broker {
        let dir = TempDir::new("broker");
        let mut broker = Broker::spawn_from(command(&dir.data(), false), false);
        broker._dir = Some(dir);
        broker
    }

    /// The same with the synthetic agent.
    fn synthetic() -> Broker {
        let dir = TempDir::new("synthetic");
        let mut broker = Broker::spawn_from(command(&dir.data(), true), false);
        broker._dir = Some(dir);
        broker
    }

    /// A broker on `data_dir`, which the test keeps.
    fn on(data_dir: &Path, synthetic: bool) -> Broker {
        Broker::spawn_from(command(data_dir, synthetic), false)
    }

    /// A broker on `data_dir` after the one before it was killed: first waits until the
    /// store's lock is free (see `wait_for_the_lock`).
    fn after_kill(data_dir: &Path, synthetic: bool) -> Broker {
        wait_for_the_lock(data_dir);
        Broker::on(data_dir, synthetic)
    }

    fn spawn_from(mut command: Command, read_stderr: bool) -> Broker {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if read_stderr {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = spawn(&mut command);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().map(|mut stderr| {
            thread::spawn(move || {
                let mut text = String::new();
                let _ = stderr.read_to_string(&mut text);
                text
            })
        });
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {
                        if sender.send(line).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Broker {
            child,
            stdin,
            lines,
            seen: Vec::new(),
            notices: Vec::new(),
            stderr,
            _dir: None,
        }
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        stdin.write_all(bytes).expect("the broker reads stdin");
        stdin.flush().expect("the broker reads stdin");
    }

    fn send(&mut self, value: &Value) {
        let mut line = canonical(value).into_bytes();
        line.push(b'\n');
        self.send_bytes(&line);
    }

    fn next(&mut self, timeout: Duration) -> Written {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => {
                self.seen.push(line.clone());
                Written {
                    frame: decode_line(&line),
                    line,
                }
            }
            Err(RecvTimeoutError::Timeout) => panic!("no frame within {timeout:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the broker closed stdout"),
        }
    }

    /// The next response; notices before it are kept in `self.notices`.
    fn response(&mut self) -> (OwnerResponse, Vec<u8>) {
        loop {
            let written = self.next(PROMPT);
            match written.frame {
                Frame::OwnerResponse(r) => return (r, written.line),
                Frame::OwnerNotice(n) => self.notices.push(n),
                other => panic!("{other:?}"),
            }
        }
    }

    fn ask(&mut self, value: &Value) -> OwnerResponse {
        self.send(value);
        self.response().0
    }

    fn result(&mut self, value: &Value) -> OwnerResult {
        match self.ask(value) {
            OwnerResponse::Result { result, .. } => result,
            OwnerResponse::Error { error, .. } => panic!("{error:?}"),
        }
    }

    fn snapshot(&mut self) -> Value {
        let OwnerResult::GetSnapshot(snapshot) =
            self.result(&request("snap", "get_snapshot", json!({})))
        else {
            panic!("not a snapshot")
        };
        serde_json::to_value(&snapshot).unwrap()
    }

    /// Waits for the first notice, kept or new, that `wanted` accepts.
    fn notice(&mut self, wanted: impl Fn(&OwnerNotice) -> bool) -> OwnerNotice {
        if let Some(n) = self.notices.iter().find(|n| wanted(n)) {
            return n.clone();
        }
        let deadline = Instant::now() + STEP;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next(left).frame {
                Frame::OwnerNotice(n) => {
                    self.notices.push(n.clone());
                    if wanted(&n) {
                        return n;
                    }
                }
                other => panic!("an unexpected frame: {other:?}"),
            }
        }
    }

    /// Waits for request `request_id` to reach `state` and returns it.
    fn request_in(&mut self, request_id: &str, state: RequestState) -> Request {
        let n = self.notice(|n| match &n.change {
            Change::Request(r) => r.request_id.as_str() == request_id && r.status.state() == state,
            _ => false,
        });
        match n.change {
            Change::Request(r) => r,
            _ => unreachable!(),
        }
    }

    /// Waits for a request other than `except` to reach `pending_approval`.
    fn pending(&mut self, except: &[&str]) -> Request {
        let n = self.notice(|n| match &n.change {
            Change::Request(r) => {
                r.status.state() == RequestState::PendingApproval
                    && !except.contains(&r.request_id.as_str())
            }
            _ => false,
        });
        match n.change {
            Change::Request(r) => r,
            _ => unreachable!(),
        }
    }

    /// Waits for a notice of grant `grant_id` that `wanted` accepts.
    fn grant(&mut self, grant_id: &str, wanted: impl Fn(&Grant) -> bool) -> Grant {
        let n = self.notice(|n| match &n.change {
            Change::Grant(g) => g.grant_id.as_str() == grant_id && wanted(g),
            _ => false,
        });
        match n.change {
            Change::Grant(g) => g,
            _ => unreachable!(),
        }
    }

    /// The highest notice sequence read so far.
    fn last_sequence(&self) -> u64 {
        self.notices
            .iter()
            .map(|n| n.sequence.get())
            .max()
            .unwrap_or(0)
    }

    /// Closes stdin and waits for the exit; the time it took and every frame still written.
    fn finish(mut self) -> (ExitStatus, Duration, Vec<Written>) {
        drop(self.stdin.take());
        let closed = Instant::now();
        let status = wait_with_bound(&mut self.child, LAUNCH_AND_EXIT);
        let took = closed.elapsed();
        let mut rest = Vec::new();
        while let Ok(line) = self.lines.recv_timeout(PROMPT) {
            self.seen.push(line.clone());
            rest.push(Written {
                frame: decode_line(&line),
                line,
            });
        }
        (status, took, rest)
    }

    /// Kills the broker with SIGKILL (TerminateProcess on Windows) and reaps it.
    fn kill(mut self) {
        self.child.kill().expect("the broker can be killed");
        let _ = self.child.wait();
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Waits, for up to ten seconds, while another process holds the store's lock in `data_dir`,
/// then releases it at once. Windows releases a killed process's file lock a short, unspecified
/// time after the process ends, so a broker started at once on the same directory could find
/// the store in use and answer `store_unavailable` there; elsewhere the lock is free at once.
/// Only the lock file is touched, never the database, so the next broker still does its own
/// recovery.
fn wait_for_the_lock(data_dir: &Path) {
    let path = data_dir.join(fabric_storage::LOCK_FILE);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("the lock file exists");
        match file.try_lock() {
            Ok(()) => {
                file.unlock().expect("the lock is released");
                return;
            }
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                panic!("the killed broker's lock on the store was still held after ten seconds")
            }
            Err(std::fs::TryLockError::Error(e)) => panic!("the lock file: {e}"),
        }
    }
}

fn wait_with_bound(child: &mut Child, bound: Duration) -> ExitStatus {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(status) = child.try_wait().expect("the child can be waited for") {
            return status;
        }
        assert!(Instant::now() < deadline, "the broker did not exit");
        thread::sleep(Duration::from_millis(5));
    }
}

fn request(id: &str, method: &str, params: Value) -> Value {
    json!({ "protocol": "fabric-desktop/1", "id": id, "method": method, "params": params })
}

fn hello(schema_version: u64) -> Value {
    request(
        "hello-1",
        "hello",
        json!({"client": {"name": "process test", "build": "0"}, "schema_version": schema_version}),
    )
}

fn subscribe(cursor: u64) -> Value {
    request("sub-1", "subscribe", json!({ "cursor": cursor }))
}

/// The code, id, method and detail of an error response.
fn error_of(response: &OwnerResponse) -> (ErrorCode, Option<String>, Option<String>, Value) {
    match response {
        OwnerResponse::Error { id, method, error } => (
            error.code(),
            id.as_ref().map(|i| i.as_str().to_owned()),
            method.as_ref().map(|m| m.as_str().to_owned()),
            error.fault.detail(),
        ),
        other => panic!("not an error: {other:?}"),
    }
}

fn decide(request: &Request, id_suffix: &str, decision: &str, duration: Value) -> Value {
    request_with(
        &format!("decide-{id_suffix}"),
        json!({
            "request_id": request.request_id,
            "request_digest": request.request_digest,
            "expected_revision": request.revision.get(),
            "decision": decision,
            "selected": null,
            "duration": duration,
        }),
    )
}

fn request_with(id: &str, params: Value) -> Value {
    request(id, "decide_request", params)
}

/// The states each request went through, in the order of the notices.
fn states_of(notices: &[OwnerNotice], request_id: &str) -> Vec<RequestState> {
    notices
        .iter()
        .filter_map(|n| match &n.change {
            Change::Request(r) if r.request_id.as_str() == request_id => Some(r.status.state()),
            _ => None,
        })
        .collect()
}

/// The sequences of `notices` rise by one from their first.
fn assert_consecutive(notices: &[OwnerNotice]) {
    let sequences: Vec<u64> = notices.iter().map(|n| n.sequence.get()).collect();
    let first = sequences.first().copied().unwrap_or(1);
    let expected: Vec<u64> = (first..first + sequences.len() as u64).collect();
    assert_eq!(sequences, expected, "the notices do not rise by one");
}

#[test]
fn hello_reports_the_limits_of_the_contract() {
    let mut broker = Broker::start();
    let OwnerResult::Hello(result) = broker.result(&hello(2)) else {
        panic!()
    };
    let text = std::fs::read_to_string(contract_dir().join("data/limits.json")).unwrap();
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        canonical(&serde_json::to_value(result.limits).unwrap()),
        canonical(&file["limits"])
    );
    assert_eq!(result.broker.name.as_str(), "fabric-broker");
    assert_eq!(
        result.broker.build.as_str(),
        format!("{} CD-W04", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(result.schema_version.get(), 2);
    assert!(result.capabilities.is_empty());
}

#[test]
fn hello_with_another_schema_version_is_refused() {
    let mut broker = Broker::start();
    let response = broker.ask(&hello(1));
    assert_eq!(
        error_of(&response),
        (
            ErrorCode::UnsupportedSchemaVersion,
            Some("hello-1".into()),
            Some("hello".into()),
            json!({"received": 1, "supported": 2})
        )
    );
}

#[test]
fn the_first_run_snapshot() {
    let mut broker = Broker::start();
    let value = broker.snapshot();
    assert_eq!(value["cursor"], 0);
    // PW p46: a run without a prover says why, as the contract's fixture does.
    assert_eq!(
        value["health"],
        json!({"prover": "unavailable", "prover_problem": "not_configured", "store": "ready"})
    );
    let fixture = contract_dir()
        .join("fixtures/messages/accept-owner-notice-health-prover-not-configured.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    assert_eq!(value["health"], fixture["value"]["change"]["health"]);
    assert_eq!(value["profiles"]["revision"], 0);
    assert_eq!(value["profiles"]["active_profile_id"], "careful");
    let ids: Vec<&Value> = value["profiles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["profile_id"])
        .collect();
    assert_eq!(ids, [&json!("careful"), &json!("flexible")]);
    for empty in ["accounts", "agents", "recipient_sets", "requests", "grants"] {
        assert_eq!(value[empty], json!([]), "{empty}");
    }
    assert_eq!(
        value["preferences"],
        json!({"revision": 0, "settings": {"zone": null, "workday": null, "protected_times": [],
            "uninterrupted": null, "buffer": null, "warning_calendars": []}})
    );
}

#[test]
fn subscribe_with_a_cursor_ahead_needs_a_snapshot() {
    let mut broker = Broker::start();
    let response = broker.ask(&request("sub-1", "subscribe", json!({"cursor": 3})));
    assert_eq!(
        error_of(&response),
        (
            ErrorCode::SnapshotRequired,
            Some("sub-1".into()),
            Some("subscribe".into()),
            Value::Null
        )
    );
}

/// The synthetic agent's first request up to its card, then an approval: the grant, the
/// approval, the duplicate command and the reused command id, and the end of the request in
/// `undecided`.
#[test]
fn an_approval_of_a_synthetic_request_creates_a_grant_and_ends_undecided() {
    let mut broker = Broker::synthetic();
    let OwnerResult::Subscribe(subscribed) = broker.result(&subscribe(0)) else {
        panic!()
    };
    assert_eq!(subscribed.cursor.get(), 0);
    let pending = broker.pending(&[]);
    let request_id = pending.request_id.as_str().to_owned();
    assert_consecutive(&broker.notices);
    assert_eq!(broker.notices[0].sequence.get(), 1);
    let kinds: Vec<&str> = broker.notices.iter().map(|n| n.change.kind()).collect();
    assert_eq!(
        kinds,
        [
            "agent", "account", "agent", "request", "request", "request", "request"
        ]
    );
    assert_eq!(
        states_of(&broker.notices, &request_id),
        [
            RequestState::Received,
            RequestState::Validating,
            RequestState::ProvingClassification,
            RequestState::PendingApproval
        ]
    );
    assert_eq!(pending.revision.get(), 4);

    let details = request(
        "details-1",
        "get_request_details",
        json!({"request_id": request_id, "redaction": "full"}),
    );
    let OwnerResult::GetRequestDetails(details) = broker.result(&details) else {
        panic!()
    };
    let card = details.card.expect("a pending request has a card");
    assert_eq!(card.insufficiency.rule.as_str(), "synthetic_no_rule_theory");
    let action = details.private.expect("full details").action;
    assert_eq!(action.tool().as_str(), "fabric_calendar_propose_create");

    let approve = decide(&pending, "1", "approve", json!({"kind": "one_request"}));
    broker.send(&approve);
    let (response, first_answer) = broker.response();
    let OwnerResponse::Result {
        result: OwnerResult::DecideRequest(decided),
        ..
    } = response
    else {
        panic!("{response:?}")
    };
    let Approval::Approve(approval) = &decided.approval else {
        panic!()
    };
    assert_eq!(approval.command_id.as_str(), "decide-1");
    let grant_id = approval.grant_id.as_str().to_owned();
    let grant = broker.grant(&grant_id, |g| g.ended.is_none());
    assert_eq!(
        grant.term,
        GrantTerm::OneRequest {
            action_digest: action.digest()
        }
    );
    let approved = broker.notice(|n| match &n.change {
        Change::Request(r) => {
            r.request_id.as_str() == request_id && r.status.state() == RequestState::Approved
        }
        _ => false,
    });
    let Change::Request(approved_request) = &approved.change else {
        unreachable!()
    };
    assert_eq!(&decided.request, approved_request);
    assert_eq!(decided.commit_sequence, approved.sequence);

    // The same command again: the same bytes.
    broker.send(&approve);
    let (_, again) = broker.response();
    assert_eq!(again, first_answer);
    // The same id with a different command.
    let mut other = approve.clone();
    other["params"]["duration"] = json!({"kind": "session"});
    let response = broker.ask(&other);
    assert_eq!(error_of(&response).0, ErrorCode::CommandReused);

    broker.request_in(&request_id, RequestState::Undecided);
    let states = states_of(&broker.notices, &request_id);
    assert_eq!(first_invalid(&states), None);
    assert_eq!(
        &states[4..],
        [
            RequestState::Approved,
            RequestState::ProvingFreshPermit,
            RequestState::Undecided
        ]
    );
    // PW p47: the classifier is not Isabelle, so its permit is never acted on, and the redacted
    // details say why. A synthetic proof is never listed.
    let details = request(
        "details-2",
        "get_request_details",
        json!({"request_id": request_id, "redaction": "redacted"}),
    );
    let OwnerResult::GetRequestDetails(details) = broker.result(&details) else {
        panic!()
    };
    assert_eq!(details.request.status.state(), RequestState::Undecided);
    assert_eq!(
        details.undecided_reason,
        Some(UndecidedReason::ProverUnavailable)
    );
    assert!(details.private.is_none());
    assert!(details.proof_ids.is_empty());
    // A one-request grant ends with its request (D18).
    let ended = broker.grant(&grant_id, |g| g.ended.is_some());
    assert_eq!(
        ended.ended.map(|e| e.reason),
        Some(GrantEndReason::Consumed)
    );
    let snapshot = broker.snapshot();
    assert_eq!(snapshot["grants"], json!([]));
    assert!(
        snapshot["requests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["request_id"] != json!(request_id))
    );
}

/// The refusals of `decide_request`, then a denial, then the synthetic agent's second request.
#[test]
fn refused_decisions_and_a_denial() {
    let mut broker = Broker::synthetic();
    broker.result(&subscribe(0));
    let pending = broker.pending(&[]);
    let request_id = pending.request_id.as_str().to_owned();

    let deny = decide(&pending, "1", "deny", Value::Null);
    let mut stale_revision = deny.clone();
    stale_revision["id"] = json!("stale-revision");
    stale_revision["params"]["expected_revision"] = json!(3);
    let mut stale_digest = deny.clone();
    stale_digest["id"] = json!("stale-digest");
    stale_digest["params"]["request_digest"] = json!(format!("sha256:{}", "a".repeat(64)));
    let mut unknown = deny.clone();
    unknown["id"] = json!("unknown");
    unknown["params"]["request_id"] = json!("request-99");
    let mut narrowed = decide(&pending, "narrowed", "approve", json!({"kind": "session"}));
    narrowed["params"]["selected"] = json!({
        "calendars": [{"account_id": "synthetic-account", "calendar_id": "synthetic-calendar"}],
        "window": null, "fields": null, "recipients": null, "send_updates": null
    });
    for (frame, code, detail) in [
        (&stale_revision, ErrorCode::StaleRequest, Value::Null),
        (&stale_digest, ErrorCode::StaleRequest, Value::Null),
        (&unknown, ErrorCode::NotFound, Value::Null),
        (
            &narrowed,
            ErrorCode::InvalidParams,
            json!({"pointer": "/params/selected"}),
        ),
    ] {
        let response = broker.ask(frame);
        assert_eq!(
            error_of(&response),
            (
                code,
                frame["id"].as_str().map(str::to_owned),
                Some("decide_request".into()),
                detail
            )
        );
    }

    let OwnerResult::DecideRequest(decided) = broker.result(&deny) else {
        panic!()
    };
    assert_eq!(decided.request.status.state(), RequestState::Denied);
    assert!(matches!(decided.approval, Approval::Deny(_)));
    let denied = broker.request_in(&request_id, RequestState::Denied);
    assert_eq!(decided.request, denied);

    let second = broker.pending(&[&request_id]);
    assert_eq!(second.tool.as_str(), "fabric_calendar_propose_create");
    let states = states_of(&broker.notices, &request_id);
    assert_eq!(first_invalid(&states), None);
    assert!(broker.notices.iter().all(|n| n.change.kind() != "grant"));
    assert_consecutive(&broker.notices);
}

#[test]
fn connect_google_is_not_available_until_cd_w05() {
    let fixtures = contract_dir().join("fixtures/messages");
    let mut frames = Vec::new();
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("accept-owner-request-connect-google") {
            continue;
        }
        let fixture: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        frames.push(fixture["value"].clone());
    }
    assert!(!frames.is_empty(), "no connect_google fixture");
    let mut broker = Broker::start();
    for frame in &frames {
        let response = broker.ask(frame);
        assert_eq!(
            error_of(&response),
            (
                ErrorCode::UnknownMethod,
                frame["id"].as_str().map(str::to_owned),
                Some("connect_google".to_owned()),
                Value::Null
            )
        );
        let OwnerResponse::Error { error, .. } = &response else {
            panic!()
        };
        assert_eq!(
            error.message.as_str(),
            "This build cannot connect a Google account yet; CD-W05 adds it."
        );
    }
    // Not stored: the same id is answered the same way again, not as a reused command.
    let response = broker.ask(&frames[0]);
    assert_eq!(error_of(&response).0, ErrorCode::UnknownMethod);
}

#[test]
fn malformed_frames() {
    let mut broker = Broker::start();
    let malformed = (ErrorCode::MalformedFrame, None, None, Value::Null);
    for line in [&b"{\"protocol\": \n"[..], b"not json\n", b"[1]\n", b"\n"] {
        broker.send_bytes(line);
        assert_eq!(error_of(&broker.response().0), malformed);
    }
    // Invalid UTF-8.
    broker.send_bytes(b"{\"a\":\"\xff\"}\n");
    assert_eq!(error_of(&broker.response().0), malformed);
    // Still answering.
    assert!(matches!(
        broker.ask(&hello(2)),
        OwnerResponse::Result { .. }
    ));
}

#[test]
fn a_line_of_exactly_the_bound_is_read_as_a_frame() {
    let mut broker = Broker::start();
    // 1,048,576 bytes with no line feed: within the bound, but not JSON.
    let mut line = vec![b'x'; MAX_FRAME_BYTES];
    line.push(b'\n');
    broker.send_bytes(&line);
    assert_eq!(
        error_of(&broker.response().0),
        (ErrorCode::MalformedFrame, None, None, Value::Null)
    );
    // Exactly one answer: the next one is the hello's.
    assert!(matches!(
        broker.ask(&hello(2)),
        OwnerResponse::Result { .. }
    ));
}

#[test]
fn an_oversized_line_is_answered_once_and_skipped() {
    let mut broker = Broker::start();
    for length in [MAX_FRAME_BYTES + 1, 3 * MAX_FRAME_BYTES + 17] {
        // A valid frame start, so that reading it whole would give a different answer.
        let mut line = br#"{"protocol":"fabric-desktop/1","id":"big","method":"hello","params":{"client":{"name":""#.to_vec();
        line.resize(length, b'a');
        line.push(b'\n');
        broker.send_bytes(&line);
        assert_eq!(
            error_of(&broker.response().0),
            (ErrorCode::MalformedFrame, None, None, Value::Null)
        );
        let OwnerResponse::Result { id, .. } = broker.ask(&hello(2)) else {
            panic!()
        };
        assert_eq!(id.as_str(), "hello-1");
    }
}

#[test]
fn wrong_protocol() {
    let mut broker = Broker::start();
    let mut frame = hello(2);
    frame["protocol"] = json!("fabric-desktop/2");
    assert_eq!(
        error_of(&broker.ask(&frame)),
        (
            ErrorCode::UnsupportedProtocol,
            Some("hello-1".into()),
            Some("hello".into()),
            json!({"received": "fabric-desktop/2", "supported": ["fabric-desktop/1"]})
        )
    );
    frame["protocol"] = json!(1);
    frame["id"] = json!("not an id");
    assert_eq!(
        error_of(&broker.ask(&frame)),
        (
            ErrorCode::UnsupportedProtocol,
            None,
            Some("hello".into()),
            json!({"received": null, "supported": ["fabric-desktop/1"]})
        )
    );
}

#[test]
fn schema_refusals_carry_the_id_and_method() {
    let mut broker = Broker::start();
    assert_eq!(
        error_of(&broker.ask(&request("x-1", "teleport", json!({})))),
        (
            ErrorCode::UnknownMethod,
            Some("x-1".into()),
            Some("teleport".into()),
            Value::Null
        )
    );
    assert_eq!(
        error_of(&broker.ask(&request("x-2", "hello", json!({})))),
        (
            ErrorCode::InvalidParams,
            Some("x-2".into()),
            Some("hello".into()),
            json!({"pointer": "/params"})
        )
    );
}

#[test]
fn an_argument_is_a_usage_error() {
    let dir = TempDir::new("usage");
    // stdin stays open and empty: the broker must exit without reading it.
    let mut child = spawn(
        command(&dir.data(), false)
            .arg("--help")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    // The bound includes the launch, so it only catches a broker that never exits.
    let status = wait_with_bound(&mut child, LAUNCH_AND_EXIT);
    assert_eq!(status.code(), Some(64));
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    assert!(stdout.is_empty());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.starts_with("usage: fabric-broker"), "{stderr}");
    assert!(!dir.data().exists(), "a usage error created the store");
}

/// End of input stops the broker at once, with exit code 0, while the synthetic agent runs;
/// and everything it wrote is a frame.
#[test]
fn end_of_input_stops_the_broker_and_stdout_holds_only_frames() {
    let mut broker = Broker::synthetic();
    broker.result(&hello(2));
    broker.result(&subscribe(0));
    broker.notice(|n| n.change.kind() == "account");
    // A request written just before the end is still answered.
    broker.send(&request("snap-1", "get_snapshot", json!({})));
    let (status, took, rest) = broker.finish();
    assert_eq!(status.code(), Some(0));
    assert!(took < STOP_BOUND, "stopping took {took:?}");
    assert!(
        rest.iter()
            .any(|w| matches!(&w.frame, Frame::OwnerResponse(OwnerResponse::Result { id, .. }) if id.as_str() == "snap-1"))
    );
    // Every line was a canonical frame: `decode_line` checked each one, including those read
    // before the end.
    assert!(rest.iter().all(|w| w.line.len() > 1));
}

#[test]
fn end_of_input_before_anything_exits_zero() {
    let broker = Broker::start();
    // Nothing shows that the broker is running when stdin closes, so the time to exit includes
    // its launch and is held only to `LAUNCH_AND_EXIT` (inside `finish`). The stop bound is
    // checked where a `hello` was answered first.
    let (status, _took, rest) = broker.finish();
    assert_eq!(status.code(), Some(0));
    assert!(rest.is_empty());
}

/// When the host is gone, the broker's next write fails and it ends with a non-zero code.
#[test]
fn a_write_error_ends_the_broker() {
    let dir = TempDir::new("write-error");
    let mut child = spawn(
        command(&dir.data(), false)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    let mut line = canonical(&hello(2)).into_bytes();
    line.push(b'\n');
    // The broker may already be gone after the first answer, so later writes may fail.
    for _ in 0..3 {
        if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
            break;
        }
    }
    // The bound includes the launch, so it only catches a broker that never exits.
    let status = wait_with_bound(&mut child, LAUNCH_AND_EXIT);
    assert!(!status.success(), "{status:?}");
    assert_eq!(status.code(), Some(74));
}

/// One refused configuration: what it is, the variables the broker gets, and the name its
/// stderr line must hold.
type ConfigCase = (&'static str, Vec<(&'static str, OsString)>, &'static str);

/// p01: every refused setting ends the broker with exit 78 and one stderr line naming what was
/// refused, before any frame and without reading stdin. No case starts a broker with the
/// Keychain on macOS.
#[test]
fn configuration_errors_exit_78_before_any_frame() {
    let dir = TempDir::new("config");
    let data = dir.data();
    let file = dir.0.join("a file");
    std::fs::write(&file, b"not a directory").unwrap();
    let data_os = data.as_os_str().to_owned();
    let os = OsString::from;
    let mut cases: Vec<ConfigCase> = vec![
        (
            "no data directory",
            vec![(KEY_SOURCE, os("test"))],
            DATA_DIR,
        ),
        (
            "a relative data directory",
            vec![(DATA_DIR, os("relative/data")), (KEY_SOURCE, os("test"))],
            DATA_DIR,
        ),
        (
            "a data directory that is a file",
            vec![
                (DATA_DIR, file.as_os_str().to_owned()),
                (KEY_SOURCE, os("test")),
            ],
            DATA_DIR,
        ),
        (
            "a data directory that cannot be created",
            vec![
                (DATA_DIR, file.join("data").into_os_string()),
                (KEY_SOURCE, os("test")),
            ],
            "data directory",
        ),
        (
            "no key source",
            vec![(DATA_DIR, data_os.clone())],
            KEY_SOURCE,
        ),
        (
            "an unknown key source",
            vec![(DATA_DIR, data_os.clone()), (KEY_SOURCE, os("plaintext"))],
            KEY_SOURCE,
        ),
        (
            "an unknown synthetic value",
            vec![
                (DATA_DIR, data_os.clone()),
                (KEY_SOURCE, os("test")),
                (SYNTHETIC_AGENT, os("yes")),
            ],
            SYNTHETIC_AGENT,
        ),
        (
            "the synthetic agent without a key source",
            vec![(DATA_DIR, data_os.clone()), (SYNTHETIC_AGENT, os("1"))],
            KEY_SOURCE,
        ),
        (
            "an unknown broker variable",
            vec![
                (DATA_DIR, data_os.clone()),
                (KEY_SOURCE, os("test")),
                ("FABRIC_BROKER_DATADIR", data_os.clone()),
            ],
            "FABRIC_BROKER_DATADIR",
        ),
    ];
    // PW p40: the three Isabelle paths, each refused before any frame.
    let fake = FakeIsabelle::new(&dir.0.join("fake"));
    let (home, user, rules) = (
        fake.home.as_os_str().to_owned(),
        fake.user.as_os_str().to_owned(),
        fake.rules.as_os_str().to_owned(),
    );
    let missing = dir.0.join("missing").into_os_string();
    let isabelle = |h: &OsString, u: &OsString, r: &OsString| {
        vec![
            (DATA_DIR, data_os.clone()),
            (KEY_SOURCE, os("test")),
            (ISABELLE_HOME, h.clone()),
            (ISABELLE_USER_HOME, u.clone()),
            (RULES_DIR, r.clone()),
        ]
    };
    let some = |set: &[(&'static str, &OsString)]| {
        let mut variables = vec![(DATA_DIR, data_os.clone()), (KEY_SOURCE, os("test"))];
        variables.extend(set.iter().map(|(name, value)| (*name, (*value).clone())));
        variables
    };
    let together = "together, or none";
    cases.extend([
        (
            "an Isabelle home alone",
            some(&[(ISABELLE_HOME, &home)]),
            together,
        ),
        (
            "an Isabelle user home alone",
            some(&[(ISABELLE_USER_HOME, &user)]),
            together,
        ),
        (
            "a rules directory alone",
            some(&[(RULES_DIR, &rules)]),
            together,
        ),
        (
            "two of the three Isabelle paths",
            some(&[(ISABELLE_HOME, &home), (RULES_DIR, &rules)]),
            together,
        ),
        (
            "a relative Isabelle home",
            isabelle(&os("isabelle"), &user, &rules),
            ISABELLE_HOME,
        ),
        (
            "an Isabelle home with no bin/isabelle",
            isabelle(&user, &user, &rules),
            ISABELLE_HOME,
        ),
        (
            "a relative Isabelle user home",
            isabelle(&home, &os("user"), &rules),
            ISABELLE_USER_HOME,
        ),
        (
            "a missing Isabelle user home",
            isabelle(&home, &missing, &rules),
            ISABELLE_USER_HOME,
        ),
        (
            "a relative rules directory",
            isabelle(&home, &user, &os("isabelle")),
            RULES_DIR,
        ),
        (
            "a rules directory without the rule theory",
            isabelle(&home, &user, &user),
            RULES_DIR,
        ),
    ]);
    #[cfg(unix)]
    {
        let fixed = FakeIsabelle::new(&dir.0.join("not executable"));
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            fixed.home.join("bin").join("isabelle"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        cases.push((
            "an Isabelle that may not be executed",
            isabelle(&fixed.home.into_os_string(), &user, &rules),
            ISABELLE_HOME,
        ));
    }
    #[cfg(unix)]
    cases.push((
        "a data directory that others may read",
        vec![
            (DATA_DIR, dir.0.as_os_str().to_owned()),
            (KEY_SOURCE, os("test")),
        ],
        "data directory mode",
    ));
    // Never on macOS: a macOS broker with `keychain` would open the login Keychain.
    #[cfg(not(target_os = "macos"))]
    {
        cases.push((
            "the keychain off macOS",
            vec![(DATA_DIR, data_os.clone()), (KEY_SOURCE, os("keychain"))],
            KEY_SOURCE,
        ));
        cases.push((
            "the synthetic agent with the keychain",
            vec![
                (DATA_DIR, data_os.clone()),
                (KEY_SOURCE, os("keychain")),
                (SYNTHETIC_AGENT, os("1")),
            ],
            KEY_SOURCE,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (case, variables, named) in cases {
        let mut command = bare_command();
        for (name, value) in &variables {
            command.env(name, value);
        }
        // stdin stays open and empty: the broker must exit without reading it.
        let mut child = spawn(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let status = wait_with_bound(&mut child, LAUNCH_AND_EXIT);
        let mut stdout = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut stdout)
            .unwrap();
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(status.code(), Some(78), "{case}: {stderr}");
        assert!(stdout.is_empty(), "{case}: a frame was written");
        assert_eq!(stderr.lines().count(), 1, "{case}: {stderr}");
        assert!(stderr.contains(named), "{case}: {stderr}");
        assert!(!stderr.contains("plaintext"), "{case}: {stderr}");
        assert!(!data.exists(), "{case}: the store was created");
    }
}

/// p02: the store's lock keeps a second broker out; it stays up, answers `hello`, and answers
/// everything else `store_unavailable`.
#[test]
fn a_second_broker_on_the_same_directory_answers_store_unavailable() {
    let dir = TempDir::new("second");
    let mut first = Broker::on(&dir.data(), false);
    assert!(matches!(first.ask(&hello(2)), OwnerResponse::Result { .. }));
    let mut second = Broker::on(&dir.data(), false);
    assert!(matches!(
        second.ask(&hello(2)),
        OwnerResponse::Result { .. }
    ));
    let (code, id, method, detail) =
        error_of(&second.ask(&request("snap-2", "get_snapshot", json!({}))));
    assert_eq!(
        (code, id.as_deref(), method.as_deref(), detail),
        (
            ErrorCode::StoreUnavailable,
            Some("snap-2"),
            Some("get_snapshot"),
            json!({"reason": "locked"})
        )
    );
    // The first broker is not disturbed.
    assert_eq!(first.snapshot()["health"]["store"], "ready");
    let (status, _, _) = second.finish();
    assert_eq!(status.code(), Some(0));
}

/// p03: a store written by a newer schema is refused and left alone; `hello` still answers and
/// stdout holds frames only.
#[test]
fn a_newer_schema_store_is_refused_but_hello_answers() {
    use fabric_storage::{MIGRATIONS, Migration, OpenOptions, Store, TestKey};
    static TWO: [Migration; 2] = [
        Migration {
            to: 1,
            sql: MIGRATIONS[0].sql,
        },
        Migration {
            to: 2,
            sql: "CREATE TABLE later_table (x INTEGER) STRICT;",
        },
    ];
    let dir = TempDir::new("newer");
    let store = Store::open_with_migrations(
        OpenOptions {
            data_dir: dir.data(),
            key: Box::new(TestKey),
            now_ms: 1_795_190_400_000,
        },
        &TWO,
    )
    .expect("a store of schema 2 is written");
    store.close().expect("the store closes");
    let database = dir.data().join(fabric_storage::DATABASE_FILE);
    let before = std::fs::read(&database).unwrap();

    let mut broker = Broker::on(&dir.data(), false);
    let OwnerResult::Hello(hello) = broker.result(&hello(2)) else {
        panic!()
    };
    assert_eq!(hello.broker.name.as_str(), "fabric-broker");
    let response = broker.ask(&request("snap-1", "get_snapshot", json!({})));
    assert_eq!(
        error_of(&response).3,
        json!({"reason": "locked"}),
        "{response:?}"
    );
    let OwnerResponse::Error { error, .. } = &response else {
        panic!()
    };
    assert_eq!(error.code(), ErrorCode::StoreUnavailable);
    assert!(error.message.as_str().contains("newer"), "{error:?}");
    let (status, _, rest) = broker.finish();
    assert_eq!(status.code(), Some(0));
    assert!(rest.is_empty());
    assert_eq!(
        std::fs::read(&database).unwrap(),
        before,
        "the store changed"
    );
}

/// p04: a kill loses nothing committed. A pending request survives, a session grant ends with
/// `broker_restart` at the next start, an until-revoked grant survives, and the notice sequences
/// continue across runs.
#[test]
fn state_survives_sigkill_and_restart() {
    let dir = TempDir::new("survive");

    // Run 1: a session grant on the first request; the second request stays pending.
    let mut run = Broker::on(&dir.data(), true);
    run.result(&subscribe(0));
    let first = run.pending(&[]);
    let first_id = first.request_id.as_str().to_owned();
    let OwnerResult::DecideRequest(decided) =
        run.result(&decide(&first, "1", "approve", json!({"kind": "session"})))
    else {
        panic!()
    };
    let Approval::Approve(approval) = &decided.approval else {
        panic!()
    };
    let session_grant = approval.grant_id.as_str().to_owned();
    run.request_in(&first_id, RequestState::Undecided);
    let second = run.pending(&[&first_id]);
    let second_id = second.request_id.as_str().to_owned();
    let cursor = run.snapshot()["cursor"].as_u64().unwrap();
    thread::sleep(Duration::from_millis(300));
    while let Ok(line) = run.lines.try_recv() {
        if let Frame::OwnerNotice(n) = decode_line(&line) {
            run.notices.push(n);
        }
    }
    let cursor = cursor.max(run.last_sequence());
    assert_consecutive(&run.notices);
    run.kill();

    // Run 2, without the synthetic agent: the pending request is there, the session grant ended
    // at the start, and the notices continue from the last one.
    let mut run = Broker::after_kill(&dir.data(), false);
    let OwnerResult::Subscribe(subscribed) = run.result(&subscribe(cursor)) else {
        panic!()
    };
    assert_eq!(subscribed.cursor.get(), cursor);
    let ended = run.grant(&session_grant, |g| g.ended.is_some());
    assert_eq!(
        ended.ended.map(|e| e.reason),
        Some(GrantEndReason::BrokerRestart)
    );
    assert_eq!(run.notices[0].sequence.get(), cursor + 1);
    assert_consecutive(&run.notices);
    let snapshot = run.snapshot();
    let requests = snapshot["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["request_id"], json!(second_id));
    assert_eq!(requests[0]["status"]["state"], "pending_approval");
    assert_eq!(snapshot["grants"], json!([]));
    let details = request(
        "details-2",
        "get_request_details",
        json!({"request_id": second_id, "redaction": "redacted"}),
    );
    let OwnerResult::GetRequestDetails(details) = run.result(&details) else {
        panic!()
    };
    let OwnerResult::DecideRequest(decided) = run.result(&decide(
        &details.request,
        "2",
        "approve",
        json!({"kind": "until_revoked"}),
    )) else {
        panic!()
    };
    let Approval::Approve(approval) = &decided.approval else {
        panic!()
    };
    let kept_grant = approval.grant_id.as_str().to_owned();
    run.request_in(&second_id, RequestState::Undecided);
    let cursor = run.snapshot()["cursor"].as_u64().unwrap();
    run.kill();

    // Run 3: the until-revoked grant is still live; nothing is open.
    let mut run = Broker::after_kill(&dir.data(), false);
    let snapshot = run.snapshot();
    assert!(snapshot["cursor"].as_u64().unwrap() >= cursor);
    let grants: Vec<&Value> = snapshot["grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| &g["grant_id"])
        .collect();
    assert_eq!(grants, [&json!(kept_grant)]);
    assert_eq!(snapshot["grants"][0]["term"]["kind"], "until_revoked");
    assert_eq!(snapshot["requests"], json!([]));
    assert_eq!(snapshot["health"]["store"], "ready");
}

/// p05: the command table is durable: after a restart the same command gets the same bytes and
/// changes nothing, and its id with another command is still refused.
#[test]
fn a_duplicate_command_after_restart_gets_the_same_bytes() {
    let dir = TempDir::new("dedup");
    let mut run = Broker::on(&dir.data(), true);
    run.result(&subscribe(0));
    let pending = run.pending(&[]);
    let deny = decide(&pending, "1", "deny", Value::Null);
    run.send(&deny);
    let (response, first_answer) = run.response();
    assert!(matches!(response, OwnerResponse::Result { .. }));
    run.request_in(pending.request_id.as_str(), RequestState::Denied);
    run.kill();

    let mut run = Broker::after_kill(&dir.data(), false);
    let cursor = run.snapshot()["cursor"].as_u64().unwrap();
    run.send(&deny);
    let (_, again) = run.response();
    assert_eq!(again, first_answer);
    let mut other = deny.clone();
    other["params"]["decision"] = json!("approve");
    other["params"]["duration"] = json!({"kind": "until_revoked"});
    assert_eq!(error_of(&run.ask(&other)).0, ErrorCode::CommandReused);
    assert_eq!(run.snapshot()["cursor"].as_u64().unwrap(), cursor);
}

/// p06: end of input ends the live sessions and their grants (`app_quit`) in one commit and the
/// broker exits within the bound; the next run starts with none.
#[test]
fn end_of_input_ends_sessions_and_exits_within_the_bound() {
    let dir = TempDir::new("quit");
    let mut run = Broker::on(&dir.data(), true);
    run.result(&subscribe(0));
    run.notice(|n| matches!(&n.change, Change::Agent(a) if a.session.is_some()));
    let pending = run.pending(&[]);
    let OwnerResult::DecideRequest(decided) = run.result(&decide(
        &pending,
        "1",
        "approve",
        json!({"kind": "session"}),
    )) else {
        panic!()
    };
    let Approval::Approve(approval) = &decided.approval else {
        panic!()
    };
    let grant_id = approval.grant_id.as_str().to_owned();
    run.request_in(pending.request_id.as_str(), RequestState::Undecided);
    let (status, took, rest) = run.finish();
    assert_eq!(status.code(), Some(0));
    assert!(took < STOP_BOUND, "stopping took {took:?}");
    let ended = rest.iter().find_map(|w| match &w.frame {
        Frame::OwnerNotice(n) => match &n.change {
            Change::Grant(g) if g.grant_id.as_str() == grant_id => g.ended.clone(),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(ended.map(|e| e.reason), Some(GrantEndReason::AppQuit));
    assert!(rest.iter().any(|w| matches!(&w.frame,
        Frame::OwnerNotice(n) if matches!(&n.change, Change::Agent(a) if a.session.is_none()))));

    let mut run = Broker::on(&dir.data(), false);
    let snapshot = run.snapshot();
    assert_eq!(snapshot["grants"], json!([]));
    assert_eq!(snapshot["agents"][0]["session"], Value::Null);
}

/// p07: the synthetic agent does nothing before the page subscribes; then it enrolls, asks,
/// and an approval ends `undecided`, because the synthetic classifier never permits. Its proof
/// reports are not shown as proofs. A second request follows.
#[test]
fn the_synthetic_agent_waits_for_subscribe_asks_and_an_approval_ends_undecided() {
    let mut broker = Broker::synthetic();
    let OwnerResult::Hello(hello) = broker.result(&hello(2)) else {
        panic!()
    };
    assert_eq!(
        hello.broker.build.as_str(),
        format!("{} CD-W04 synthetic", env!("CARGO_PKG_VERSION"))
    );
    thread::sleep(Duration::from_millis(1500));
    let before = broker.snapshot();
    for empty in ["accounts", "agents", "requests", "grants"] {
        assert_eq!(before[empty], json!([]), "{empty} before subscribe");
    }
    assert_eq!(before["health"]["prover"], "unavailable");
    assert!(broker.notices.is_empty());

    let cursor = before["cursor"].as_u64().unwrap();
    broker.result(&subscribe(cursor));
    let agent = broker.notice(|n| n.change.kind() == "agent");
    let Change::Agent(agent) = agent.change else {
        unreachable!()
    };
    assert_eq!(agent.agent_id.as_str(), "synthetic-agent");
    assert_eq!(
        agent.registered_name.as_str(),
        "Synthetic agent (test build)"
    );
    broker.notice(|n| n.change.kind() == "account");
    let pending = broker.pending(&[]);
    let request_id = pending.request_id.as_str().to_owned();
    assert_eq!(pending.agent_id.as_str(), "synthetic-agent");
    assert_eq!(
        pending.account_id.as_ref().map(|a| a.as_str()),
        Some("synthetic-account")
    );

    let details = request(
        "details-1",
        "get_request_details",
        json!({"request_id": request_id, "redaction": "full"}),
    );
    let response = broker.ask(&details);
    let OwnerResponse::Result {
        result: OwnerResult::GetRequestDetails(read),
        ..
    } = &response
    else {
        panic!("{response:?}")
    };
    let card = read.card.clone().expect("a card");
    assert_eq!(card.insufficiency.rule.as_str(), "synthetic_no_rule_theory");
    let value = serde_json::to_value(read.private.as_ref().expect("full details")).unwrap();
    assert_eq!(
        value["action"]["arguments"]["event"]["title"],
        SYNTHETIC_TITLE
    );
    let proof = request(
        "proof-1",
        "get_proof",
        json!({"proof_id": card.insufficiency.proof_id}),
    );
    assert_eq!(error_of(&broker.ask(&proof)).0, ErrorCode::NotFound);

    let approve = decide(&pending, "1", "approve", json!({"kind": "until_revoked"}));
    let OwnerResult::DecideRequest(decided) = broker.result(&approve) else {
        panic!()
    };
    let Approval::Approve(approval) = &decided.approval else {
        panic!()
    };
    let grant_id = approval.grant_id.as_str().to_owned();
    broker.request_in(&request_id, RequestState::Undecided);
    let states = states_of(&broker.notices, &request_id);
    assert_eq!(first_invalid(&states), None);
    assert_eq!(
        states,
        [
            RequestState::Received,
            RequestState::Validating,
            RequestState::ProvingClassification,
            RequestState::PendingApproval,
            RequestState::Approved,
            RequestState::ProvingFreshPermit,
            RequestState::Undecided
        ]
    );
    let second = broker.pending(&[&request_id]);
    assert_ne!(second.request_id.as_str(), request_id);
    assert_consecutive(&broker.notices);
    let snapshot = broker.snapshot();
    let grants: Vec<&Value> = snapshot["grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| &g["grant_id"])
        .collect();
    assert_eq!(grants, [&json!(grant_id)]);
}

/// p08: stdout holds frames only and no event content outside a full-detail read; stderr holds
/// neither event content nor the key.
#[test]
fn stdout_and_stderr_carry_no_event_content() {
    let dir = TempDir::new("content");
    let mut broker = Broker::spawn_from(command(&dir.data(), true), true);
    broker.result(&subscribe(0));
    let pending = broker.pending(&[]);
    let request_id = pending.request_id.as_str().to_owned();
    let details = request(
        "details-1",
        "get_request_details",
        json!({"request_id": request_id, "redaction": "redacted"}),
    );
    broker.result(&details);
    broker.result(&decide(
        &pending,
        "1",
        "approve",
        json!({"kind": "one_request"}),
    ));
    broker.request_in(&request_id, RequestState::Undecided);
    broker.pending(&[&request_id]);
    let stderr = broker.stderr.take().expect("stderr is read");
    let mut lines = broker.seen.clone();
    let (status, _, rest) = broker.finish();
    assert_eq!(status.code(), Some(0));
    lines.extend(rest.into_iter().map(|w| w.line));
    let stderr = stderr.join().expect("stderr was read");
    let key_hex = "46".repeat(32);
    assert!(lines.len() > 10, "only {} lines", lines.len());
    for line in &lines {
        let text = String::from_utf8_lossy(line);
        assert!(!text.contains(SYNTHETIC_TITLE), "{text}");
        assert!(!text.contains(&key_hex), "{text}");
    }
    assert!(!stderr.contains(SYNTHETIC_TITLE), "{stderr}");
    assert!(!stderr.to_lowercase().contains(&key_hex), "{stderr}");
}

/// Paths laid out as the three Isabelle variables need them, under a directory the test keeps:
/// `bin/isabelle` a script that exits 1 at once, an empty user home, and the repository's rule
/// theory. No Isabelle runs from it, so the prover of a broker given these paths fails to start
/// and nothing is ever proved.
struct FakeIsabelle {
    home: PathBuf,
    user: PathBuf,
    rules: PathBuf,
}

impl FakeIsabelle {
    fn new(root: &Path) -> FakeIsabelle {
        let home = root.join("isabelle");
        let user = root.join("user");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let program = home.join("bin").join("isabelle");
        std::fs::write(&program, b"#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakeIsabelle {
            home,
            user,
            rules: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../isabelle"),
        }
    }

    /// The broker on `data_dir` with the test key, the synthetic agent and these paths: a
    /// practice run whose prover cannot start.
    #[cfg(unix)]
    fn command(&self, data_dir: &Path) -> Command {
        let mut command = command(data_dir, true);
        command
            .env(ISABELLE_HOME, &self.home)
            .env(ISABELLE_USER_HOME, &self.user)
            .env(RULES_DIR, &self.rules);
        command
    }
}

#[cfg(unix)]
impl Broker {
    /// Asks for snapshots until the prover's health is `health`, for up to `bound`.
    fn prover_becomes(&mut self, health: &str, bound: Duration) -> Value {
        let deadline = Instant::now() + bound;
        loop {
            let snapshot = self.snapshot();
            if snapshot["health"]["prover"] == health {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "the prover was not {health} within {bound:?}: {}",
                snapshot["health"]
            );
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// PW p41: an Isabelle that cannot start leaves the prover unavailable, with the problem
/// `failed_will_retry`: the synthetic agent of the practice run proposes nothing, no request is
/// made, stderr names no path, and after quit the prover's work directory is gone. (A request
/// made while the prover is down ends `undecided`: the shell's unit test PW sh06, since the
/// practice script never proposes then.)
#[cfg(unix)]
#[test]
fn an_isabelle_that_cannot_start_leaves_the_prover_unavailable_and_proposes_nothing() {
    let dir = TempDir::new("no-isabelle");
    let fake = FakeIsabelle::new(&dir.0.join("fake"));
    let mut broker = Broker::spawn_from(fake.command(&dir.data()), true);
    let OwnerResult::Hello(hello) = broker.result(&hello(2)) else {
        panic!()
    };
    assert_eq!(
        hello.broker.build.as_str(),
        format!("{} CD-W04 synthetic practice", env!("CARGO_PKG_VERSION"))
    );
    let snapshot = broker.snapshot();
    broker.result(&subscribe(snapshot["cursor"].as_u64().unwrap()));
    // Asserted on the snapshot, not on a notice: the fake may fail before the subscription's
    // cursor. The first failure leaves a restart wait, so the broker will try again.
    let unavailable = broker.prover_becomes("unavailable", PROMPT);
    assert_eq!(
        unavailable["health"]["prover_problem"], "failed_will_retry",
        "{}",
        unavailable["health"]
    );
    // Longer than the agent's pace: it would have proposed by now if it could.
    thread::sleep(Duration::from_millis(2500));
    let snapshot = broker.snapshot();
    assert_eq!(snapshot["health"]["prover"], "unavailable");
    assert_eq!(snapshot["requests"], json!([]));
    assert!(broker.notices.iter().all(|n| n.change.kind() != "request"));
    let stderr = broker.stderr.take().expect("stderr is read");
    let (status, _, _) = broker.finish();
    assert_eq!(status.code(), Some(0));
    let stderr = stderr.join().expect("stderr was read");
    assert!(
        stderr.contains("fabric-broker: the rule prover could not start"),
        "{stderr}"
    );
    assert!(!stderr.contains(&*dir.0.to_string_lossy()), "{stderr}");
    assert!(!dir.data().join("prover").exists());
}

/// PW p44: a second broker on a directory whose store the first one holds never starts a prover,
/// so it leaves the first one's work directory alone. The marker goes into that directory only
/// after the first prover failed, and is checked before the first one's restart wait (10 s)
/// could empty the directory again. `failed` lags the start of that wait by up to one poll and
/// a snapshot, so the test concludes only with a second to spare.
#[cfg(unix)]
#[test]
fn a_second_broker_on_a_locked_directory_leaves_the_first_brokers_prover_alone() {
    let dir = TempDir::new("locked-prover");
    let fake = FakeIsabelle::new(&dir.0.join("fake"));
    let mut first = Broker::spawn_from(fake.command(&dir.data()), false);
    first.prover_becomes("unavailable", PROMPT);
    let failed = Instant::now();
    let marker = dir.data().join("prover").join("marker");
    std::fs::write(&marker, b"the first broker's").expect("the first broker's prover directory");
    let mut second = Broker::spawn_from(fake.command(&dir.data()), false);
    assert!(matches!(
        second.ask(&hello(2)),
        OwnerResponse::Result { .. }
    ));
    let (code, ..) = error_of(&second.ask(&request("snap-2", "get_snapshot", json!({}))));
    assert_eq!(code, ErrorCode::StoreUnavailable);
    let (status, _, _) = second.finish();
    assert_eq!(status.code(), Some(0));
    let elapsed = failed.elapsed();
    assert!(
        elapsed < fabric_broker::prover::RESTART_WAITS[0] - Duration::from_secs(1),
        "too slow to conclude: {elapsed:?} since the first prover failed"
    );
    assert!(
        marker.exists(),
        "the second broker touched the first one's prover"
    );
    let (status, _, _) = first.finish();
    assert_eq!(status.code(), Some(0));
    assert!(!dir.data().join("prover").exists());
}

/// PW p45: a check theory a killed broker left in `<data_dir>/prover` is removed
/// once the store is open, also by a broker started without the three Isabelle variables.
#[test]
fn a_start_without_isabelle_removes_a_check_theory_a_killed_broker_left() {
    let dir = TempDir::new("left-check");
    let (status, _, _) = Broker::on(&dir.data(), false).finish();
    assert_eq!(status.code(), Some(0));
    let work_dir = dir.data().join("prover");
    std::fs::create_dir_all(&work_dir).unwrap();
    let left = work_dir.join("Check_000001.thy");
    std::fs::write(&left, "theory Check_000001 (* left by a killed broker *)").unwrap();
    let mut broker = Broker::on(&dir.data(), false);
    let OwnerResult::Hello(_) = broker.result(&hello(2)) else {
        panic!()
    };
    assert!(!work_dir.exists(), "the left check theory stays");
    let (status, _, _) = broker.finish();
    assert_eq!(status.code(), Some(0));
}
