//! The real broker driven through the host's relay crate, started the way the Tauri host
//! starts it: `BrokerConfig::check` with the relay's environment allowlist, then
//! `Session::start` with a sink that hands every event to the test through a channel.
//!
//! The broker under test is `FABRIC_BROKER_UNDER_TEST` when it is set (an absolute path, for
//! example the broker inside a packaged app) and otherwise the binary Cargo built.
//!
//! Like the host, each start takes the test's environment without any `FABRIC_BROKER_*` name
//! and adds the broker's three variables: a new data directory of its own under the system's
//! temporary directory, the public test key, and the synthetic agent. No broker here opens a
//! store with the Keychain.
//!
//! "PW" numbers, such as PW p43, refer to `docs/plans/calendar-desktop-prover-wiring-design.md`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fabric_desktop_contracts::error::ErrorCode;
use fabric_desktop_contracts::owner::{
    Approval, Change, OwnerNotice, OwnerResponse, OwnerResult, Request, RequestDetails,
};
use fabric_desktop_contracts::{
    Frame, Layer, Limits, RequestState, SCHEMA_VERSION, canonical, first_invalid,
};
use fabric_desktop_relay::{
    BrokerConfig, ENVIRONMENT_ALLOWLIST, Ending, Event, QUIT_DRAIN, SendError, Session,
};
use serde_json::{Value, json};

/// How long to wait for a frame that is due at once. The first answer of a broker also waits
/// for its launch, which for a newly signed executable on macOS can take seconds.
const PROMPT: Duration = Duration::from_secs(10);
/// How long to wait for a step of the synthetic agent, beyond its nominal time.
const STEP: Duration = Duration::from_secs(6);

/// Held across every process start in this file (each `Session::start`, and the `kill` or
/// `taskkill` command), and released as soon as the start returns. On macOS there is no
/// `pipe2`, so `std` makes a child's pipes and marks them close-on-exec in two steps; a child
/// started by another test thread in between inherits the write end of a broker's stdin or
/// stdout. The broker then sees end of input, or the relay sees the end of its stdout and
/// sends `Ended`, only when that other child exits too.
static SPAWN: Mutex<()> = Mutex::new(());

fn spawn_lock() -> MutexGuard<'static, ()> {
    SPAWN.lock().unwrap_or_else(PoisonError::into_inner)
}

fn broker_path() -> PathBuf {
    match std::env::var_os("FABRIC_BROKER_UNDER_TEST") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env!("CARGO_BIN_EXE_fabric-broker")),
    }
}

/// The test's environment without any `FABRIC_BROKER_*` name, then the broker's own three:
/// `data_dir`, the test key and the synthetic agent.
fn environment(data_dir: &Path) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| {
            !name
                .as_encoded_bytes()
                .to_ascii_uppercase()
                .starts_with(b"FABRIC_BROKER_")
        })
        .collect();
    environment.extend([
        (
            OsString::from("FABRIC_BROKER_DATA_DIR"),
            data_dir.as_os_str().to_owned(),
        ),
        (
            OsString::from("FABRIC_BROKER_KEY_SOURCE"),
            OsString::from("test"),
        ),
        (
            OsString::from("FABRIC_BROKER_SYNTHETIC_AGENT"),
            OsString::from("1"),
        ),
    ]);
    environment
}

/// A relay session with the broker and the events its sink received.
struct Relay {
    session: Session,
    events: Receiver<Event>,
    /// Every notice read so far, with the time it reached the test.
    notices: Vec<(Instant, OwnerNotice)>,
    /// The broker's data directory, removed after the session.
    _dir: TempDir,
}

impl Relay {
    fn start() -> Relay {
        Relay::start_at(&broker_path())
    }

    fn start_at(program: &Path) -> Relay {
        Relay::start_with(program, Vec::new())
    }

    /// A session with the broker's three variables and `more`, which the relay must pass too.
    fn start_with(program: &Path, more: Vec<(OsString, OsString)>) -> Relay {
        let dir = TempDir::new("fabric relay round trip");
        let expected = 3 + more.len();
        let mut host = environment(&dir.data());
        host.extend(more);
        let config = BrokerConfig::check(program, ENVIRONMENT_ALLOWLIST, host)
            .unwrap_or_else(|e| panic!("the broker configuration is refused: {e}"));
        let names: Vec<&OsString> = config.environment().iter().map(|(n, _)| n).collect();
        assert!(
            names
                .iter()
                .filter(|n| n.as_encoded_bytes().starts_with(b"FABRIC_BROKER_"))
                .count()
                == expected,
            "the broker gets its {expected} variables and no other: {names:?}"
        );
        let (sender, events) = mpsc::channel();
        let spawning = spawn_lock();
        let session = Session::start(&config, move |event| {
            // The test may have stopped listening; the event is then of no use.
            let _ = sender.send(event);
        })
        .expect("the broker starts");
        drop(spawning);
        Relay {
            session,
            events,
            notices: Vec::new(),
            _dir: dir,
        }
    }

    fn send(&self, value: &Value) {
        self.session
            .send(canonical(value).as_bytes())
            .unwrap_or_else(|e| panic!("the relay did not send the frame: {e}"));
    }

    fn event(&self, timeout: Duration) -> Event {
        match self.events.recv_timeout(timeout) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => panic!("no event within {timeout:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the sink was dropped"),
        }
    }

    /// The next response; notices before it are kept.
    fn response(&mut self) -> OwnerResponse {
        loop {
            match self.event(PROMPT) {
                Event::Frame(Frame::OwnerResponse(r)) => return r,
                Event::Frame(Frame::OwnerNotice(n)) => self.notices.push((Instant::now(), n)),
                other => panic!("an unexpected event: {other:?}"),
            }
        }
    }

    fn ask(&mut self, value: &Value) -> OwnerResponse {
        self.send(value);
        self.response()
    }

    fn result(&mut self, value: &Value) -> OwnerResult {
        match self.ask(value) {
            OwnerResponse::Result { result, .. } => result,
            OwnerResponse::Error { error, .. } => panic!("{error:?}"),
        }
    }

    /// Waits for the first notice, kept or new, that `wanted` accepts.
    fn notice(&mut self, wanted: impl Fn(&OwnerNotice) -> bool) -> OwnerNotice {
        if let Some((_, n)) = self.notices.iter().find(|(_, n)| wanted(n)) {
            return n.clone();
        }
        let deadline = Instant::now() + STEP;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.event(left) {
                Event::Frame(Frame::OwnerNotice(n)) => {
                    self.notices.push((Instant::now(), n.clone()));
                    if wanted(&n) {
                        return n;
                    }
                }
                other => panic!("an unexpected event: {other:?}"),
            }
        }
    }

    /// Waits for request `request_id` to reach `state` and returns it.
    fn request_in(&mut self, request_id: &str, state: RequestState) -> Request {
        let n = self.notice(|n| is_request_in(n, request_id, state));
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

    /// The sequences of every notice read so far rise by one from the first.
    fn assert_consecutive(&self) {
        let sequences: Vec<u64> = self.notices.iter().map(|(_, n)| n.sequence.get()).collect();
        let first = sequences.first().copied().unwrap_or(1);
        let expected: Vec<u64> = (first..first + sequences.len() as u64).collect();
        assert_eq!(sequences, expected, "the notices do not rise by one");
    }

    /// The states request `request_id` went through, in the order of the notices.
    fn states_of(&self, request_id: &str) -> Vec<RequestState> {
        self.notices
            .iter()
            .filter_map(|(_, n)| match &n.change {
                Change::Request(r) if r.request_id.as_str() == request_id => Some(r.status.state()),
                _ => None,
            })
            .collect()
    }

    /// Waits for the session's end, skipping frames still in flight.
    fn ending(&self, timeout: Duration) -> Ending {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.event(left) {
                Event::Ended(ending) => return ending,
                Event::Frame(_) => {}
            }
        }
    }
}

fn is_request_in(notice: &OwnerNotice, request_id: &str, state: RequestState) -> bool {
    match &notice.change {
        Change::Request(r) => r.request_id.as_str() == request_id && r.status.state() == state,
        _ => false,
    }
}

fn request(id: &str, method: &str, params: Value) -> Value {
    json!({ "protocol": "fabric-desktop/1", "id": id, "method": method, "params": params })
}

fn hello() -> Value {
    request(
        "hello-1",
        "hello",
        json!({"client": {"name": "relay round trip", "build": "0"}, "schema_version": 2}),
    )
}

fn subscribe(cursor: u64) -> Value {
    request("sub-1", "subscribe", json!({ "cursor": cursor }))
}

fn decide(id: &str, read: &Request, decision: &str, duration: Value) -> Value {
    request(
        id,
        "decide_request",
        json!({
            "request_id": read.request_id,
            "request_digest": read.request_digest,
            "expected_revision": read.revision.get(),
            "decision": decision,
            "selected": null,
            "duration": duration,
        }),
    )
}

fn details(relay: &mut Relay, request_id: &str) -> RequestDetails {
    let ask = request(
        "details-1",
        "get_request_details",
        json!({"request_id": request_id, "redaction": "full"}),
    );
    match relay.result(&ask) {
        OwnerResult::GetRequestDetails(d) => d,
        other => panic!("{other:?}"),
    }
}

/// Checks a `hello` round trip through the relay.
fn echo(relay: &mut Relay) {
    let OwnerResult::Hello(result) = relay.result(&hello()) else {
        panic!("not a hello result")
    };
    assert_eq!(result.limits, Limits::CD_G0);
    assert_eq!(result.schema_version.get(), SCHEMA_VERSION);
    assert_eq!(result.broker.name.as_str(), "fabric-broker");
}

#[test]
fn echo_through_the_relay() {
    let mut relay = Relay::start();
    echo(&mut relay);
}

#[test]
fn notices_are_pushed_over_time() {
    let mut relay = Relay::start();
    let OwnerResult::GetSnapshot(snapshot) =
        relay.result(&request("snap-1", "get_snapshot", json!({})))
    else {
        panic!()
    };
    let cursor = snapshot.cursor.get();
    assert_eq!(cursor, 0);
    let OwnerResult::Subscribe(subscribed) = relay.result(&subscribe(cursor)) else {
        panic!()
    };
    assert_eq!(subscribed.cursor.get(), cursor);
    let pending = relay.pending(&[]);
    let request_id = pending.request_id.as_str().to_owned();
    relay.assert_consecutive();
    assert_eq!(relay.notices[0].1.sequence.get(), cursor + 1);
    let states = relay.states_of(&request_id);
    assert_eq!(
        states,
        [
            RequestState::Received,
            RequestState::Validating,
            RequestState::ProvingClassification,
            RequestState::PendingApproval
        ]
    );
    assert_eq!(first_invalid(&states), None);
    // The synthetic agent asks about a second after the page subscribed, and the account it
    // asks about came before its request.
    let kinds: Vec<&str> = relay.notices.iter().map(|(_, n)| n.change.kind()).collect();
    let account = kinds
        .iter()
        .position(|k| *k == "account")
        .expect("an account");
    let received = kinds
        .iter()
        .position(|k| *k == "request")
        .expect("a request");
    assert!(account < received, "{kinds:?}");
}

#[test]
fn approval_then_denial() {
    let mut relay = Relay::start();
    relay.result(&subscribe(0));
    let pending = relay.pending(&[]);
    let first = pending.request_id.as_str().to_owned();
    let read = details(&mut relay, &first);
    assert!(read.card.is_some() && read.private.is_some());

    let approve = decide(
        "decide-1",
        &read.request,
        "approve",
        json!({"kind": "one_request"}),
    );
    let OwnerResult::DecideRequest(decided) = relay.result(&approve) else {
        panic!()
    };
    let Approval::Approve(approval) = &decided.approval else {
        panic!("not an approval: {:?}", decided.approval)
    };
    assert_eq!(approval.command_id.as_str(), "decide-1");
    assert_eq!(decided.request.request_digest, pending.request_digest);
    let grant = relay.notice(|n| n.change.kind() == "grant");
    let Change::Grant(grant) = &grant.change else {
        panic!()
    };
    assert_eq!(grant.grant_id, approval.grant_id);
    let approved = relay.notice(|n| is_request_in(n, &first, RequestState::Approved));
    assert_eq!(decided.commit_sequence, approved.sequence);
    relay.request_in(&first, RequestState::Undecided);
    assert_eq!(first_invalid(&relay.states_of(&first)), None);

    let second = relay.pending(&[&first]);
    let second_id = second.request_id.as_str().to_owned();
    let read = details(&mut relay, &second_id);
    let deny = decide("decide-2", &read.request, "deny", Value::Null);
    let OwnerResult::DecideRequest(denied) = relay.result(&deny) else {
        panic!()
    };
    assert!(matches!(denied.approval, Approval::Deny(_)));
    assert_eq!(denied.request.status.state(), RequestState::Denied);
    relay.request_in(&second_id, RequestState::Denied);
    assert_eq!(first_invalid(&relay.states_of(&second_id)), None);
    relay.assert_consecutive();
}

#[test]
fn a_duplicate_command_is_answered_once() {
    let mut relay = Relay::start();
    relay.result(&subscribe(0));
    let pending = relay.pending(&[]);
    let request_id = pending.request_id.as_str().to_owned();
    let approve = decide("decide-1", &pending, "approve", json!({"kind": "session"}));
    relay.send(&approve);
    relay.send(&approve);
    let first = relay.response();
    let second = relay.response();
    assert_eq!(first, second);
    assert!(matches!(first, OwnerResponse::Result { .. }));
    relay.request_in(&request_id, RequestState::Undecided);
    let approvals = relay
        .notices
        .iter()
        .filter(|(_, n)| is_request_in(n, &request_id, RequestState::Approved))
        .count();
    assert_eq!(approvals, 1);
    let grants = relay
        .notices
        .iter()
        .filter(|(_, n)| n.change.kind() == "grant")
        .count();
    assert_eq!(grants, 1);

    let mut other = approve.clone();
    other["params"]["duration"] = json!({"kind": "until_revoked"});
    match relay.ask(&other) {
        OwnerResponse::Error { id, error, .. } => {
            assert_eq!(error.code(), ErrorCode::CommandReused);
            assert_eq!(
                id.map(|i| i.as_str().to_owned()).as_deref(),
                Some("decide-1")
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_refused_frame_never_reaches_the_broker() {
    let mut relay = Relay::start();
    for (frame, layer, code) in [
        (&b"{}"[..], Layer::Protocol, ErrorCode::UnsupportedProtocol),
        (
            br#"{"protocol":"fabric-desktop/1","id":"x","method":"teleport","params":{}}"#,
            Layer::Schema,
            ErrorCode::UnknownMethod,
        ),
        (b"not json", Layer::Frame, ErrorCode::MalformedFrame),
    ] {
        match relay.session.send(frame) {
            Err(SendError::Refused(refusal)) => {
                assert_eq!((refusal.layer, refusal.code), (layer, code));
            }
            other => panic!("{other:?}"),
        }
    }
    relay.send(&hello());
    match relay.response() {
        OwnerResponse::Result { id, result } => {
            assert_eq!(id.as_str(), "hello-1");
            assert!(matches!(result, OwnerResult::Hello(_)));
        }
        other => panic!("the broker answered something else first: {other:?}"),
    }
    // And it is the only one: nothing else arrives, and no notice came before it.
    assert!(relay.notices.is_empty());
    match relay.events.recv_timeout(Duration::from_millis(300)) {
        Err(RecvTimeoutError::Timeout) => {}
        other => panic!("another event arrived: {other:?}"),
    }
}

/// Kills the process `pid` from outside, as the system or the owner might.
fn kill_from_outside(pid: u32) {
    #[cfg(unix)]
    let mut command = std::process::Command::new("kill");
    #[cfg(unix)]
    command.args(["-KILL", &pid.to_string()]);
    #[cfg(windows)]
    let mut command = std::process::Command::new("taskkill");
    #[cfg(windows)]
    command
        .args(["/F", "/PID", &pid.to_string()])
        .stdout(std::process::Stdio::null());
    let spawning = spawn_lock();
    let child = command.spawn();
    drop(spawning);
    let status = child
        .and_then(|mut c| c.wait())
        .expect("the kill command runs");
    assert!(status.success(), "the kill command failed: {status:?}");
}

#[test]
fn the_death_of_the_broker_ends_the_session() {
    let mut relay = Relay::start();
    echo(&mut relay);
    kill_from_outside(relay.session.pid());
    match relay.ending(PROMPT) {
        Ending::Exited { code, description } => {
            #[cfg(unix)]
            {
                assert_eq!(code, None);
                assert_eq!(description, "killed by signal 9");
            }
            #[cfg(not(unix))]
            {
                let _ = (code, description);
            }
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        relay.session.send(canonical(&hello()).as_bytes()),
        Err(SendError::NotRunning)
    );
}

#[test]
fn quit_stops_the_broker_without_a_kill() {
    let mut relay = Relay::start();
    echo(&mut relay);
    // While the synthetic agent runs, so that the broker has timers pending.
    relay.result(&subscribe(0));
    relay.notice(|n| n.change.kind() == "account");
    // Only `stop` is timed: the broker has answered, so its launch is behind it, and `stop`
    // returns when the child is gone.
    let started = Instant::now();
    relay.session.stop(QUIT_DRAIN);
    let took = started.elapsed();
    assert!(took < QUIT_DRAIN / 2, "stopping took {took:?}");
    assert_eq!(relay.ending(PROMPT), Ending::Stopped { killed: false });
    assert_eq!(
        relay.session.send(canonical(&hello()).as_bytes()),
        Err(SendError::NotRunning)
    );
}

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// A fresh directory under the system's temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{name} {} {nanos} {n}", std::process::id()));
        std::fs::create_dir(&path).expect("a fresh temporary directory");
        TempDir(path)
    }

    /// The broker's data directory: a child that the store creates.
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_broker_in_a_directory_with_spaces() {
    let dir = TempDir::new("fabric broker with spaces");
    let copy = dir
        .0
        .join(format!("fabric-broker{}", std::env::consts::EXE_SUFFIX));
    // `fs::copy` keeps the permission bits, so the copy stays executable on Unix.
    std::fs::copy(broker_path(), &copy).expect("the broker can be copied");
    assert!(copy.is_absolute() && copy.to_string_lossy().contains(' '));
    let mut relay = Relay::start_at(&copy);
    echo(&mut relay);
    relay.session.stop(QUIT_DRAIN);
    assert_eq!(relay.ending(PROMPT), Ending::Stopped { killed: false });
}

/// A directory laid out as the three Isabelle variables need it: `bin/isabelle` a script that
/// exits 1 at once, an empty user home, and the repository's rule theory. No Isabelle runs from
/// it, so the prover of a broker given these paths fails to start.
struct FakeIsabelle {
    dir: TempDir,
}

impl FakeIsabelle {
    fn new() -> FakeIsabelle {
        let dir = TempDir::new("fake isabelle");
        let bin = dir.0.join("isabelle").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(dir.0.join("user")).unwrap();
        let program = bin.join("isabelle");
        std::fs::write(&program, b"#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakeIsabelle { dir }
    }

    /// The three variables with this layout's paths.
    fn variables(&self) -> Vec<(OsString, OsString)> {
        let rules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../isabelle");
        vec![
            (
                OsString::from("FABRIC_BROKER_ISABELLE_HOME"),
                self.dir.0.join("isabelle").into_os_string(),
            ),
            (
                OsString::from("FABRIC_BROKER_ISABELLE_USER_HOME"),
                self.dir.0.join("user").into_os_string(),
            ),
            (
                OsString::from("FABRIC_BROKER_RULES_DIR"),
                rules.into_os_string(),
            ),
        ]
    }
}

/// PW p43: the relay passes the three Isabelle variables to the broker: with them and the
/// synthetic agent, the broker names a practice build, which it does only when it read all
/// three.
#[test]
fn the_relay_passes_the_three_isabelle_names() {
    let isabelle = FakeIsabelle::new();
    let mut relay = Relay::start_with(&broker_path(), isabelle.variables());
    let OwnerResult::Hello(result) = relay.result(&hello()) else {
        panic!("not a hello result")
    };
    assert_eq!(
        result.broker.build.as_str(),
        format!("{} CD-W04 synthetic practice", env!("CARGO_PKG_VERSION"))
    );
}
