//! An in-process driver of the broker's authority core (`fabric_broker_core::Broker`), shared by
//! the tests that need a card the real `open_card` opened and the fresh-permit job the real core
//! hands out after an approval (tests p09 and p10 of the CD-W04 design, D40).
//!
//! It is a small copy of the core's own test harness (`crates/fabric-broker-core/tests/common`):
//! the public test key, a temporary data directory of its own, a clock the test moves, an
//! Isabelle prover and a provider attached, and a scripted classification. No prover runs here:
//! the test answers the classification itself and takes the fresh-permit job from the output.
//! Seeding is for debug builds only (`fabric_broker_core::Seed`), so every user of this driver
//! is too.

#![allow(
    dead_code,
    reason = "each test binary uses a part of the shared driver"
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use fabric_broker_core::ports::{
    AgentInput, ConnectionId, ProofDone, ProofJob, ProverIdentity, ProverKind, ProverStatus,
};
use fabric_broker_core::{
    Attachments, Broker, Input, Job, Output, Seed, SeedAgent, Settings, format_timestamp,
};
use fabric_desktop_contracts::owner::{Account, Destination, ProofPurpose, ProofReport, Ruling};
use fabric_desktop_contracts::scalar::{Id, Label};
use fabric_desktop_contracts::{Entry, Frame, decode, decode_record};
use fabric_storage::{OpenOptions, Store, TestKey};
use serde_json::{Value, json};

/// The start time of every run: 2026-11-20T16:00:00.000Z, as in the core's tests.
pub const T0: u64 = 1_795_190_400_000;
/// The enrolled agent.
pub const AGENT: &str = "agent-a";
/// The connected account.
pub const ACCOUNT: &str = "acct-1";
/// Its plain selected calendar.
pub const CALENDAR: &str = "cal-primary";
/// Its Agent Drafts calendar.
pub const DRAFTS: &str = "cal-drafts";
/// The agent's connection.
const CONNECTION: u64 = 1;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// A directory under the system's temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> TempDir {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fabric-broker-driver-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a fresh temporary directory");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An id the test knows is valid.
#[must_use]
pub fn id(value: &str) -> Id {
    Id::new(value).unwrap_or_else(|e| panic!("{value} is not an id: {e}"))
}

/// The bytes of a JSON value, for `decode_record`, which reads a contract record from them as
/// the contract reads it.
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a JSON value serializes")
}

/// The arguments of a `propose_create` on `ACCOUNT` and `calendar`.
#[must_use]
pub fn create_params(calendar: &str) -> Value {
    json!({
        "account_id": ACCOUNT,
        "calendar_id": calendar,
        "event": {
            "title": "Plan the week",
            "start": {"date_time": "2026-11-27T10:00:00+01:00", "zone": "Europe/Berlin"},
            "end": {"date_time": "2026-11-27T11:00:00+01:00", "zone": "Europe/Berlin"},
            "description": null,
            "location": null
        },
        "task": "Plan the week."
    })
}

/// The connected `ACCOUNT` with `CALENDAR` as a plain calendar and `DRAFTS` as its Agent Drafts
/// calendar, both selected and owned.
#[must_use]
pub fn account() -> Account {
    let value = json!({
        "account_id": ACCOUNT,
        "revision": 1,
        "state": "connected",
        "email": "owner@example.com",
        "calendars": [
            {"calendar_id": CALENDAR, "name": "Calendar", "access": "owner", "selected": true,
             "agent_drafts": false},
            {"calendar_id": DRAFTS, "name": "Agent Drafts", "access": "owner", "selected": true,
             "agent_drafts": true}
        ]
    });
    decode_record(&bytes(&value)).unwrap_or_else(|e| panic!("not an account ({e:?}): {value}"))
}

/// One broker core in its own data directory, with an Isabelle prover and a provider attached
/// (the attachments of a run that can carry out a change, D40), `AGENT` enrolled and
/// connected, and `account()` seeded.
pub struct Core {
    _dir: TempDir,
    broker: Broker,
    /// The clock the test moves.
    pub now: u64,
    counter: u64,
}

impl Core {
    /// Starts the core at `T0`, seeds `AGENT` and `account()`, and connects `AGENT`.
    #[must_use]
    pub fn start(label: &str) -> Core {
        let dir = TempDir::new(label);
        let data_dir = dir.0.join("data");
        let opener = Box::new(move |now_ms| {
            Store::open(OpenOptions {
                data_dir: data_dir.clone(),
                key: Box::new(TestKey),
                now_ms,
            })
        });
        let attachments = Attachments {
            prover: Some(ProverIdentity {
                kind: ProverKind::Isabelle,
                health: ProverStatus::Ready,
            }),
            provider_present: true,
        };
        let (broker, _) = Broker::start(
            opener,
            Settings::cd_w04("0.1.0 CD-W04 test".to_owned()),
            attachments,
            T0,
        )
        .unwrap_or_else(|e| panic!("the core does not start: {e}"));
        let mut core = Core {
            _dir: dir,
            broker,
            now: T0,
            counter: 0,
        };
        let seed = Seed {
            agents: vec![SeedAgent {
                agent_id: id(AGENT),
                registered_name: Label::new(AGENT).expect("a label"),
                destination: Destination::Local {
                    endpoint: Label::new("in-process").expect("a label"),
                },
            }],
            accounts: vec![account()],
        };
        let _ = core.broker.seed(&seed, core.now);
        let _ = core.handle(Input::Agent(AgentInput::Connect {
            connection: ConnectionId(CONNECTION),
            agent_id: id(AGENT),
        }));
        core
    }

    fn handle(&mut self, input: Input) -> Output {
        self.broker.handle(input, self.now)
    }

    fn next(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }

    /// Sends one owner request; returns its response.
    pub fn owner(&mut self, method: &str, params: Value) -> Value {
        let id = self.next("owner");
        let line = serde_json::to_vec(&json!({
            "protocol": "fabric-desktop/1",
            "id": id,
            "method": method,
            "params": params
        }))
        .expect("a JSON value serializes");
        let output = self.handle(Input::OwnerLine(line));
        output
            .owner
            .iter()
            .find(|f| matches!(f, Frame::OwnerResponse(_)) && f.to_value()["id"] == json!(id))
            .unwrap_or_else(|| panic!("no owner response to {method}"))
            .to_value()
    }

    /// The result of an owner request that must succeed.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let response = self.owner(method, params);
        match response.get("result") {
            Some(result) => result.clone(),
            None => panic!("expected a result: {response}"),
        }
    }

    /// The snapshot.
    pub fn snapshot(&mut self) -> Value {
        self.ok("get_snapshot", json!({}))
    }

    /// Activates the profile `profile_id` at the profiles revision of the snapshot.
    pub fn activate(&mut self, profile_id: &str) {
        let revision = self.snapshot()["profiles"]["revision"].clone();
        let _ = self.ok(
            "save_profile",
            json!({
                "expected_revision": revision,
                "change": {"kind": "activate", "profile_id": profile_id}
            }),
        );
    }

    /// Saves a custom profile with `rows`, acknowledging its expansion when it widens, and
    /// activates it.
    pub fn activate_custom(&mut self, name: &str, rows: &Value) {
        let revision = self.snapshot()["profiles"]["revision"].clone();
        let save = |acknowledged: Value| {
            json!({"expected_revision": revision,
                   "change": {"kind": "save_custom",
                              "profile": {"profile_id": null, "name": name, "rows": rows},
                              "acknowledged_expansion": acknowledged}})
        };
        let response = self.owner("save_profile", save(Value::Null));
        let answer = match response.get("error") {
            Some(refusal) => {
                assert_eq!(refusal["code"], json!("expansion_not_acknowledged"));
                self.ok(
                    "save_profile",
                    save(refusal["detail"]["expansion_digest"].clone()),
                )
            }
            None => response["result"].clone(),
        };
        let profile_id = answer["profiles"]["items"]
            .as_array()
            .expect("profiles")
            .iter()
            .find(|p| p["name"] == json!(name))
            .expect("the saved profile")["profile_id"]
            .as_str()
            .expect("a profile id")
            .to_owned();
        self.activate(&profile_id);
    }

    /// The redacted details of a request.
    pub fn details(&mut self, request_id: &str) -> Value {
        self.ok(
            "get_request_details",
            json!({"request_id": request_id, "redaction": "redacted"}),
        )
    }

    /// The duration the card of `request_id` proposes.
    pub fn proposed(&mut self, request_id: &str) -> String {
        self.details(request_id)["card"]["proposed_duration"]["kind"]
            .as_str()
            .expect("the request has a card")
            .to_owned()
    }

    /// One agent call of `tool` whose classification the test answers `need_approval`: the
    /// request waits in `pending_approval` with a card the real `open_card` opened.
    pub fn pending(&mut self, tool: &str, params: Value) -> String {
        let call_id = self.next("call");
        let line = serde_json::to_vec(&json!({
            "protocol": "fabric-desktop/1",
            "id": call_id,
            "method": tool,
            "params": params
        }))
        .expect("a JSON value serializes");
        let request = match decode(Entry::AgentRequest, &line) {
            Ok(Frame::AgentRequest(request)) => request,
            other => panic!("not an agent request: {other:?}"),
        };
        let output = self.handle(Input::Agent(AgentInput::Call {
            connection: ConnectionId(CONNECTION),
            request,
        }));
        let reply = output
            .agent
            .iter()
            .map(|r| r.frame.to_value())
            .find(|r| r["id"] == json!(call_id))
            .unwrap_or_else(|| panic!("no reply to the {tool} call"));
        let request_id = reply["result"]["request"]["request_id"]
            .as_str()
            .unwrap_or_else(|| panic!("the reply names no request: {reply}"))
            .to_owned();
        let job = job_for(&output, &request_id, ProofPurpose::Classification);
        let _ = self.handle(Input::ProofDone(need_approval(&job)));
        assert_eq!(
            self.details(&request_id)["request"]["status"]["state"],
            json!("pending_approval"),
            "{tool}"
        );
        request_id
    }

    /// Approves a pending request for `duration` at its current revision and digest; returns
    /// the new grant's id and the fresh-permit job the approval started.
    pub fn approve(&mut self, request_id: &str, duration: &str) -> (String, ProofJob) {
        let details = self.details(request_id);
        let request = &details["request"];
        let line = serde_json::to_vec(&json!({
            "protocol": "fabric-desktop/1",
            "id": self.next("decide"),
            "method": "decide_request",
            "params": {
                "request_id": request_id,
                "request_digest": request["request_digest"],
                "expected_revision": request["revision"],
                "decision": "approve",
                "selected": null,
                "duration": {"kind": duration}
            }
        }))
        .expect("a JSON value serializes");
        let output = self.handle(Input::OwnerLine(line));
        let response = output
            .owner
            .iter()
            .find(|f| matches!(f, Frame::OwnerResponse(_)))
            .expect("an answer to decide_request")
            .to_value();
        let grant = response["result"]["approval"]["grant_id"]
            .as_str()
            .unwrap_or_else(|| panic!("the approval was refused: {response}"))
            .to_owned();
        (
            grant,
            job_for(&output, request_id, ProofPurpose::FreshPermit),
        )
    }
}

/// The one proof job of `request_id` for `purpose` in `output`.
#[must_use]
pub fn job_for(output: &Output, request_id: &str, purpose: ProofPurpose) -> ProofJob {
    let jobs: Vec<&ProofJob> = output
        .jobs
        .iter()
        .filter_map(|j| match j {
            Job::Prove(p) if p.request_id.as_str() == request_id && p.purpose == purpose => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(
        jobs.len(),
        1,
        "expected one {purpose:?} job for {request_id}"
    );
    jobs[0].clone()
}

/// A scripted `need_approval` result for `job`, as the core's tests write one.
fn need_approval(job: &ProofJob) -> ProofDone {
    let proof_id = format!("proof-{}", job.job_id);
    let report = json!({
        "proof_id": proof_id,
        "request_id": job.request_id,
        "purpose": job.purpose,
        "outcome": "need_approval",
        "failing_rule": "test_rule",
        "rule_verdicts": [],
        "isabelle_version": "Isabelle2025",
        "theory_hash": format!("sha256:{}", "a".repeat(64)),
        "context_digest": format!("sha256:{}", "b".repeat(64)),
        "action_digest": job.action_digest,
        "started_at": format_timestamp(job.started_at_ms),
        "elapsed_ms": 10
    });
    let report: ProofReport = decode_record(&bytes(&report)).expect("a proof report");
    let ruling = json!({
        "rule": "test_rule",
        "explanation": "The test classification asks the owner.",
        "proof_id": proof_id
    });
    let ruling: Ruling = decode_record(&bytes(&ruling)).expect("a ruling");
    ProofDone {
        job_id: job.job_id,
        request_id: job.request_id.clone(),
        purpose: job.purpose,
        report: Some(report),
        ruling: Some(ruling),
        authorizing_grants: Vec::new(),
        undecided_reason: None,
        check_theory_text: None,
        cause: None,
    }
}
