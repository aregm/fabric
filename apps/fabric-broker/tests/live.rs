//! Live proofs of the broker with the real Isabelle: never run in CI (RD-7);
//! each test is `#[ignore]` and runs by name through `sh scripts/verify-desktop.sh
//! --live-proofs` on a developer's Mac, or with `cargo test -p fabric-broker --test live --
//! --ignored --test-threads 1`.
//!
//! Isabelle is where the three variables `FABRIC_BROKER_ISABELLE_HOME`,
//! `FABRIC_BROKER_ISABELLE_USER_HOME` and `FABRIC_BROKER_RULES_DIR` say when all three are set,
//! and otherwise in the repository's development layout (`.tooling/isabelle`, installed by hand
//! with `sh scripts/setup-demo.sh`). Every test takes the `LIVE` lock, uses the public test key
//! and a temporary data directory of its own (hence an Isabelle server of its own), and never
//! touches the Keychain.
//!
//! The in-process tests run the same code as the executable: `Plan`, `Shell::new`,
//! `shell::next_event`, `shell::input_of`, `Shell::turn` and `Shell::stop`, with the practice
//! calendar and the synthetic agent's practice script. The test plays the owner: it writes owner
//! frames as `Input::OwnerLine` and reads the frames the shell wrote. Other tests run the built
//! broker. Every test is a top-level function of this file, so its name is its full test path.
//!
//! No test here gives the core a proof it did not get from Isabelle.
//!
//! Run them on an otherwise idle Mac: the in-process stop must be acknowledged within
//! `QUIT_BOUND` (1.5 s); the worker gives Isabelle at most 1.2 s to stop and then kills its
//! server, and a failing assertion names the time it took.
//!
//! The test ids PW L1 to PW L22 are those of section 7.3 of the prover wiring's design,
//! `docs/plans/calendar-desktop-prover-wiring-design.md`, not the CD-W04 design's package decisions
//! L1 to L7; a bare DEF-n, D40 or p10 cites the CD-W04 design
//! (`docs/plans/calendar-desktop-w04-design.md`).
//!
//! One test, p10, drives the authority core in process through `tests/support` and asks the
//! gate itself, with no shell: the duration a card proposes against the rule theory.

#![cfg(all(unix, debug_assertions))]

mod support;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use fabric_broker::config::{
    Config, ISABELLE_HOME, ISABELLE_USER_HOME, IsabelleSettings, KeySource, RULES_DIR,
};
use fabric_broker::practice::{FOREIGN_EVENT_ID, FOREIGN_TITLE, Faults, PracticeCalendar};
use fabric_broker::prover::map::context_of;
use fabric_broker::prover::{
    ProverOptions, QUIT_BOUND, built_theory_sha256, gate_config, server_name,
};
use fabric_broker::shell::{self, Event, Plan, ProverChoice, QUEUED_EVENTS, Shell, now_ms};
use fabric_broker::synthetic::{ACCOUNT_ID, AGENT_ID, CALENDAR_ID, SyntheticAgent, TITLE};
use fabric_broker_core::ports::{ObservedEvent, ProofJob};
use fabric_broker_core::{Broker, Input, Settings};
use fabric_broker_policy::{BrokerGate, GateConfig, Outcome};
use fabric_desktop_contracts::common::CalendarRef;
use fabric_desktop_contracts::owner::{
    Change, GrantTerm, Health, LifecycleStatus, OwnerNotice, OwnerResponse, OwnerResult,
    ProfileRows, ProverHealth, ProverProblem, Request,
};
use fabric_desktop_contracts::{Entry, Frame, RequestState, canonical, decode};
use fabric_desktop_relay::{BrokerConfig, ENVIRONMENT_ALLOWLIST};
use serde_json::{Value, json};

/// How long a first start may take: Isabelle may build the rule theory's heap first.
const START: Duration = Duration::from_secs(600);
/// How long one step of the practice script may take: a proof or two and the agent's pace.
const STEP: Duration = Duration::from_secs(90);
/// The relay's drain at quit (`fabric_desktop_relay::QUIT_DRAIN`).
const DRAIN: Duration = Duration::from_secs(2);
/// The marker in the synthetic agent's event title.
const TITLE_MARKER: &str = "5f1c";
/// The rule that blocks the fresh proof of a request whose grant was revoked.
const REVOKED_RULE: &str = "relied_grant_live";
/// The rule that blocks the fresh proof of a request whose agent was paused.
const PAUSED_RULE: &str = "agent_not_paused";

/// One live test at a time: they share Isabelle's user home and its server registry.
static LIVE: Mutex<()> = Mutex::new(());

fn live_lock() -> MutexGuard<'static, ()> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where Isabelle and the rule theory are: the three variables when all are set, else the
/// repository's development layout. A partial set, or no Isabelle there, fails the test.
fn isabelle() -> IsabelleSettings {
    let values: Vec<Option<OsString>> = [ISABELLE_HOME, ISABELLE_USER_HOME, RULES_DIR]
        .iter()
        .map(std::env::var_os)
        .collect();
    let settings = match values.as_slice() {
        [None, None, None] => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("the repository root");
            let gate = GateConfig::for_repository(&root);
            IsabelleSettings {
                isabelle_home: gate.isabelle_home,
                user_home: gate.user_home,
                rules_dir: gate.rules_dir,
            }
        }
        [Some(home), Some(user), Some(rules)] => IsabelleSettings {
            isabelle_home: PathBuf::from(home),
            user_home: PathBuf::from(user),
            rules_dir: PathBuf::from(rules),
        },
        _ => panic!(
            "set {ISABELLE_HOME}, {ISABELLE_USER_HOME} and {RULES_DIR} together, or none for the repository's layout; run sh scripts/setup-demo.sh by hand"
        ),
    };
    assert!(
        settings
            .isabelle_home
            .join("bin")
            .join("isabelle")
            .is_file(),
        "no Isabelle at {}: run sh scripts/setup-demo.sh by hand",
        settings.isabelle_home.display()
    );
    settings
}

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// A fresh directory under the system's temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> TempDir {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fabric-broker-live-{label}-{}-{n}",
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

/// The process ids whose command line names `server`: the Isabelle server of one data
/// directory.
fn servers(server: &str) -> Vec<u32> {
    let output = Command::new("pgrep")
        .arg("-f")
        .arg(server)
        .output()
        .expect("pgrep runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

/// Waits up to `bound` until no process names `server`.
fn servers_gone(server: &str, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if servers(server).is_empty() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut open = vec![dir.to_path_buf()];
    while let Some(dir) = open.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => open.push(path),
                Ok(kind) if kind.is_file() => files.push(path),
                _ => {}
            }
        }
    }
    files
}

/// The calendar data of the practice run that a check theory may carry, and the titles that
/// never reach Isabelle: the synthetic agent's title and the foreign event's, the account,
/// calendar and agent ids, and each of `event_ids` both as written and as the byte list a check
/// theory writes it in.
fn calendar_text(event_ids: &[&str]) -> Vec<Vec<u8>> {
    let mut needles: Vec<Vec<u8>> = [TITLE, FOREIGN_TITLE, ACCOUNT_ID, CALENDAR_ID, AGENT_ID]
        .iter()
        .map(|text| text.as_bytes().to_vec())
        .collect();
    for id in event_ids {
        needles.push(id.as_bytes().to_vec());
        let bytes: Vec<String> = id.bytes().map(|b| b.to_string()).collect();
        needles.push(format!("[{}]", bytes.join(", ")).into_bytes());
    }
    needles
}

/// The files under `dir` changed since `since` that hold any of `needles`.
fn calendar_text_under(dir: &Path, since: SystemTime, needles: &[Vec<u8>]) -> Vec<PathBuf> {
    files_under(dir)
        .into_iter()
        .filter(|path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified >= since)
        })
        .filter(|path| {
            std::fs::read(path)
                .is_ok_and(|bytes| needles.iter().any(|needle| contains(&bytes, needle)))
        })
        .collect()
}

/// Isabelle's directory for temporary files (`ISABELLE_TMP_PREFIX`), as the gate's server sees
/// it: the test's environment with the gate's `USER_HOME`.
fn isabelle_tmp_prefix(isabelle: &IsabelleSettings) -> PathBuf {
    let output = Command::new(isabelle.isabelle_home.join("bin").join("isabelle"))
        .args(["getenv", "-b", "ISABELLE_TMP_PREFIX"])
        .env("USER_HOME", &isabelle.user_home)
        .output()
        .expect("isabelle getenv runs");
    assert!(output.status.success(), "isabelle getenv failed");
    let prefix = String::from_utf8(output.stdout).expect("a UTF-8 path");
    PathBuf::from(prefix.trim())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn owner_line(id: &str, method: &str, params: Value) -> Vec<u8> {
    canonical(
        &json!({ "protocol": "fabric-desktop/1", "id": id, "method": method, "params": params }),
    )
    .into_bytes()
}

/// Decodes one line the broker wrote, without its line feed, or fails the test.
fn decode_line(bytes: &[u8]) -> (Frame, Value) {
    let value: Value = serde_json::from_slice(bytes).expect("a JSON line");
    let entry = if value.get("sequence").is_some() {
        Entry::OwnerNotice
    } else {
        Entry::OwnerResponse
    };
    let frame = decode(entry, bytes)
        .unwrap_or_else(|r| panic!("not a frame ({r}): {}", String::from_utf8_lossy(bytes)));
    assert_cross_member_rules(&frame, &value);
    (frame, value)
}

/// The two rules between members of contract schema version 2 (README section 7), as in
/// the process tests (PW p48): a health names a problem only while the prover is unavailable, and
/// request details name an `undecided_reason` only while the request is `undecided`.
fn assert_cross_member_rules(frame: &Frame, value: &Value) {
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
                "a reason on a request that is not undecided: {value}"
            );
            None
        }
        _ => None,
    };
    if let Some(health) = health {
        assert!(
            health.prover == ProverHealth::Unavailable || health.prover_problem.is_none(),
            "a problem while the prover is not unavailable: {value}"
        );
    }
}

/// The in-process broker of one live test: a practice run with the real Isabelle.
struct Live {
    /// Taken by `stop`.
    shell: Option<Shell<Vec<u8>>>,
    broker: Broker,
    events: Receiver<Event>,
    calendar: PracticeCalendar,
    isabelle: IsabelleSettings,
    server: String,
    /// Bytes of the shell's stdout already read.
    read: usize,
    notices: Vec<OwnerNotice>,
    /// Every response, as JSON, in order.
    responses: Vec<Value>,
    /// The health the owner was told in notices, in order: the prover's and, while it is
    /// unavailable, why.
    healths: Vec<Health>,
    next_id: u64,
    began: Instant,
    began_at: SystemTime,
    dir: TempDir,
}

impl Live {
    /// A practice run on a new data directory: the real Isabelle, the practice calendar with
    /// `faults`, and the synthetic agent's practice script. `tune` may change the prover's
    /// options; it still starts the real gate, pinned to the built rule theory.
    fn start(label: &str, faults: Faults, tune: impl FnOnce(&mut ProverOptions)) -> Live {
        let dir = TempDir::new(label);
        let isabelle = isabelle();
        let config = Config {
            data_dir: dir.data(),
            key_source: KeySource::Test,
            synthetic_agent: true,
            isabelle: Some(isabelle.clone()),
        };
        assert!(config.practice());
        let mut options = ProverOptions::new(&isabelle, &config.data_dir);
        tune(&mut options);
        let server = options.gate.server_name.clone();
        assert_eq!(server, server_name(&config.data_dir));
        let calendar = PracticeCalendar::new(now_ms(), faults);
        let plan = Plan {
            prover: ProverChoice::Isabelle(Box::new(options)),
            work_dir: config.data_dir.join("prover"),
            practice: Some(calendar.clone()),
            agent: Some(SyntheticAgent::practice(calendar.clone())),
        };
        let (broker, started) = Broker::start(
            shell::opener(&config),
            Settings::cd_w04(shell::build_label(&config)),
            plan.attachments(),
            now_ms(),
        )
        .unwrap_or_else(|e| panic!("the broker starts: {e}"));
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let shell = Shell::new(Vec::new(), plan, sender);
        let mut live = Live {
            shell: Some(shell),
            broker,
            events,
            calendar,
            isabelle,
            server,
            read: 0,
            notices: Vec::new(),
            responses: Vec::new(),
            healths: Vec::new(),
            next_id: 0,
            began: Instant::now(),
            began_at: SystemTime::now(),
            dir,
        };
        let shell = live.shell.as_mut().expect("the shell");
        shell
            .emit(&live.broker, started)
            .unwrap_or_else(|_| panic!("the start was written"));
        live.read_frames();
        live
    }

    fn data(&self) -> PathBuf {
        self.dir.data()
    }

    fn work_dir(&self) -> PathBuf {
        self.data().join("prover")
    }

    /// One turn of the shell with `input`, then the frames it wrote.
    fn turn(&mut self, input: Input) {
        let shell = self.shell.as_mut().expect("the shell runs");
        shell
            .turn(&mut self.broker, input, now_ms())
            .unwrap_or_else(|_| panic!("the turn ended the run"));
        self.read_frames();
    }

    /// One pass of the main loop, waiting at most `wait` for an event.
    fn step(&mut self, wait: Duration) {
        let shell = self.shell.as_ref().expect("the shell runs");
        let cap = now_ms() + u64::try_from(wait.as_millis()).unwrap_or(u64::MAX);
        let due = shell
            .next_due_ms(&self.broker)
            .map_or(cap, |due| due.min(cap));
        let input = match shell::next_event(&self.events, Some(due)) {
            None => Input::Tick,
            Some(event) => shell::input_of(event).expect("no end of input in a live test"),
        };
        self.turn(input);
    }

    /// Runs the main loop until `found` answers, for up to `bound`.
    fn until<T>(
        &mut self,
        what: &str,
        bound: Duration,
        mut found: impl FnMut(&mut Live) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + bound;
        loop {
            if let Some(answer) = found(self) {
                return answer;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: not within {bound:?}; prover health so far {:?}",
                self.healths
            );
            self.step(Duration::from_millis(200));
        }
    }

    /// Runs the main loop for `time`.
    fn run_for(&mut self, time: Duration) {
        let deadline = Instant::now() + time;
        while Instant::now() < deadline {
            self.step(Duration::from_millis(100));
        }
    }

    fn read_frames(&mut self) {
        let shell = self.shell.as_mut().expect("the shell runs");
        let stdout = shell.stdout_mut();
        let fresh = stdout[self.read..].to_vec();
        let complete = fresh.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        self.read += complete;
        for line in fresh[..complete]
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
        {
            match decode_line(line) {
                (Frame::OwnerNotice(notice), _) => {
                    if let Change::Health(health) = &notice.change {
                        self.healths.push(*health);
                    }
                    self.notices.push(notice);
                }
                (Frame::OwnerResponse(_), value) => self.responses.push(value),
                (other, _) => panic!("an unexpected frame: {other:?}"),
            }
        }
    }

    /// Sends an owner frame and returns its response, written in the same turn.
    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = format!("live-{}", self.next_id);
        let before = self.responses.len();
        self.turn(Input::OwnerLine(owner_line(&id, method, params)));
        let response = self.responses[before..]
            .iter()
            .find(|r| r["id"] == json!(id))
            .cloned();
        response.unwrap_or_else(|| panic!("no response to {method}"))
    }

    /// The result of an owner frame that must succeed.
    fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.ask(method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }

    fn snapshot(&mut self) -> Value {
        self.result("get_snapshot", json!({}))
    }

    fn details(&mut self, request_id: &str) -> Value {
        self.result(
            "get_request_details",
            json!({"request_id": request_id, "redaction": "redacted"}),
        )
    }

    fn proof(&mut self, proof_id: &str) -> Value {
        self.result("get_proof", json!({"proof_id": proof_id}))
    }

    /// Subscribes from the current cursor, which lets the synthetic agent start.
    fn subscribe(&mut self) {
        let cursor = self.snapshot()["cursor"].clone();
        self.result("subscribe", json!({"cursor": cursor}));
    }

    /// Waits until the owner was told the prover's health is `health`; returns when.
    fn health_becomes(&mut self, health: ProverHealth, bound: Duration) -> Duration {
        let seen = self.healths.len();
        self.until(&format!("prover health {health:?}"), bound, |live| {
            live.provers(seen).contains(&health).then_some(())
        });
        self.began.elapsed()
    }

    /// The prover's health in the notices from the `from`-th on.
    fn provers(&self, from: usize) -> Vec<ProverHealth> {
        self.healths[from..].iter().map(|h| h.prover).collect()
    }

    /// The last notice of request `request_id`, if any.
    fn last_of(&self, request_id: &str) -> Option<Request> {
        self.notices.iter().rev().find_map(|n| match &n.change {
            Change::Request(r) if r.request_id.as_str() == request_id => Some(r.clone()),
            _ => None,
        })
    }

    /// Waits for request `request_id` to be in `state`.
    fn request_in(&mut self, request_id: &str, state: RequestState, bound: Duration) -> Request {
        self.until(&format!("{request_id} {state:?}"), bound, |live| {
            live.last_of(request_id)
                .filter(|r| r.status.state() == state)
        })
    }

    /// Waits for a request other than `except` whose last notice `wanted` accepts.
    fn request_where(
        &mut self,
        what: &str,
        except: &[&str],
        bound: Duration,
        wanted: impl Fn(&Request) -> bool,
    ) -> Request {
        self.until(what, bound, |live| {
            let mut ids: Vec<String> = Vec::new();
            for n in &live.notices {
                if let Change::Request(r) = &n.change
                    && !ids.contains(&r.request_id.as_str().to_owned())
                {
                    ids.push(r.request_id.as_str().to_owned());
                }
            }
            ids.iter()
                .filter(|id| !except.contains(&id.as_str()))
                .filter_map(|id| live.last_of(id))
                .find(|r| wanted(r))
        })
    }

    /// Waits for a new request in `pending_approval`.
    fn pending(&mut self, except: &[&str]) -> Request {
        self.request_where("a request waiting for the owner", except, STEP, |r| {
            r.status.state() == RequestState::PendingApproval
        })
    }

    /// Waits until every request is settled, runs the loop for three seconds more (longer than
    /// the agent's pace and one check), and waits again, so no check runs when the test stops.
    fn settle_all(&mut self) {
        let all_settled = |live: &mut Live| {
            live.notices
                .iter()
                .filter_map(|n| match &n.change {
                    Change::Request(r) => Some(r.request_id.as_str().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .iter()
                .all(|id| live.last_of(id).is_some_and(|r| settled(r.status.state())))
                .then_some(())
        };
        self.until("every request settles", STEP, all_settled);
        self.run_for(Duration::from_secs(3));
        self.until("every request settles again", STEP, all_settled);
    }

    /// The proof reports of request `request_id`, in the order the store lists them.
    fn proofs_of(&mut self, request_id: &str) -> Vec<Value> {
        let details = self.details(request_id);
        let ids: Vec<String> = details["proof_ids"]
            .as_array()
            .expect("proof ids")
            .iter()
            .filter_map(|id| id.as_str().map(ToOwned::to_owned))
            .collect();
        ids.iter().map(|id| self.proof(id)).collect()
    }

    /// The ids of every request the owner was told of, in the order they first appeared.
    fn request_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for notice in &self.notices {
            if let Change::Request(r) = &notice.change
                && !ids.contains(&r.request_id.as_str().to_owned())
            {
                ids.push(r.request_id.as_str().to_owned());
            }
        }
        ids
    }

    /// Approves `request` with `duration`; returns the decision's result.
    fn approve(&mut self, request: &Request, duration: Value) -> Value {
        self.result(
            "decide_request",
            json!({
                "request_id": request.request_id,
                "request_digest": request.request_digest,
                "expected_revision": request.revision.get(),
                "decision": "approve",
                "selected": null,
                "duration": duration,
            }),
        )
    }

    /// Checks a stored proof report: Isabelle's, on the built rule theory, with 18 rule
    /// verdicts. Returns it.
    fn isabelle_proof(&mut self, proof_id: &str, outcome: &str) -> Value {
        let report = self.proof(proof_id);
        assert_eq!(report["outcome"], outcome, "{report}");
        assert_eq!(report["isabelle_version"], "Isabelle2025-2", "{report}");
        assert_eq!(
            report["theory_hash"],
            json!(format!("sha256:{}", built_theory_sha256())),
            "{report}"
        );
        assert_eq!(
            report["rule_verdicts"].as_array().map(Vec::len),
            Some(18),
            "{report}"
        );
        report
    }

    /// Ends the run as main does: quit, then the shell's stop. Returns whether the worker
    /// acknowledged and how long the stop took, which it prints. Without an acknowledgement the
    /// worker was late: a kill (the gate's at its bound, or the kill handle's of a check still
    /// running at the stop's instant) or what follows it took longer than `QUIT_BOUND`, as on a
    /// busy Mac, or the kill failed (PW K1); it waits for the server to go, so the server does
    /// not outlive the test process.
    fn stop(&mut self) -> (bool, Duration) {
        let mut shell = self.shell.take().expect("the shell runs");
        shell
            .quit(&mut self.broker, now_ms())
            .unwrap_or_else(|_| panic!("quit was written"));
        let began = Instant::now();
        let acknowledged = shell.stop(&self.events);
        let took = began.elapsed();
        println!(
            "LIVE stop took {} ms (bound {} ms)",
            took.as_millis(),
            QUIT_BOUND.as_millis()
        );
        if !acknowledged {
            servers_gone(&self.server, Duration::from_secs(30));
        }
        (acknowledged, took)
    }

    /// [`Live::stop`], and the assertion that the worker acknowledged within `QUIT_BOUND`, which
    /// keeps a quit inside the relay's drain. The gate's stop is bounded at 1.2 s, and then the
    /// gate kills its server; a busy Mac can still fail the assertion: the message names the
    /// measured time.
    fn stop_within_bound(&mut self) -> Duration {
        let (acknowledged, took) = self.stop();
        assert!(
            acknowledged,
            "the worker did not acknowledge the stop within {QUIT_BOUND:?}: it took {took:?} (run the live proofs on an otherwise idle Mac)"
        );
        took
    }
}

impl Drop for Live {
    /// A failed test still stops its prover, so its Isabelle server does not outlive it.
    fn drop(&mut self) {
        if let Some(shell) = self.shell.take() {
            let _ = shell.stop(&self.events);
        }
    }
}

/// The request's denial, when it is blocked.
fn denial(request: &Request) -> Option<(String, String)> {
    match &request.status {
        LifecycleStatus::Blocked { denial } => Some((
            denial.rule.as_str().to_owned(),
            denial.proof_id.as_str().to_owned(),
        )),
        _ => None,
    }
}

/// Checks that `request` ended blocked on `rule` by an Isabelle proof of its fresh permit, made
/// after an earlier fresh proof (whose result the change made void), and prints its proof
/// reports. Returns the reports.
fn assert_blocked_by_a_fresh_proof(live: &mut Live, request: &Request, rule: &str) -> Vec<Value> {
    let request_id = request.request_id.as_str().to_owned();
    assert_eq!(request.status.state(), RequestState::Blocked, "{request:?}");
    let (denied, proof_id) = denial(request).expect("a denial");
    let reports = live.proofs_of(&request_id);
    for report in &reports {
        println!(
            "LIVE {request_id}: {} {} {}",
            report["purpose"], report["outcome"], report["failing_rule"]
        );
    }
    assert_eq!(denied, rule, "{reports:?}");
    let report = live.isabelle_proof(&proof_id, "block");
    assert_eq!(report["purpose"], "fresh_permit", "{report}");
    assert_eq!(report["failing_rule"], rule, "{report}");
    let fresh = reports
        .iter()
        .filter(|r| r["purpose"] == "fresh_permit")
        .count();
    assert!(fresh >= 2, "no fresh proof after the change: {reports:?}");
    reports
}

/// States in which nothing more happens to a request without the owner or a provider.
fn settled(state: RequestState) -> bool {
    !matches!(
        state,
        RequestState::Received
            | RequestState::Validating
            | RequestState::ProvingClassification
            | RequestState::Allowed
            | RequestState::Approved
            | RequestState::ProvingFreshPermit
            | RequestState::Queued
            | RequestState::Dispatching
            | RequestState::RetryWait
    )
}

/// PW L1 (DEF-7): the practice script with the real Isabelle. A create asks, is approved, is
/// proved permitted and is applied on the practice calendar, with its proof reports and its
/// authorization recorded; a delete of the event Fabric did not make is blocked by
/// `fabric_created`, its proof kept; an update and a delete of the created event are approved
/// and applied. No check file and no calendar data (titles, account, calendar, agent and event
/// ids, see [`calendar_text`]) is left in the prover's directory, Isabelle's user home or
/// Isabelle's temporary directory, and after quit the prover's directory is gone.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn the_practice_script_is_asked_approved_permitted_applied_and_blocked() {
    let _live = live_lock();
    assert!(TITLE.contains(TITLE_MARKER));
    let mut live = Live::start("script", Faults::default(), |_| {});
    live.subscribe();
    let ready = live.health_becomes(ProverHealth::Ready, START);
    println!("LIVE prover ready after {} ms", ready.as_millis());

    // A: a create asks, on the rule `authority`.
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    assert_eq!(a.tool.as_str(), "fabric_calendar_propose_create");
    let card = live.details(&a_id)["card"].clone();
    assert_eq!(card["insufficiency"]["rule"], "authority", "{card}");
    let asked = card["insufficiency"]["proof_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let report = live.isabelle_proof(&asked, "need_approval");
    assert_eq!(report["purpose"], "classification");
    assert_eq!(report["failing_rule"], "authority");
    live.approve(&a, json!({"kind": "one_request"}));
    live.request_in(&a_id, RequestState::Applied, STEP);
    let details = live.details(&a_id);
    let authorization = details["authorization"].clone();
    assert_eq!(authorization["request_id"], json!(a_id), "{details}");
    assert!(authorization["consumed_at"].is_string(), "{details}");
    let permit = authorization["proof_id"].as_str().unwrap().to_owned();
    let proof_ids: Vec<&str> = details["proof_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(proof_ids.contains(&asked.as_str()), "{details}");
    assert!(proof_ids.contains(&permit.as_str()), "{details}");
    let report = live.isabelle_proof(&permit, "permit");
    assert_eq!(report["purpose"], "fresh_permit");
    assert_eq!(report["action_digest"], authorization["action_digest"]);
    assert_eq!(details["attempts"][0]["outcome"], "applied", "{details}");
    assert_eq!(
        details["attempts"][0]["proof_id"],
        json!(permit),
        "{details}"
    );
    let event = live
        .calendar
        .event_of_request(&a.request_id)
        .expect("the practice event of A");
    assert_eq!(
        (event.revision.as_str(), event.fabric_created, event.deleted),
        ("1", true, false)
    );
    // `provider_event` stays null: the core fills it only with reconciliation (CD-W05).

    // B: the delete of the event Fabric did not make is blocked, with its rule and proof.
    let b = live.request_where("a blocked request", &[&a_id], STEP, |r| {
        r.status.state() == RequestState::Blocked
    });
    assert_eq!(b.tool.as_str(), "fabric_calendar_propose_delete");
    let (rule, proof_id) = denial(&b).expect("a denial");
    assert_eq!(rule, "fabric_created");
    let report = live.isabelle_proof(&proof_id, "block");
    assert_eq!(report["failing_rule"], "fabric_created");
    let b_id = b.request_id.as_str().to_owned();
    let details = live.details(&b_id);
    assert!(details["authorization"].is_null(), "{details}");
    let foreign = live
        .calendar
        .event(FOREIGN_EVENT_ID)
        .expect("the foreign event");
    assert_eq!((foreign.revision.as_str(), foreign.deleted), ("1", false));

    // C: the update of A's event asks, is approved and applied.
    let c = live.pending(&[&a_id, &b_id]);
    let c_id = c.request_id.as_str().to_owned();
    assert_eq!(c.tool.as_str(), "fabric_calendar_propose_update");
    live.approve(&c, json!({"kind": "one_request"}));
    live.request_in(&c_id, RequestState::Applied, STEP);
    let event = live.calendar.event(&event.event_id).expect("A's event");
    assert_eq!((event.revision.as_str(), event.deleted), ("2", false));

    // D: the delete of A's event asks, is approved and applied.
    let d = live.pending(&[&a_id, &b_id, &c_id]);
    let d_id = d.request_id.as_str().to_owned();
    assert_eq!(d.tool.as_str(), "fabric_calendar_propose_delete");
    live.approve(&d, json!({"kind": "one_request"}));
    live.request_in(&d_id, RequestState::Applied, STEP);
    let event = live.calendar.event(&event.event_id).expect("A's event");
    assert!(event.deleted);
    assert_eq!(live.calendar.dispatches(), 3);

    // While the prover runs: no check file is left, and no file holds calendar text.
    let left = files_under(&live.work_dir());
    assert!(
        left.iter().all(|p| !p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("Check_"))),
        "{left:?}"
    );
    let a_event_id = live
        .calendar
        .event_of_request(&a.request_id)
        .expect("the practice event of A")
        .event_id;
    let needles = calendar_text(&[FOREIGN_EVENT_ID, a_event_id.as_str()]);
    assert_eq!(
        calendar_text_under(&live.work_dir(), live.began_at, &needles),
        Vec::<PathBuf>::new()
    );
    let mut timings = Vec::new();
    for id in [&a_id, &b_id, &c_id, &d_id] {
        let details = live.details(id);
        for proof in details["proof_ids"].as_array().unwrap().clone() {
            let report = live.proof(proof.as_str().unwrap());
            timings.push(report["elapsed_ms"].as_u64().unwrap());
        }
    }
    timings.sort_unstable();
    let mean = timings.iter().sum::<u64>() / timings.len() as u64;
    println!(
        "LIVE {} checks: mean {mean} ms, median {} ms, max {} ms",
        timings.len(),
        timings[timings.len() / 2],
        timings[timings.len() - 1]
    );

    let took = live.stop_within_bound();
    assert!(took < DRAIN, "the stop took {took:?}");
    assert!(!live.work_dir().exists(), "the prover's directory is left");
    let user = live.isabelle.user_home.join(".isabelle");
    assert_eq!(
        calendar_text_under(&user, live.began_at, &needles),
        Vec::<PathBuf>::new()
    );
    let tmp = isabelle_tmp_prefix(&live.isabelle);
    println!(
        "LIVE scanned Isabelle's temporary files in {}",
        tmp.display()
    );
    assert_eq!(
        calendar_text_under(&tmp, live.began_at, &needles),
        Vec::<PathBuf>::new()
    );
    assert!(
        servers_gone(&live.server, DRAIN),
        "the Isabelle server is left"
    );
}

/// PW L7: revoking the grant of an approved request while its fresh proof is out dispatches
/// nothing: the result comes back to a changed fence and is not used, the request is proved
/// again, and Isabelle blocks it on the revoked grant; it ends without an authorization. The
/// approval is for one request: under the Careful profile a grant until revoked does not serve a
/// create (the rule theory's `standing_serves`), so without the revocation this approval would
/// be carried out.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn revoke_during_a_live_proof_dispatches_nothing() {
    let _live = live_lock();
    let mut live = Live::start("revoke", Faults::default(), |_| {});
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    let decided = live.approve(&a, json!({"kind": "one_request"}));
    let grant_id = decided["approval"]["grant_id"].clone();
    // No event is read between the approval and the revocation: the fresh proof's result, if
    // it is already back, waits in the channel.
    let snapshot = live.snapshot();
    let grant = snapshot["grants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["grant_id"] == grant_id)
        .cloned()
        .expect("the approval's grant");
    live.result(
        "revoke_grant",
        json!({"grant_id": grant_id, "expected_revision": grant["revision"]}),
    );
    let ended = live.until("A settles", STEP, |live| {
        live.last_of(&a_id).filter(|r| settled(r.status.state()))
    });
    println!("LIVE revoked request ended {:?}", ended.status.state());
    assert_blocked_by_a_fresh_proof(&mut live, &ended, REVOKED_RULE);
    live.run_for(Duration::from_secs(2));
    assert_ne!(
        live.last_of(&a_id).unwrap().status.state(),
        RequestState::Applied
    );
    assert_eq!(live.calendar.dispatches(), 0);
    assert!(live.calendar.event_of_request(&a.request_id).is_none());
    assert!(live.details(&a_id)["authorization"].is_null());
    live.settle_all();
    assert_eq!(live.calendar.dispatches(), 0);
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
}

/// PW L15: pausing every agent after an approval, while the fresh proof is out, dispatches nothing.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn pause_during_a_live_proof_dispatches_nothing() {
    let _live = live_lock();
    let mut live = Live::start("pause", Faults::default(), |_| {});
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    live.approve(&a, json!({"kind": "one_request"}));
    live.result(
        "pause_agent",
        json!({"target": {"kind": "all_agents"}, "paused": true}),
    );
    let ended = live.until("A settles", STEP, |live| {
        live.last_of(&a_id).filter(|r| settled(r.status.state()))
    });
    println!("LIVE paused request ended {:?}", ended.status.state());
    assert_blocked_by_a_fresh_proof(&mut live, &ended, PAUSED_RULE);
    live.run_for(Duration::from_secs(2));
    assert_ne!(
        live.last_of(&a_id).unwrap().status.state(),
        RequestState::Applied
    );
    assert_eq!(live.calendar.dispatches(), 0);
    assert!(live.calendar.event_of_request(&a.request_id).is_none());
    assert!(live.details(&a_id)["authorization"].is_null());
    live.settle_all();
    assert_eq!(live.calendar.dispatches(), 0);
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
}

/// A copy of the rule directory Isabelle is given, in a new temporary directory.
fn copied_rules(label: &str) -> TempDir {
    let rules = TempDir::new(label);
    let source = isabelle().rules_dir;
    for file in files_under(&source) {
        let to = rules.0.join(file.strip_prefix(&source).unwrap());
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(&file, &to).unwrap();
    }
    rules
}

/// A practice run on `rules` that the theory pin refuses: the prover is unavailable for the
/// run, no Isabelle server is started, the synthetic agent proposes nothing, and nothing is
/// proved.
fn assert_pinned_out(label: &str, rules: &Path) {
    let mut live = Live::start(label, Faults::default(), |options| {
        options.gate.rules_dir = rules.to_path_buf();
    });
    live.subscribe();
    live.health_becomes(ProverHealth::Unavailable, STEP);
    live.run_for(Duration::from_millis(2500));
    assert!(
        !live.provers(0).contains(&ProverHealth::Ready),
        "{label}: {:?}",
        live.healths
    );
    // The owner is told why: the rules are not the built ones (PW 2.2).
    assert!(
        live.healths
            .iter()
            .filter(|h| h.prover == ProverHealth::Unavailable)
            .all(|h| h.prover_problem == Some(ProverProblem::WrongRules)),
        "{label}: {:?}",
        live.healths
    );
    let snapshot = live.snapshot();
    assert_eq!(
        snapshot["health"]["prover_problem"], "wrong_rules",
        "{label}"
    );
    assert_eq!(snapshot["requests"], json!([]), "{label}");
    assert!(
        servers(&live.server).is_empty(),
        "{label}: an Isabelle server was started"
    );
    assert_eq!(files_under(&live.work_dir()), Vec::<PathBuf>::new());
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
}

/// PW L18, the theory pin: a rule theory that differs from the built one by one byte is refused
/// before any gate starts. The prover is unavailable for the run, no Isabelle server is started,
/// the synthetic agent proposes nothing, and nothing is proved.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn the_theory_pin_refuses_a_changed_rule_file() {
    let _live = live_lock();
    let rules = copied_rules("changed-rules");
    let theory = rules.0.join("Fabric_Broker").join("Fabric_Broker.thy");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&theory)
        .unwrap();
    file.write_all(b"\n").unwrap();
    drop(file);
    assert_pinned_out("pin", &rules.0);
}

/// PW L19, the theory pin: the built theory stays in place, but the session's `ROOT` points the
/// session `Fabric_Broker` at a copied theory changed by one byte, or a `ROOTS` beside it adds a
/// directory that defines the session with that theory; each is refused before any gate starts.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn the_theory_pin_refuses_session_files_that_load_another_theory() {
    let _live = live_lock();
    let redirects: [(&str, &str); 2] = [
        (
            "Fabric_Broker/ROOT",
            "session Fabric_Broker in \"../Rules2\" = HOL +\n  options [document = false]\n  theories\n    Fabric_Broker\n",
        ),
        ("Fabric_Broker/ROOTS", "../Rules2\n"),
    ];
    for (file, text) in redirects {
        let rules = copied_rules("redirected-rules");
        let other = rules.0.join("Rules2");
        std::fs::create_dir_all(&other).unwrap();
        let theory =
            std::fs::read_to_string(rules.0.join("Fabric_Broker").join("Fabric_Broker.thy"))
                .unwrap();
        std::fs::write(other.join("Fabric_Broker.thy"), format!("{theory}\n")).unwrap();
        if file == "Fabric_Broker/ROOTS" {
            std::fs::copy(
                rules.0.join("Fabric_Broker").join("ROOT"),
                other.join("ROOT"),
            )
            .unwrap();
        }
        std::fs::write(rules.0.join(file), text).unwrap();
        assert_pinned_out(&format!("pin-{}", file.replace('/', "-")), &rules.0);
    }
}

/// PW L16: an Isabelle server killed while a request is open ends that request `undecided` with
/// nothing dispatched and the reason `failed`; the owner sees the prover unavailable with
/// `failed_will_retry`, then starting and ready again, and the next request is decided.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn killing_isabelle_ends_the_open_request_undecided_then_ready_again() {
    let _live = live_lock();
    let mut live = Live::start("kill", Faults::default(), |options| {
        options.restart_waits = vec![Duration::from_secs(1); 2];
    });
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    let pids = servers(&live.server);
    assert!(
        !pids.is_empty(),
        "no process names the server {}",
        live.server
    );
    for pid in &pids {
        let killed = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status()
            .expect("kill runs");
        assert!(killed.success());
    }
    assert!(servers_gone(&live.server, Duration::from_secs(5)));
    let seen = live.healths.len();
    live.approve(&a, json!({"kind": "one_request"}));
    live.request_in(&a_id, RequestState::Undecided, STEP);
    assert_eq!(live.calendar.dispatches(), 0);
    let details = live.details(&a_id);
    assert!(details["authorization"].is_null(), "{details}");
    // The check on the killed server fails before its deadline: `FAILED` (PW 2.4).
    assert_eq!(details["undecided_reason"], "failed", "{details}");
    live.until("the owner told unavailable", STEP, |live| {
        live.provers(seen)
            .contains(&ProverHealth::Unavailable)
            .then_some(())
    });
    let back = live.health_becomes(ProverHealth::Ready, START);
    println!("LIVE prover ready again after {} ms", back.as_millis());
    assert_restarted(&live.healths[seen..]);
    // The script goes on with B, which the restarted prover decides.
    let b = live.request_where("a blocked request", &[&a_id], STEP, |r| {
        r.status.state() == RequestState::Blocked
    });
    assert_eq!(denial(&b).map(|d| d.0).as_deref(), Some("fabric_created"));
    assert_eq!(live.calendar.dispatches(), 0);
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
    assert!(servers_gone(&live.server, DRAIN));
}

/// The health the owner was told after a server was killed (PW 2.2): unavailable with
/// `failed_will_retry`, since a restart wait is left, then starting and ready with no problem.
fn assert_restarted(order: &[Health]) {
    let unavailable = order
        .iter()
        .position(|h| h.prover == ProverHealth::Unavailable)
        .unwrap_or_else(|| panic!("never unavailable: {order:?}"));
    assert_eq!(
        order[unavailable].prover_problem,
        Some(ProverProblem::FailedWillRetry),
        "{order:?}"
    );
    let starting = order[unavailable..]
        .iter()
        .position(|h| h.prover == ProverHealth::Starting)
        .map(|i| unavailable + i)
        .unwrap_or_else(|| panic!("never starting again: {order:?}"));
    let ready = order[starting..]
        .iter()
        .position(|h| h.prover == ProverHealth::Ready)
        .map(|i| starting + i)
        .unwrap_or_else(|| panic!("never ready again: {order:?}"));
    for at in [starting, ready] {
        assert_eq!(order[at].prover_problem, None, "{order:?}");
    }
}

/// The check theories in `dir`.
fn check_files(dir: &Path) -> Vec<PathBuf> {
    files_under(dir)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("Check_"))
        })
        .collect()
}

/// PW L2: an Isabelle server killed while it proves an approved request's
/// fresh permit, its check theory on disk, ends that request `undecided` with nothing dispatched,
/// no authorization and the reason `failed`; no check theory is left; the owner sees the prover
/// unavailable with `failed_will_retry`, then starting and ready again, and the next request is
/// decided.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn killing_isabelle_mid_check_is_undecided_then_ready_again() {
    let _live = live_lock();
    let mut live = Live::start("kill-mid", Faults::default(), |options| {
        options.restart_waits = vec![Duration::from_secs(1); 2];
    });
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    let pids = servers(&live.server);
    assert!(
        !pids.is_empty(),
        "no process names the server {}",
        live.server
    );
    let seen = live.healths.len();
    live.approve(&a, json!({"kind": "one_request"}));
    // The fresh proof's job is with the worker now; the main loop is not run until the kill, so
    // nothing of the result is read before it. The check theory is on disk while it runs.
    let deadline = Instant::now() + Duration::from_secs(10);
    let check = loop {
        if let Some(check) = check_files(&live.work_dir()).pop() {
            break check;
        }
        assert!(Instant::now() < deadline, "no check theory appeared");
        thread::sleep(Duration::from_millis(2));
    };
    let running = check.exists();
    for pid in &pids {
        let killed = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status()
            .expect("kill runs");
        assert!(killed.success());
    }
    assert!(running, "the check ended before the kill");
    assert!(servers_gone(&live.server, Duration::from_secs(5)));
    let ended = live.until("A settles", STEP, |live| {
        live.last_of(&a_id).filter(|r| settled(r.status.state()))
    });
    assert_eq!(ended.status.state(), RequestState::Undecided, "{ended:?}");
    assert_eq!(live.calendar.dispatches(), 0);
    assert!(live.calendar.event_of_request(&a.request_id).is_none());
    let details = live.details(&a_id);
    assert!(details["authorization"].is_null(), "{details}");
    // The check breaks before its deadline when its server is killed: `FAILED` (PW 2.4).
    assert_eq!(details["undecided_reason"], "failed", "{details}");
    assert_eq!(check_files(&live.work_dir()), Vec::<PathBuf>::new());
    live.until("the owner told unavailable", STEP, |live| {
        live.provers(seen)
            .contains(&ProverHealth::Unavailable)
            .then_some(())
    });
    let back = live.health_becomes(ProverHealth::Ready, START);
    println!("LIVE prover ready again after {} ms", back.as_millis());
    assert_restarted(&live.healths[seen..]);
    let b = live.request_where("a blocked request", &[&a_id], STEP, |r| {
        r.status.state() == RequestState::Blocked
    });
    assert_eq!(denial(&b).map(|d| d.0).as_deref(), Some("fabric_created"));
    assert_eq!(live.calendar.dispatches(), 0);
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
    assert!(servers_gone(&live.server, DRAIN));
}

/// PW L17, before a retry (plan 14): the grant of a request whose first dispatch
/// failed transiently is revoked while the request waits for its retry. The retry's fresh proof
/// sees the revocation and Isabelle blocks it: nothing more is dispatched.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn revoke_before_a_live_retry_blocks_the_retry() {
    let _live = live_lock();
    let faults = Faults {
        lose_answers: 0,
        transient: 1,
    };
    let mut live = Live::start("revoke-retry", faults, |_| {});
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    let decided = live.approve(&a, json!({"kind": "one_request"}));
    let grant_id = decided["approval"]["grant_id"].clone();
    live.request_in(&a_id, RequestState::RetryWait, STEP);
    assert_eq!(live.calendar.dispatches(), 1);
    let snapshot = live.snapshot();
    let grant = snapshot["grants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["grant_id"] == grant_id)
        .cloned()
        .expect("the approval's grant");
    live.result(
        "revoke_grant",
        json!({"grant_id": grant_id, "expected_revision": grant["revision"]}),
    );
    let ended = live.until("A settles", STEP, |live| {
        live.last_of(&a_id).filter(|r| settled(r.status.state()))
    });
    println!(
        "LIVE request revoked before its retry ended {:?}",
        ended.status.state()
    );
    let reports = assert_blocked_by_a_fresh_proof(&mut live, &ended, REVOKED_RULE);
    let permits = reports
        .iter()
        .filter(|r| r["purpose"] == "fresh_permit" && r["outcome"] == "permit")
        .count();
    assert_eq!(permits, 1, "{reports:?}");
    live.run_for(Duration::from_secs(2));
    assert_eq!(live.calendar.dispatches(), 1);
    let details = live.details(&a_id);
    let attempts = details["attempts"].as_array().expect("attempts");
    assert_eq!(attempts.len(), 1, "{details}");
    assert_eq!(attempts[0]["outcome"], "transient_failure", "{details}");
    live.settle_all();
    assert_eq!(live.calendar.dispatches(), 1);
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
}

/// A started built broker: its stdout lines on a channel.
struct Process {
    child: std::process::Child,
    lines: Receiver<Vec<u8>>,
    /// Set once stderr said the prover is not tried again in this run.
    given_up: std::sync::Arc<std::sync::atomic::AtomicBool>,
    next_id: u64,
    /// The name of its data directory's Isabelle server.
    server: String,
}

/// The fixed `PATH` the relay gives the broker on Unix; Isabelle's scripts need `/usr/sbin`
/// (`sysctl`).
const UNIX_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

impl Process {
    /// The built broker on `data_dir` with the test key and `isabelle`, and with `synthetic`
    /// the synthetic agent (a practice run), started the way the host starts it: the relay's
    /// allowlist and its command, which clears every other variable (no `HOME`) and sets only
    /// the fixed [`UNIX_PATH`]. Its stderr is copied to the test's.
    fn start(data_dir: &Path, isabelle: &IsabelleSettings, synthetic: bool) -> Process {
        // The test's whole environment goes in; the relay keeps only its allowlist, and the
        // command clears everything else.
        let mut host: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| !name.as_encoded_bytes().starts_with(b"FABRIC_BROKER_"))
            .collect();
        host.extend([
            (
                "FABRIC_BROKER_DATA_DIR".into(),
                data_dir.as_os_str().to_owned(),
            ),
            ("FABRIC_BROKER_KEY_SOURCE".into(), "test".into()),
            (
                ISABELLE_HOME.into(),
                isabelle.isabelle_home.as_os_str().to_owned(),
            ),
            (
                ISABELLE_USER_HOME.into(),
                isabelle.user_home.as_os_str().to_owned(),
            ),
            (RULES_DIR.into(), isabelle.rules_dir.as_os_str().to_owned()),
        ]);
        if synthetic {
            host.push(("FABRIC_BROKER_SYNTHETIC_AGENT".into(), "1".into()));
        }
        let config = BrokerConfig::check(
            Path::new(env!("CARGO_BIN_EXE_fabric-broker")),
            ENVIRONMENT_ALLOWLIST,
            host,
        )
        .unwrap_or_else(|e| panic!("the relay refuses the configuration: {e}"));
        let names: Vec<&OsString> = config.environment().iter().map(|(n, _)| n).collect();
        assert!(
            names
                .iter()
                .all(|n| n.as_encoded_bytes().starts_with(b"FABRIC_BROKER_")),
            "only the broker's own variables: {names:?}"
        );
        let mut command = config.command();
        let envs: Vec<(OsString, Option<OsString>)> = command
            .get_envs()
            .map(|(n, v)| (n.to_owned(), v.map(ToOwned::to_owned)))
            .collect();
        let paths: Vec<&Option<OsString>> = envs
            .iter()
            .filter(|(n, _)| n == "PATH")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(
            paths,
            vec![&Some(OsString::from(UNIX_PATH))],
            "the relay's fixed PATH, never the test's"
        );
        command.stderr(std::process::Stdio::piped());
        let mut child = command.spawn().expect("the broker starts");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let given_up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&given_up);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("{line}");
                if line.contains("in this run") {
                    flag.store(true, Ordering::SeqCst);
                }
            }
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
        Process {
            child,
            lines,
            given_up,
            next_id: 0,
            server: server_name(data_dir),
        }
    }

    /// Sends an owner frame and returns the result of its response, skipping notices.
    fn result(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = format!("process-{}", self.next_id);
        let mut line = owner_line(&id, method, params);
        line.push(b'\n');
        let stdin = self.child.stdin.as_mut().expect("stdin is open");
        stdin.write_all(&line).unwrap();
        stdin.flush().unwrap();
        loop {
            let line = match self.lines.recv_timeout(Duration::from_secs(10)) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => panic!("no answer to {method}"),
                Err(RecvTimeoutError::Disconnected) => panic!("the broker closed stdout"),
            };
            let (frame, value) = decode_line(line.strip_suffix(b"\n").expect("a line feed"));
            if let Frame::OwnerResponse(_) = frame
                && value["id"] == json!(id)
            {
                assert!(value.get("error").is_none(), "{method}: {value}");
                return value["result"].clone();
            }
        }
    }

    /// Asks for snapshots until the prover is `health`; returns when.
    fn prover_becomes(&mut self, health: &str, bound: Duration) -> Duration {
        let began = Instant::now();
        loop {
            let snapshot = self.result("get_snapshot", json!({}));
            if snapshot["health"]["prover"] == health {
                return began.elapsed();
            }
            assert!(
                !self.given_up.load(Ordering::SeqCst),
                "the broker gave up its prover for the run (its stderr is above)"
            );
            assert!(
                began.elapsed() < bound,
                "the prover was not {health} within {bound:?}: {}",
                snapshot["health"]
            );
            thread::sleep(Duration::from_millis(250));
        }
    }

    /// Subscribes from the current cursor, which lets the synthetic agent start.
    fn subscribe(&mut self) {
        let cursor = self.result("get_snapshot", json!({}))["cursor"].clone();
        self.result("subscribe", json!({"cursor": cursor}));
    }

    /// Asks for snapshots until it lists a request, for up to `bound`.
    fn first_request(&mut self, bound: Duration) {
        let began = Instant::now();
        loop {
            let snapshot = self.result("get_snapshot", json!({}));
            if snapshot["requests"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
            {
                return;
            }
            assert!(began.elapsed() < bound, "no request within {bound:?}");
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Closes stdin and waits up to `bound` for the exit; how long it took, or `None`.
    fn quit(&mut self, bound: Duration) -> Option<Duration> {
        drop(self.child.stdin.take());
        let began = Instant::now();
        while began.elapsed() < bound {
            if let Some(status) = self.child.try_wait().expect("the child can be waited for") {
                assert_eq!(status.code(), Some(0));
                return Some(began.elapsed());
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Process {
    /// A failed test still ends its broker the way the host does: end of input, up to the
    /// relay's drain for the broker to stop its Isabelle server. Only then SIGKILL, which leaves
    /// the server running (PW L4), so its processes are killed too.
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let began = Instant::now();
        while began.elapsed() < DRAIN {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        for pid in servers(&self.server) {
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .status();
        }
    }
}

/// PW L11: the built broker, with the environment the relay leaves it (the broker's own
/// variables and the relay's fixed `PATH`; no `HOME`, nothing of the test's), starts Isabelle and
/// reports the prover ready. Run first: if Isabelle cannot start this way, no live proof is
/// possible in the app.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn the_built_broker_starts_isabelle_with_an_empty_environment() {
    let _live = live_lock();
    let dir = TempDir::new("empty-environment");
    let isabelle = isabelle();
    let mut broker = Process::start(&dir.data(), &isabelle, false);
    let ready = broker.prover_becomes("ready", START);
    println!(
        "LIVE built broker: prover ready after {} ms",
        ready.as_millis()
    );
    let server = server_name(&dir.data());
    assert!(
        broker.quit(DRAIN).is_some(),
        "the broker did not exit within the drain"
    );
    assert!(servers_gone(&server, DRAIN), "the Isabelle server is left");
    assert!(!dir.data().join("prover").exists());
}

/// PW L13: once the prover is ready, end of input stops the built broker within the relay's
/// drain, its Isabelle server with it, and removes the prover's directory.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn quit_stops_isabelle_within_the_relay_drain() {
    let _live = live_lock();
    let dir = TempDir::new("quit");
    let isabelle = isabelle();
    let mut broker = Process::start(&dir.data(), &isabelle, false);
    broker.prover_becomes("ready", START);
    let server = server_name(&dir.data());
    assert!(
        !servers(&server).is_empty(),
        "no process names the server {server}"
    );
    let took = broker
        .quit(DRAIN)
        .expect("the broker exits within the drain");
    println!("LIVE built broker quit in {} ms", took.as_millis());
    assert!(
        servers_gone(&server, Duration::from_millis(500)),
        "the Isabelle server is left"
    );
    assert!(!dir.data().join("prover").exists());
}

/// PW L3: a check over its budget is undecided. With a check limit of 1 ms every check runs
/// past it, so Isabelle's report comes back with the outcome `undecided`, no failing rule and no
/// authorizing grant; the request ends `undecided` with no authorization and the reason
/// `out_of_time` (the check took its whole limit), nothing is dispatched, and the gate is kept
/// (the prover stays ready). Isabelle ends the check at its first look, so
/// the whole check takes under 500 ms (a full check takes about a second), not only its idle
/// time.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn a_check_over_budget_is_undecided() {
    let _live = live_lock();
    let mut live = Live::start("budget", Faults::default(), |options| {
        options.gate.check_timeout = Duration::from_millis(1);
    });
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let seen = live.healths.len();
    let a = live.request_where("an undecided request", &[], STEP, |r| {
        r.status.state() == RequestState::Undecided
    });
    let a_id = a.request_id.as_str().to_owned();
    assert_eq!(a.tool.as_str(), "fabric_calendar_propose_create");
    let details = live.details(&a_id);
    assert!(details["authorization"].is_null(), "{details}");
    // The check took at least the 1 ms limit the adapter gave it (PW 2.3).
    assert_eq!(details["undecided_reason"], "out_of_time", "{details}");
    let reports = live.proofs_of(&a_id);
    assert_eq!(reports.len(), 1, "{reports:?}");
    let report = &reports[0];
    assert_eq!(report["outcome"], "undecided", "{report}");
    assert_eq!(report["purpose"], "classification", "{report}");
    assert!(report["failing_rule"].is_null(), "{report}");
    assert_eq!(report["isabelle_version"], "Isabelle2025-2", "{report}");
    assert_eq!(
        report["theory_hash"],
        json!(format!("sha256:{}", built_theory_sha256())),
        "{report}"
    );
    println!(
        "LIVE a check over a 1 ms budget took {} ms",
        report["elapsed_ms"]
    );
    assert!(
        report["elapsed_ms"].as_u64().unwrap() < 500,
        "the check ran on past its limit: {report}"
    );
    live.settle_all();
    assert_eq!(live.calendar.dispatches(), 0);
    for id in live.request_ids() {
        assert_eq!(
            live.last_of(&id).map(|r| r.status.state()),
            Some(RequestState::Undecided),
            "{id}"
        );
        assert_eq!(live.details(&id)["undecided_reason"], "out_of_time", "{id}");
    }
    assert!(
        !live.provers(seen).contains(&ProverHealth::Unavailable),
        "{:?}",
        live.healths
    );
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
    assert!(servers_gone(&live.server, DRAIN));
}

/// PW L4: a built broker killed with SIGKILL in the middle of a practice run leaves its
/// prover directory and maybe its Isabelle server behind. A broker started again on the same
/// data directory empties the prover directory before its gate starts (a check file the killed
/// one left is gone), ends with as many server processes as one server has (it finds the
/// orphan by its name or starts one, never a second), reports the prover ready, and on quit
/// leaves no server and no prover directory.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn a_killed_broker_restarts_with_an_empty_work_dir_and_one_server() {
    let _live = live_lock();
    let dir = TempDir::new("killed");
    let isabelle = isabelle();
    let server = server_name(&dir.data());
    let work_dir = dir.data().join("prover");
    // Dropped after both brokers: a server a failure or the second quit leaves (a reused
    // orphan, PW K1) is killed.
    let _servers = KillServersOnDrop(server.clone());

    let mut first = Process::start(&dir.data(), &isabelle, true);
    first.prover_becomes("ready", START);
    first.subscribe();
    first.first_request(STEP);
    let one_server = servers(&server);
    assert!(
        !one_server.is_empty(),
        "no process names the server {server}"
    );
    first.child.kill().expect("SIGKILL reaches the broker");
    first.child.wait().expect("the killed broker is reaped");
    // What a check that was running at the kill leaves: its theory file, with calendar text.
    let left = work_dir.join("Check_000999.thy");
    std::fs::write(&left, format!("theory Check_000999 (* {TITLE_MARKER} *)"))
        .expect("the killed broker's prover directory is still there");
    println!(
        "LIVE after SIGKILL: {} of {} server processes are left",
        servers(&server).len(),
        one_server.len()
    );

    let mut second = Process::start(&dir.data(), &isabelle, false);
    second.prover_becomes("ready", START);
    assert!(!left.exists(), "the killed broker's check file is left");
    let checks: Vec<PathBuf> = files_under(&work_dir)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("Check_"))
        })
        .collect();
    assert_eq!(checks, Vec::<PathBuf>::new());
    assert_eq!(
        servers(&server).len(),
        one_server.len(),
        "not the processes of one server"
    );
    let quit = second.quit(DRAIN);
    if let Some(took) = quit {
        println!("LIVE the second broker quit in {} ms", took.as_millis());
    }
    assert!(quit.is_some(), "the broker did not exit within the drain");
    assert!(servers_gone(&server, DRAIN), "the Isabelle server is left");
    assert!(!work_dir.exists());
    drop(first);
}

/// Sends `kill -KILL` to its process ids on drop; a process already gone is no error.
struct KillOnDrop(Vec<u32>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        for pid in &self.0 {
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// Sends `kill -KILL`, on drop, to every process that still names the server. Declared before a
/// test's brokers it drops after them, so a failing test leaves no Isabelle server running.
struct KillServersOnDrop(String);

impl Drop for KillServersOnDrop {
    fn drop(&mut self) {
        drop(KillOnDrop(servers(&self.0)));
    }
}

/// PW L20: a quit while the Isabelle server does not answer (its processes stopped with
/// SIGSTOP) still ends the built broker within the relay's drain: the gate's stop waits its
/// bound, then kills the server's process groups. No server is left and the prover's directory
/// is gone. The test kills the stopped processes itself at its end, so a failure leaves nothing
/// stopped.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn quit_kills_a_server_that_does_not_answer() {
    let _live = live_lock();
    let dir = TempDir::new("unanswering");
    let isabelle = isabelle();
    let mut broker = Process::start(&dir.data(), &isabelle, false);
    broker.prover_becomes("ready", START);
    let server = server_name(&dir.data());
    let pids = servers(&server);
    assert!(!pids.is_empty(), "no process names the server {server}");
    for pid in &pids {
        let _ = Command::new("kill")
            .arg("-STOP")
            .arg(pid.to_string())
            .status();
    }
    let _stopped = KillOnDrop(pids);
    let took = broker
        .quit(DRAIN)
        .expect("the broker exits within the drain");
    println!(
        "LIVE built broker quit with an unanswering server in {} ms",
        took.as_millis()
    );
    assert!(
        servers_gone(&server, Duration::from_millis(500)),
        "the Isabelle server is left"
    );
    assert!(!dir.data().join("prover").exists());
}

/// PW L21: a quit while the gate is still starting leaves no server. Once a process names the
/// server and the owner channel still reports the prover `starting`, end of input ends the built
/// broker within the relay's drain: the worker kills the start through its kill handle before it
/// acknowledges. No server is left 500 ms later and the prover's directory is gone. A prover
/// already ready by then fails the test, since the quit would not be one during a start.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn quit_during_a_gate_start_leaves_no_server() {
    let _live = live_lock();
    let dir = TempDir::new("quit-starting");
    let isabelle = isabelle();
    let server = server_name(&dir.data());
    // Dropped after the broker: a server a failure leaves is killed.
    let _servers = KillServersOnDrop(server.clone());
    let mut broker = Process::start(&dir.data(), &isabelle, false);
    let began = Instant::now();
    loop {
        if !servers(&server).is_empty() {
            let snapshot = broker.result("get_snapshot", json!({}));
            let prover = &snapshot["health"]["prover"];
            assert!(
                *prover != "ready",
                "the prover was ready before the test could quit during its start: no quit during a start was tried"
            );
            assert_eq!(
                *prover, "starting",
                "a process names the server, but the prover is not starting: {}",
                snapshot["health"]
            );
            assert!(snapshot["health"]["prover_problem"].is_null(), "{snapshot}");
            break;
        }
        assert!(
            !broker.given_up.load(Ordering::SeqCst),
            "the broker gave up its prover for the run (its stderr is above)"
        );
        assert!(
            began.elapsed() < START,
            "no process named the server {server} within {START:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let took = broker
        .quit(DRAIN)
        .expect("the broker exits within the drain");
    println!(
        "LIVE built broker quit during a gate start in {} ms",
        took.as_millis()
    );
    assert!(
        servers_gone(&server, Duration::from_millis(500)),
        "the Isabelle server is left"
    );
    assert!(
        !dir.data().join("prover").exists(),
        "the prover's directory is left"
    );
}

/// PW L22: a quit while the worker is inside a check that does not end (its server's processes
/// stopped with SIGSTOP) is still acknowledged within `QUIT_BOUND`: at the stop's instant, 1.2 s
/// after the stop began, the calling thread finds the check running and kills its server through
/// the kill handle; the check fails, the broken gate's stop returns at once and the worker
/// acknowledges. A check theory in the work directory shows that the check runs when the stop
/// begins. No server is left 500 ms later and the work directory is gone. The test kills the
/// stopped processes itself at its end, so a failure leaves nothing stopped.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn quit_during_a_check_kills_its_server() {
    let _live = live_lock();
    let mut live = Live::start("quit-checking", Faults::default(), |_| {});
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    // A's classification check has ended: A waits for the owner.
    let a = live.pending(&[]);
    let pids = servers(&live.server);
    assert!(
        !pids.is_empty(),
        "no process names the server {}",
        live.server
    );
    for pid in &pids {
        let _ = Command::new("kill")
            .arg("-STOP")
            .arg(pid.to_string())
            .status();
    }
    let _stopped = KillOnDrop(pids);
    live.approve(&a, json!({"kind": "one_request"}));
    // The fresh permit's job is with the worker now; the main loop is not run before the stop.
    // A check theory is on disk only while a check runs, and with the server stopped this one
    // does not end: its limit is the job's deadline, far beyond this test's few seconds.
    let deadline = Instant::now() + Duration::from_secs(10);
    let check = loop {
        if let Some(check) = check_files(&live.work_dir()).pop() {
            break check;
        }
        assert!(Instant::now() < deadline, "no check theory appeared");
        thread::sleep(Duration::from_millis(2));
    };
    assert!(check.exists(), "the check ended before the stop");
    let took = live.stop_within_bound();
    println!("LIVE stop during a check took {} ms", took.as_millis());
    // The gate's stop bound (`GATE_STOP_BOUND` in the prover adapter, not visible here): the
    // check is killed only at the stop's instant, this long after the stop began.
    let gate_stop_bound = Duration::from_millis(1_200);
    assert!(
        took >= gate_stop_bound,
        "the stop took {took:?}, less than the gate's stop bound {gate_stop_bound:?}: the check was not running at the stop's instant"
    );
    assert!(
        servers_gone(&live.server, Duration::from_millis(500)),
        "the Isabelle server is left"
    );
    assert!(!live.work_dir().exists(), "the work directory is left");
}

/// PW L5 (PLAN 11): a dispatch whose answer is lost changes the practice calendar once and ends
/// `outcome_unknown`; it is never sent again.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn a_lost_answer_is_outcome_unknown_and_never_sent_again() {
    let _live = live_lock();
    let faults = Faults {
        lose_answers: 1,
        transient: 0,
    };
    let mut live = Live::start("lost", faults, |_| {});
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    live.approve(&a, json!({"kind": "one_request"}));
    live.request_in(&a_id, RequestState::OutcomeUnknown, STEP);
    live.settle_all();
    assert_eq!(
        live.last_of(&a_id).map(|r| r.status.state()),
        Some(RequestState::OutcomeUnknown)
    );
    assert_eq!(live.calendar.dispatches(), 1);
    let event = live
        .calendar
        .event_of_request(&a.request_id)
        .expect("the lost answer's change is kept");
    assert_eq!(event.revision, "1");
    let details = live.details(&a_id);
    let attempts = details["attempts"].as_array().expect("attempts");
    assert_eq!(attempts.len(), 1, "{details}");
    assert_eq!(attempts[0]["outcome"], "outcome_unknown", "{details}");
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
}

/// PW L6 (PLAN 5): a transient provider failure is retried, and the retry has a fresh proof of
/// its own: two attempts, each resting on its own Isabelle permit, the second applied.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn a_transient_failure_is_proved_again_before_the_retry() {
    let _live = live_lock();
    let faults = Faults {
        lose_answers: 0,
        transient: 1,
    };
    let mut live = Live::start("transient", faults, |_| {});
    live.subscribe();
    live.health_becomes(ProverHealth::Ready, START);
    let a = live.pending(&[]);
    let a_id = a.request_id.as_str().to_owned();
    live.approve(&a, json!({"kind": "one_request"}));
    live.request_in(&a_id, RequestState::Applied, STEP);
    assert_eq!(live.calendar.dispatches(), 2);
    let details = live.details(&a_id);
    let attempts = details["attempts"].as_array().expect("attempts").clone();
    assert_eq!(attempts.len(), 2, "{details}");
    assert_eq!(attempts[0]["outcome"], "transient_failure", "{details}");
    assert_eq!(attempts[1]["outcome"], "applied", "{details}");
    let first = attempts[0]["proof_id"].as_str().unwrap().to_owned();
    let second = attempts[1]["proof_id"].as_str().unwrap().to_owned();
    assert_ne!(first, second, "the retry reused the first attempt's proof");
    for proof_id in [&first, &second] {
        let report = live.isabelle_proof(proof_id, "permit");
        assert_eq!(report["purpose"], "fresh_permit", "{report}");
    }
    live.settle_all();
    live.stop_within_bound();
    assert!(!live.work_dir().exists());
}

/// PW L14 (PLAN 5): the checks of the whole practice script are timed. Prints the prover's start
/// and the mean, median and maximum of the checks; a mean over 1.5 s fails, because W03 then
/// reopens the proof timeout.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn checks_are_timed() {
    let _live = live_lock();
    let mut live = Live::start("timed", Faults::default(), |_| {});
    live.subscribe();
    let ready = live.health_becomes(ProverHealth::Ready, START);
    println!("LIVE prover start: ready after {} ms", ready.as_millis());
    let mut approved: Vec<String> = Vec::new();
    for _ in 0..3 {
        let except: Vec<&str> = approved.iter().map(String::as_str).collect();
        let request = live.pending(&except);
        let id = request.request_id.as_str().to_owned();
        live.approve(&request, json!({"kind": "one_request"}));
        live.request_in(&id, RequestState::Applied, STEP);
        approved.push(id);
    }
    live.settle_all();
    let mut timings = Vec::new();
    for id in live.request_ids() {
        for report in live.proofs_of(&id) {
            timings.push(report["elapsed_ms"].as_u64().expect("elapsed_ms"));
        }
    }
    assert!(timings.len() >= 7, "only {} checks", timings.len());
    timings.sort_unstable();
    let count = u64::try_from(timings.len()).expect("a count");
    let mean = timings.iter().sum::<u64>() / count;
    println!(
        "LIVE {} checks: mean {mean} ms, median {} ms, max {} ms",
        timings.len(),
        timings[timings.len() / 2],
        timings[timings.len() - 1]
    );
    live.stop_within_bound();
    assert!(
        mean <= 1_500,
        "the mean check took {mean} ms, over 1.5 s: W03 reopens the proof timeout (PLAN 5)"
    );
}

/// What the action of a cell names: no event, or the event the broker would observe.
#[derive(Clone, Copy, Debug)]
enum Observed {
    /// A create, a read or an availability request names no event.
    Nothing,
    /// An event Fabric created, with no attendees.
    Own,
    /// An event Fabric did not create, with no attendees.
    Foreign,
    /// An event Fabric created, with one attendee in no recipient set.
    OwnWithAttendee,
    /// An invitation the owner received.
    Invitation,
}

/// One cell of the duration table (p10 of the CD-W04 design, D40): a tool, the profile in
/// force (a built-in one, or Careful's rows with `changes` saved as a custom profile), the
/// calendar and the event the action names; the duration the core proposes; and what the
/// theory proves with only the real lasting grant in the context (for `until_revoked` and for
/// the live session alike) and with only the one-request grant. The expected outcomes are the
/// worked cases of `isabelle/Fabric_Broker_Examples/Broker_Examples.thy`, "What a lasting
/// approval is worth".
struct Cell {
    name: &'static str,
    tool: &'static str,
    profile: &'static str,
    changes: &'static [(&'static str, &'static str)],
    calendar: &'static str,
    observed: Observed,
    proposed: &'static str,
    lasting: Outcome,
    one_request: Outcome,
}

const AVAILABILITY: &str = "fabric_calendar_availability";
const READ: &str = "fabric_calendar_read";
const CREATE: &str = "fabric_calendar_propose_create";
const UPDATE: &str = "fabric_calendar_propose_update";
const INVITATION: &str = "fabric_calendar_propose_invitation";
const RESPONSE: &str = "fabric_calendar_propose_response";
const DELETE: &str = "fabric_calendar_propose_delete";

/// The table. Each cell that proposes `one_request` where the lasting grant would be permitted
/// is a deliberate under-proposal (D40; PW K14 names the condition that lifts each).
#[rustfmt::skip]
const CELLS: &[Cell] = &[
    Cell { name: "availability, allow", tool: AVAILABILITY, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Nothing, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "availability, ask", tool: AVAILABILITY, profile: "careful", changes: &[("list_and_availability", "ask")], calendar: support::CALENDAR, observed: Observed::Nothing, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "read, allow", tool: READ, profile: "flexible", changes: &[], calendar: support::CALENDAR, observed: Observed::Nothing, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "read, ask (CD-M02)", tool: READ, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Nothing, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "create in Agent Drafts, allow", tool: CREATE, profile: "careful", changes: &[], calendar: support::DRAFTS, observed: Observed::Nothing, proposed: "until_revoked", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "create in Agent Drafts, ask", tool: CREATE, profile: "careful", changes: &[("create_in_agent_drafts", "ask")], calendar: support::DRAFTS, observed: Observed::Nothing, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "create in a plain calendar, allow", tool: CREATE, profile: "flexible", changes: &[], calendar: support::CALENDAR, observed: Observed::Nothing, proposed: "until_revoked", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "create in a plain calendar, ask", tool: CREATE, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Nothing, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "edit own event in Agent Drafts, allow", tool: UPDATE, profile: "flexible", changes: &[], calendar: support::DRAFTS, observed: Observed::Own, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "edit own event in Agent Drafts, ask (CD-M05)", tool: UPDATE, profile: "careful", changes: &[], calendar: support::DRAFTS, observed: Observed::Own, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "edit own event elsewhere, allow", tool: UPDATE, profile: "flexible", changes: &[], calendar: support::CALENDAR, observed: Observed::Own, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "edit own event elsewhere, ask", tool: UPDATE, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Own, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "edit an event Fabric did not create, allow", tool: UPDATE, profile: "careful", changes: &[("edit_preexisting_event", "allow")], calendar: support::CALENDAR, observed: Observed::Foreign, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "edit an event Fabric did not create, ask", tool: UPDATE, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Foreign, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "edit an event with attendees, allow", tool: UPDATE, profile: "flexible", changes: &[], calendar: support::CALENDAR, observed: Observed::OwnWithAttendee, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::NeedApproval },
    Cell { name: "edit an event with attendees, ask", tool: UPDATE, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::OwnWithAttendee, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::NeedApproval },
    Cell { name: "invitation, allow", tool: INVITATION, profile: "flexible", changes: &[], calendar: support::CALENDAR, observed: Observed::Own, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "invitation, ask", tool: INVITATION, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Own, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "response, allow", tool: RESPONSE, profile: "careful", changes: &[("respond_to_invitation", "allow")], calendar: support::CALENDAR, observed: Observed::Invitation, proposed: "one_request", lasting: Outcome::Permit, one_request: Outcome::Permit },
    Cell { name: "response, ask", tool: RESPONSE, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Invitation, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "delete under Careful", tool: DELETE, profile: "careful", changes: &[], calendar: support::CALENDAR, observed: Observed::Own, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
    Cell { name: "delete under Flexible", tool: DELETE, profile: "flexible", changes: &[], calendar: support::CALENDAR, observed: Observed::Own, proposed: "one_request", lasting: Outcome::NeedApproval, one_request: Outcome::Permit },
];

/// The event every action of a cell names.
const EVENT_ID: &str = "evt-fabric-0042";
const EVENT_REVISION: &str = "3181161799000000";

/// The arguments of the cell's call.
fn arguments(cell: &Cell) -> Value {
    let account = support::ACCOUNT;
    let calendar = cell.calendar;
    let window = json!({"start": "2026-11-27T06:00:00.000Z", "end": "2026-11-28T06:00:00.000Z"});
    let event = json!({"event_id": EVENT_ID, "revision": EVENT_REVISION});
    match cell.tool {
        AVAILABILITY => json!({"account_id": account, "calendar_ids": [calendar],
                               "window": window, "task": "Find a free hour."}),
        READ => json!({"account_id": account, "calendar_ids": [calendar], "window": window,
                       "fields": ["time", "title"], "task": "See what is on Friday."}),
        CREATE => support::create_params(calendar),
        UPDATE => json!({"account_id": account, "calendar_id": calendar, "event": event,
                         "changes": {"title": "Design review", "start": null, "end": null,
                                     "description": null, "location": null},
                         "send_updates": "all", "task": "Rename the event."}),
        INVITATION => json!({"account_id": account, "calendar_id": calendar, "event": event,
                             "attendees": ["ally@example.net"], "send_updates": "all",
                             "task": "Invite Ally."}),
        RESPONSE => json!({"account_id": account, "calendar_id": calendar,
                           "target": {"kind": "event", "event": event},
                           "response": "accepted", "send_updates": "all",
                           "task": "Answer the invitation."}),
        DELETE => json!({"account_id": account, "calendar_id": calendar, "event": event,
                         "send_updates": "all", "task": "Remove the draft I made."}),
        other => panic!("no arguments for {other}"),
    }
}

/// The event the broker would observe for the cell, as CD-W05 will put it in the facts.
fn observation(cell: &Cell) -> Option<ObservedEvent> {
    let (fabric_created, invitation, attendees) = match cell.observed {
        Observed::Nothing => return None,
        Observed::Own => (true, false, Vec::new()),
        Observed::Foreign => (false, false, Vec::new()),
        Observed::OwnWithAttendee => (true, false, vec!["bob@example.net".to_owned()]),
        Observed::Invitation => (false, true, Vec::new()),
    };
    Some(ObservedEvent {
        calendar: CalendarRef {
            account_id: support::id(support::ACCOUNT),
            calendar_id: support::id(cell.calendar),
        },
        event_id: EVENT_ID.to_owned(),
        revision: EVENT_REVISION.to_owned(),
        fabric_created,
        recurring: false,
        default_type: true,
        timed: true,
        invitation,
        attendees,
        occurrence_start_ms: None,
    })
}

/// The fresh-permit job `job` with `grant_id` as its only grant, under `term`, and the request
/// resting on it alone. Tools and scope are those `create_grant` gave the real grant; they are
/// the same for every duration, so only the term is changed.
fn with_term(job: &ProofJob, grant_id: &str, term: GrantTerm) -> ProofJob {
    let mut job = job.clone();
    let mut fact = job
        .facts
        .grants
        .iter()
        .find(|g| g.grant.grant_id.as_str() == grant_id)
        .cloned()
        .unwrap_or_else(|| panic!("the job offers no grant {grant_id}"));
    fact.grant.term = term;
    fact.used = false;
    fact.ended = false;
    job.facts.grants = vec![fact];
    job.facts.relied = vec![support::id(grant_id)];
    job
}

/// The gate's answer for `job`: its outcome, or `None` when undecided, and whether the grant
/// authorizes it.
fn gate_answer(gate: &mut BrokerGate, job: &ProofJob, grant_id: &str) -> (Option<Outcome>, bool) {
    let (context, action) = context_of(job).expect("the job maps");
    let check = gate.check(&context, &action).expect("the gate checks");
    let authorizes = check.authorizing_grants.iter().any(|g| g == grant_id);
    (check.verdict.outcome(), authorizes)
}

/// p10 of the CD-W04 design (D40): the duration the real core proposes for each cell of the
/// table agrees with the rule theory. For each cell a core in its own data directory proposes
/// the cell's call under the cell's profile; the classification is scripted to ask, so the real
/// `open_card` opens the card. The owner approves the proposed duration (`one_request` where the
/// card proposes no more); the fresh-permit job the core hands out carries the real grant, with
/// the observed event set where the cell names one. Isabelle then checks that job with the
/// grant as its only grant, under `until_revoked`, under the live session and under one
/// request. A cell that proposes `until_revoked` must be permitted on that grant, and every
/// outcome must be the worked case's. The table it found is printed.
#[test]
#[ignore = "live: needs Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]
fn the_proposed_duration_agrees_with_the_rule_theory() {
    let _live = live_lock();
    let dir = TempDir::new("durations");
    std::fs::create_dir_all(dir.data()).expect("the data directory");
    let mut gate =
        BrokerGate::start(gate_config(&isabelle(), &dir.data())).expect("the gate starts");
    let mut rows = Vec::new();
    let mut wrong = Vec::new();
    for cell in CELLS {
        let mut core = support::Core::start("p10");
        if !cell.changes.is_empty() {
            let mut changed = serde_json::to_value(ProfileRows::CAREFUL).expect("rows");
            for (row, mode) in cell.changes {
                changed[*row] = json!(mode);
            }
            core.activate_custom("Careful with one row changed", &changed);
        } else if cell.profile != "careful" {
            core.activate(cell.profile);
        }
        let request_id = core.pending(cell.tool, arguments(cell));
        let proposed = core.proposed(&request_id);
        let duration = if proposed == "until_revoked" {
            "until_revoked"
        } else {
            "one_request"
        };
        let (grant_id, mut job) = core.approve(&request_id, duration);
        job.facts.event = observation(cell);
        let session = job
            .facts
            .session
            .as_ref()
            .expect("the agent's live session is a fact")
            .session_id
            .clone();
        let digest = job.action_digest.clone();
        let (until, until_authorizes) = gate_answer(
            &mut gate,
            &with_term(&job, &grant_id, GrantTerm::UntilRevoked),
            &grant_id,
        );
        let (session, _) = gate_answer(
            &mut gate,
            &with_term(
                &job,
                &grant_id,
                GrantTerm::Session {
                    session_id: session,
                },
            ),
            &grant_id,
        );
        let (one, _) = gate_answer(
            &mut gate,
            &with_term(
                &job,
                &grant_id,
                GrantTerm::OneRequest {
                    action_digest: digest,
                },
            ),
            &grant_id,
        );
        let row = format!(
            "{:<48} proposed {:<13} until_revoked {:<14} session {:<14} one_request {:?}",
            cell.name,
            proposed,
            format!("{until:?}"),
            format!("{session:?}"),
            one
        );
        if proposed != cell.proposed
            || until != Some(cell.lasting)
            || session != Some(cell.lasting)
            || one != Some(cell.one_request)
            || (proposed == "until_revoked" && !until_authorizes)
        {
            wrong.push(row.clone());
        }
        rows.push(row);
    }
    println!("LIVE the duration table against the rule theory:");
    for row in &rows {
        println!("LIVE   {row}");
    }
    gate.stop_within(Duration::from_secs(20))
        .expect("the gate stops");
    assert!(wrong.is_empty(), "cells that disagree: {wrong:#?}");
}
