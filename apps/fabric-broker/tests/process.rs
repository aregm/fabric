//! Process tests of the built `fabric-broker` executable, with plain `std::process`.
//!
//! The broker under test is `FABRIC_BROKER_UNDER_TEST` when it is set (an absolute path, for
//! example the broker inside a packaged app) and otherwise the binary Cargo built. Every line
//! the broker writes must decode as an owner response or an owner notice; the helper fails the
//! test otherwise.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use fabric_desktop_contracts::error::{ErrorCode, Fault};
use fabric_desktop_contracts::owner::{
    Approval, Change, GrantTerm, OwnerNotice, OwnerResponse, OwnerResult, Request,
};
use fabric_desktop_contracts::{
    Entry, Frame, MAX_FRAME_BYTES, RequestState, canonical, decode, first_invalid, parse_strict,
};
use serde_json::{Value, json};

/// How long to wait for a frame that is due at once. The first answer of a broker also waits
/// for its launch, which for a newly signed executable on macOS can take seconds.
const PROMPT: Duration = Duration::from_secs(10);
/// How long to wait for a script step, beyond its nominal time.
const STEP: Duration = Duration::from_secs(6);
/// The bound on stopping after end of input, for a broker that has already answered.
const STOP_BOUND: Duration = Duration::from_millis(1000);
/// How long a broker may take to exit when nothing showed that it had started yet.
const LAUNCH_AND_EXIT: Duration = Duration::from_secs(10);

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
    frame
}

struct Broker {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Vec<u8>>,
    /// Notices read while waiting for a response.
    notices: Vec<OwnerNotice>,
}

impl Broker {
    fn start() -> Broker {
        let mut child = spawn(
            Command::new(broker_path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        );
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
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
            notices: Vec::new(),
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

    fn next(&self, timeout: Duration) -> Written {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Written {
                frame: decode_line(&line),
                line,
            },
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

    /// Closes stdin and waits for the exit; the time it took and every frame still written.
    fn finish(mut self) -> (ExitStatus, Duration, Vec<Written>) {
        drop(self.stdin.take());
        let closed = Instant::now();
        let status = wait_with_bound(&mut self.child, LAUNCH_AND_EXIT);
        let took = closed.elapsed();
        let mut rest = Vec::new();
        while let Ok(line) = self.lines.recv_timeout(PROMPT) {
            rest.push(Written {
                frame: decode_line(&line),
                line,
            });
        }
        (status, took, rest)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

#[test]
fn hello_reports_the_limits_of_the_contract() {
    let mut broker = Broker::start();
    let OwnerResult::Hello(result) = broker.result(&hello(1)) else {
        panic!()
    };
    let text = std::fs::read_to_string(contract_dir().join("data/limits.json")).unwrap();
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        canonical(&serde_json::to_value(result.limits).unwrap()),
        canonical(&file["limits"])
    );
    assert_eq!(result.broker.name.as_str(), "fabric-broker-spike");
    assert_eq!(
        result.broker.build.as_str(),
        format!("{} CD-W02 spike", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(result.schema_version.get(), 1);
    assert!(result.capabilities.is_empty());
}

#[test]
fn hello_with_another_schema_version_is_refused() {
    let mut broker = Broker::start();
    let response = broker.ask(&hello(2));
    assert_eq!(
        error_of(&response),
        (
            ErrorCode::UnsupportedSchemaVersion,
            Some("hello-1".into()),
            Some("hello".into()),
            json!({"received": 2, "supported": 1})
        )
    );
}

#[test]
fn the_first_run_snapshot() {
    let mut broker = Broker::start();
    let OwnerResult::GetSnapshot(snapshot) =
        broker.result(&request("snap-1", "get_snapshot", json!({})))
    else {
        panic!()
    };
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(value["cursor"], 0);
    assert_eq!(
        value["health"],
        json!({"prover": "unavailable", "store": "locked"})
    );
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

/// The script up to the first card, then an approval: the grant, the approval, the duplicate
/// command and the reused command id, and the end of the request in `undecided`.
#[test]
fn the_script_and_an_approval() {
    let mut broker = Broker::start();
    let OwnerResult::Subscribe(subscribed) =
        broker.result(&request("sub-1", "subscribe", json!({"cursor": 0})))
    else {
        panic!()
    };
    assert_eq!(subscribed.cursor.get(), 0);
    let pending = broker.request_in("spike-request-1", RequestState::PendingApproval);
    let first_six: Vec<&OwnerNotice> = broker.notices.iter().collect();
    let sequences: Vec<u64> = first_six.iter().map(|n| n.sequence.get()).collect();
    assert_eq!(sequences, [1, 2, 3, 4, 5, 6]);
    let kinds: Vec<&str> = first_six.iter().map(|n| n.change.kind()).collect();
    assert_eq!(
        kinds,
        [
            "agent", "account", "request", "request", "request", "request"
        ]
    );
    assert_eq!(
        states_of(&broker.notices, "spike-request-1"),
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
        json!({"request_id": "spike-request-1", "redaction": "full"}),
    );
    let OwnerResult::GetRequestDetails(details) = broker.result(&details) else {
        panic!()
    };
    let card = details.card.expect("a pending request has a card");
    assert_eq!(card.insufficiency.rule.as_str(), "spike_no_rule_theory");
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
    let grant = broker.notice(|n| n.change.kind() == "grant");
    assert_eq!(grant.sequence.get(), 7);
    let Change::Grant(grant) = grant.change else {
        panic!()
    };
    assert_eq!(grant.grant_id.as_str(), "spike-grant-1");
    assert_eq!(
        grant.term,
        GrantTerm::OneRequest {
            action_digest: action.digest()
        }
    );
    let approved = broker.request_in("spike-request-1", RequestState::Approved);
    assert_eq!(decided.request, approved);
    assert_eq!(decided.commit_sequence.get(), 8);
    let Approval::Approve(approval) = &decided.approval else {
        panic!()
    };
    assert_eq!(approval.command_id.as_str(), "decide-1");
    assert_eq!(approval.grant_id, grant.grant_id);

    // The same command again: the same bytes, and no notice.
    broker.send(&approve);
    let (_, again) = broker.response();
    assert_eq!(again, first_answer);
    // The same id with a different command.
    let mut other = approve.clone();
    other["params"]["duration"] = json!({"kind": "session"});
    let response = broker.ask(&other);
    assert_eq!(error_of(&response).0, ErrorCode::CommandReused);

    broker.request_in("spike-request-1", RequestState::Undecided);
    let states = states_of(&broker.notices, "spike-request-1");
    assert_eq!(first_invalid(&states), None);
    assert_eq!(
        &states[4..],
        [
            RequestState::Approved,
            RequestState::ProvingFreshPermit,
            RequestState::Undecided
        ]
    );
    let grants = broker.notices.iter().filter(|n| n.change.kind() == "grant");
    assert_eq!(grants.count(), 1);
    let OwnerResult::GetSnapshot(snapshot) =
        broker.result(&request("snap-1", "get_snapshot", json!({})))
    else {
        panic!()
    };
    assert!(snapshot.requests.is_empty());
    assert_eq!(snapshot.grants.len(), 1);
    assert_eq!(snapshot.cursor.get(), 10);
}

/// The refusals of `decide_request`, then a denial, then the second request.
#[test]
fn refused_decisions_and_a_denial() {
    let mut broker = Broker::start();
    broker.result(&request("sub-1", "subscribe", json!({"cursor": 0})));
    let pending = broker.request_in("spike-request-1", RequestState::PendingApproval);

    let deny = decide(&pending, "1", "deny", Value::Null);
    let mut stale_revision = deny.clone();
    stale_revision["id"] = json!("stale-revision");
    stale_revision["params"]["expected_revision"] = json!(3);
    let mut stale_digest = deny.clone();
    stale_digest["id"] = json!("stale-digest");
    stale_digest["params"]["request_digest"] = json!(format!("sha256:{}", "a".repeat(64)));
    let mut unknown = deny.clone();
    unknown["id"] = json!("unknown");
    unknown["params"]["request_id"] = json!("spike-request-99");
    let mut narrowed = decide(&pending, "narrowed", "approve", json!({"kind": "session"}));
    narrowed["params"]["selected"] = json!({
        "calendars": [{"account_id": "spike-account", "calendar_id": "spike-calendar"}],
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
    let denied = broker.request_in("spike-request-1", RequestState::Denied);
    assert_eq!(decided.commit_sequence.get(), 7);
    assert_eq!(decided.request, denied);

    let received = broker.request_in("spike-request-2", RequestState::Received);
    assert_eq!(received.tool.as_str(), "fabric_calendar_propose_invitation");
    let states = states_of(&broker.notices, "spike-request-1");
    assert_eq!(first_invalid(&states), None);
    assert!(broker.notices.iter().all(|n| n.change.kind() != "grant"));
}

#[test]
fn the_eight_methods_the_spike_does_not_implement() {
    let unimplemented = [
        "connect_google",
        "save_profile",
        "save_preferences",
        "save_group",
        "pause_agent",
        "revoke_grant",
        "get_proof",
        "disconnect_google",
    ];
    let fixtures = contract_dir().join("fixtures/messages");
    let mut frames = Vec::new();
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("accept-owner-request-") {
            continue;
        }
        let fixture: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        frames.push(fixture["value"].clone());
    }
    let mut broker = Broker::start();
    for method in unimplemented {
        let frame = frames
            .iter()
            .find(|f| f["method"] == method)
            .unwrap_or_else(|| panic!("no fixture for {method}"));
        let response = broker.ask(frame);
        assert_eq!(
            error_of(&response),
            (
                ErrorCode::UnknownMethod,
                frame["id"].as_str().map(str::to_owned),
                Some(method.to_owned()),
                Value::Null
            )
        );
        let OwnerResponse::Error { error, .. } = &response else {
            panic!()
        };
        assert_eq!(
            error.message.as_str(),
            format!("The spike broker of CD-W02 does not implement {method}.")
        );
    }
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
        broker.ask(&hello(1)),
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
        broker.ask(&hello(1)),
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
        let OwnerResponse::Result { id, .. } = broker.ask(&hello(1)) else {
            panic!()
        };
        assert_eq!(id.as_str(), "hello-1");
    }
}

#[test]
fn wrong_protocol() {
    let mut broker = Broker::start();
    let mut frame = hello(1);
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
    // stdin stays open and empty: the broker must exit without reading it.
    let mut child = spawn(
        Command::new(broker_path())
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
}

/// End of input stops the broker at once, with exit code 0, while the script is running; and
/// everything it wrote is a frame.
#[test]
fn end_of_input_stops_the_broker_and_stdout_holds_only_frames() {
    let mut broker = Broker::start();
    broker.result(&hello(1));
    broker.result(&request("sub-1", "subscribe", json!({"cursor": 0})));
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
    let mut child = spawn(
        Command::new(broker_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    );
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    let mut line = canonical(&hello(1)).into_bytes();
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
