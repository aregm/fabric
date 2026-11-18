//! The harness the core acceptance tests share (design section 9.3).
//!
//! Every test runs the broker in process with the public test key, in its own temporary
//! directory, and moves its clock by hand. It drives the broker through `Broker::handle`, plays
//! the prover and the provider from `Output.jobs`, and reads the store through the public API of
//! `fabric-storage` once the broker is stopped. A second SQLite connection, keyed with the test
//! key, injects faults (design 7.2) and reads the ledger and audit tables of section 3.5. No
//! test touches the Keychain or the network.

#![allow(
    dead_code,
    reason = "each test binary uses a part of the shared harness"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fabric_broker_core::ports::{
    AgentInput, AttemptDone, AttemptReport, ConnectionId, DispatchOrder, ProofDone, ProofJob,
    ProverCause, ProverIdentity, ProverKind, ProverStatus,
};
use fabric_broker_core::{
    Attachments, Broker, Input, Job, Opener, Output, Seed, SeedAgent, Settings, format_timestamp,
};
use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::common::{FailureCode, ToolName};
use fabric_desktop_contracts::owner::{
    Account, Destination, ProofPurpose, ProofReport, ProverProblem, Ruling,
};
use fabric_desktop_contracts::scalar::{Explanation, Id, Label};
use fabric_desktop_contracts::{Entry, Frame, decode, decode_record};
use fabric_storage::{
    Actor, DATABASE_FILE, Mutation, OpMeta, OpenOptions, ReadTx, Store, StoreFault, TEST_KEY,
    TestKey, TxError, UndecidedCause,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// The start time of every test (design section 9): 2026-11-20T16:00:00.000Z.
pub const T0: u64 = 1_795_190_400_000;
/// The agent most tests use.
pub const AGENT: &str = "agent-a";
/// A second agent.
pub const OTHER: &str = "agent-b";
/// The account most tests use.
pub const ACCOUNT: &str = "acct-1";
/// Its selected calendar.
pub const CALENDAR: &str = "cal-primary";
/// Its Agent Drafts calendar, seeded by `setup`. Under Careful a create there proposes
/// `until_revoked` (D40).
pub const DRAFTS: &str = "cal-drafts";
/// The agent tool of every proposal the tests make.
pub const CREATE: &str = "fabric_calendar_propose_create";

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
static NEXT_PROOF: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temporary directory, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    #[must_use]
    pub fn new(label: &str) -> TempDir {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fabric-broker-core-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        TempDir { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// An id the tests know is valid.
#[must_use]
pub fn id(value: &str) -> Id {
    Id::new(value).unwrap_or_else(|e| panic!("{value} is not an id: {e}"))
}

/// A label the tests know is valid.
#[must_use]
pub fn label(value: &str) -> Label {
    Label::new(value).unwrap_or_else(|e| panic!("{value} is not a label: {e}"))
}

/// A contract record read from JSON as the contract reads it.
#[must_use]
pub fn record<T: DeserializeOwned>(value: &Value) -> T {
    let bytes = serde_json::to_vec(value).expect("a JSON value serializes");
    decode_record(&bytes).unwrap_or_else(|e| panic!("not a contract record ({e}): {value}"))
}

/// The contract text of a timestamp.
#[must_use]
pub fn timestamp(unix_ms: u64) -> Value {
    serde_json::to_value(format_timestamp(unix_ms)).expect("a timestamp serializes")
}

/// An Isabelle prover and a provider are attached.
#[must_use]
pub fn isabelle() -> Attachments {
    Attachments {
        prover: Some(ProverIdentity {
            kind: ProverKind::Isabelle,
            health: ProverStatus::Ready,
        }),
        provider_present: true,
    }
}

/// An Isabelle prover that is still starting, as the release shell attaches it, and a provider.
#[must_use]
pub fn isabelle_starting() -> Attachments {
    Attachments {
        prover: Some(ProverIdentity {
            kind: ProverKind::Isabelle,
            health: ProverStatus::Starting,
        }),
        provider_present: true,
    }
}

/// An Isabelle prover is attached; no provider is.
#[must_use]
pub fn isabelle_without_provider() -> Attachments {
    Attachments {
        provider_present: false,
        ..isabelle()
    }
}

/// The synthetic classifier of the debug builds is attached, and a provider.
#[must_use]
pub fn synthetic() -> Attachments {
    Attachments {
        prover: Some(ProverIdentity {
            kind: ProverKind::Synthetic,
            health: ProverStatus::Unavailable(Some(ProverProblem::NotConfigured)),
        }),
        provider_present: true,
    }
}

/// Nothing is attached.
#[must_use]
pub fn no_prover() -> Attachments {
    Attachments {
        prover: None,
        provider_present: false,
    }
}

/// An opener on `data_dir` with the public test key.
#[must_use]
pub fn opener(data_dir: PathBuf) -> Opener {
    Box::new(move |now_ms| {
        Store::open(OpenOptions {
            data_dir: data_dir.clone(),
            key: Box::new(TestKey),
            now_ms,
        })
    })
}

/// A connected account whose calendars are all selected.
#[must_use]
pub fn account(account_id: &str, calendars: &[&str]) -> Account {
    let calendars: Vec<Value> = calendars
        .iter()
        .map(|calendar_id| {
            json!({
                "calendar_id": calendar_id,
                "name": "Calendar",
                "access": "owner",
                "selected": true,
                "agent_drafts": false
            })
        })
        .collect();
    record(&json!({
        "account_id": account_id,
        "revision": 1,
        "state": "connected",
        "email": "owner@example.com",
        "calendars": calendars
    }))
}

/// A connected account whose calendars are all selected: `calendars` as plain calendars and
/// `drafts` as its Agent Drafts calendar.
#[must_use]
pub fn account_with_drafts(account_id: &str, calendars: &[&str], drafts: &str) -> Account {
    let mut value = serde_json::to_value(account(account_id, calendars)).expect("an account");
    value["calendars"]
        .as_array_mut()
        .expect("calendars")
        .push(json!({
            "calendar_id": drafts,
            "name": "Agent Drafts",
            "access": "owner",
            "selected": true,
            "agent_drafts": true
        }));
    record(&value)
}

/// The arguments of a `propose_create` on `account` and `calendar`.
#[must_use]
pub fn create_params(account: &str, calendar: &str, title: &str) -> Value {
    json!({
        "account_id": account,
        "calendar_id": calendar,
        "event": {
            "title": title,
            "start": {"date_time": "2026-11-27T10:00:00+01:00", "zone": "Europe/Berlin"},
            "end": {"date_time": "2026-11-27T11:00:00+01:00", "zone": "Europe/Berlin"},
            "description": null,
            "location": null
        },
        "task": "Plan the week."
    })
}

/// The action of a `propose_create`, read as the contract reads it.
#[must_use]
pub fn create_action(params: Value) -> Action {
    Action::from_parts::<serde_json::Error>(ToolName::ProposeCreate, params)
        .expect("the arguments are an action")
        .expect("a create is an action")
}

/// One owner request line.
#[must_use]
pub fn owner_line(id: &str, method: &str, params: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "protocol": "fabric-desktop/1",
        "id": id,
        "method": method,
        "params": params
    }))
    .expect("a JSON value serializes")
}

/// The owner response with `id` in `output`.
#[must_use]
pub fn response_frame<'a>(output: &'a Output, id: &str) -> &'a Frame {
    output
        .owner
        .iter()
        .find(|f| matches!(f, Frame::OwnerResponse(_)) && f.to_value()["id"] == json!(id))
        .unwrap_or_else(|| panic!("no owner response to {id}"))
}

/// The owner response with `id` in `output`, as JSON.
#[must_use]
pub fn response(output: &Output, id: &str) -> Value {
    response_frame(output, id).to_value()
}

/// The result of a response; panics on an error.
#[must_use]
pub fn result(response: &Value) -> Value {
    match response.get("result") {
        Some(result) => result.clone(),
        None => panic!("expected a result: {response}"),
    }
}

/// The error of a response; panics on a result.
#[must_use]
pub fn error(response: &Value) -> Value {
    match response.get("error") {
        Some(error) => error.clone(),
        None => panic!("expected an error: {response}"),
    }
}

/// The owner notices in `output`, in order, as JSON.
#[must_use]
pub fn notices(output: &Output) -> Vec<Value> {
    output
        .owner
        .iter()
        .filter(|f| matches!(f, Frame::OwnerNotice(_)))
        .map(Frame::to_value)
        .collect()
}

/// The notices in `output` whose change is of `kind`.
#[must_use]
pub fn notices_of(output: &Output, kind: &str) -> Vec<Value> {
    notices(output)
        .into_iter()
        .filter(|n| n["change"]["kind"] == json!(kind))
        .collect()
}

/// The replies on `connection` in `output`, as JSON.
#[must_use]
pub fn agent_replies(output: &Output, connection: u64) -> Vec<Value> {
    output
        .agent
        .iter()
        .filter(|r| r.connection == ConnectionId(connection))
        .map(|r| r.frame.to_value())
        .collect()
}

/// The request id an agent reply names; panics on an error.
#[must_use]
pub fn request_id_of(reply: &Value) -> String {
    reply["result"]["request"]["request_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the reply names no request: {reply}"))
        .to_owned()
}

/// The proof jobs in `output`.
#[must_use]
pub fn proof_jobs(output: &Output) -> Vec<ProofJob> {
    output
        .jobs
        .iter()
        .filter_map(|j| match j {
            Job::Prove(p) => Some(p.clone()),
            Job::Dispatch(_) => None,
        })
        .collect()
}

/// The dispatch orders in `output`.
#[must_use]
pub fn dispatches(output: &Output) -> Vec<DispatchOrder> {
    output
        .jobs
        .iter()
        .filter_map(|j| match j {
            Job::Dispatch(d) => Some(d.clone()),
            Job::Prove(_) => None,
        })
        .collect()
}

/// The proof jobs of `request_id` in `output`.
#[must_use]
pub fn jobs_of(output: &Output, request_id: &str) -> Vec<ProofJob> {
    proof_jobs(output)
        .into_iter()
        .filter(|j| j.request_id.as_str() == request_id)
        .collect()
}

/// The one proof job of `request_id` for `purpose` in `output`.
#[must_use]
pub fn job_for(output: &Output, request_id: &str, purpose: ProofPurpose) -> ProofJob {
    let jobs: Vec<ProofJob> = jobs_of(output, request_id)
        .into_iter()
        .filter(|j| j.purpose == purpose)
        .collect();
    assert_eq!(
        jobs.len(),
        1,
        "expected one {purpose:?} job for {request_id}, found {}",
        jobs.len()
    );
    jobs.into_iter().next().expect("one job")
}

/// The ids of the grants a job's facts offer, with their `ended` mark.
#[must_use]
pub fn offered_grants(job: &ProofJob) -> Vec<(String, bool)> {
    job.facts
        .grants
        .iter()
        .map(|f| (f.grant.grant_id.as_str().to_owned(), f.ended))
        .collect()
}

/// The relied list of a job's facts.
#[must_use]
pub fn relied(job: &ProofJob) -> Vec<String> {
    job.facts
        .relied
        .iter()
        .map(|g| g.as_str().to_owned())
        .collect()
}

/// Ids as strings.
#[must_use]
pub fn strings(ids: &[Id]) -> Vec<String> {
    ids.iter().map(|i| i.as_str().to_owned()).collect()
}

/// What the test prover answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Permit,
    NeedApproval,
    Block,
    Undecided,
}

/// The rule a test Block or NeedApproval names.
pub const RULE: &str = "test_rule";

/// A prover result for `job`: a report, a ruling for Block and NeedApproval, and `grants` as the
/// authorizing grants.
#[must_use]
pub fn proof_done(job: &ProofJob, outcome: Outcome, grants: &[&str]) -> ProofDone {
    let n = NEXT_PROOF.fetch_add(1, Ordering::Relaxed);
    let proof_id = format!("proof-{}-{n}", job.job_id);
    let (name, rule) = match outcome {
        Outcome::Permit => ("permit", None),
        Outcome::NeedApproval => ("need_approval", Some(RULE)),
        Outcome::Block => ("block", Some(RULE)),
        Outcome::Undecided => ("undecided", None),
    };
    let report: ProofReport = record(&json!({
        "proof_id": proof_id,
        "request_id": job.request_id,
        "purpose": job.purpose,
        "outcome": name,
        "failing_rule": rule,
        "rule_verdicts": [],
        "isabelle_version": "Isabelle2025",
        "theory_hash": format!("sha256:{}", "a".repeat(64)),
        "context_digest": format!("sha256:{}", "b".repeat(64)),
        "action_digest": job.action_digest,
        "started_at": timestamp(job.started_at_ms),
        "elapsed_ms": 10
    }));
    let ruling = rule.map(|rule| {
        record::<Ruling>(&json!({
            "rule": rule,
            "explanation": "The test prover says so.",
            "proof_id": proof_id
        }))
    });
    ProofDone {
        job_id: job.job_id,
        request_id: job.request_id.clone(),
        purpose: job.purpose,
        report: Some(report),
        ruling,
        authorizing_grants: grants.iter().map(|g| id(g)).collect(),
        undecided_reason: None,
        check_theory_text: None,
        cause: None,
    }
}

/// A prover result for `job` with no report: no check ran, for `cause` (`None`: a prover that
/// broke the port's rule and stated none).
#[must_use]
pub fn no_report_done(job: &ProofJob, cause: Option<ProverCause>) -> ProofDone {
    ProofDone {
        job_id: job.job_id,
        request_id: job.request_id.clone(),
        purpose: job.purpose,
        report: None,
        ruling: None,
        authorizing_grants: Vec::new(),
        undecided_reason: Some("the test prover did not check".to_owned()),
        check_theory_text: None,
        cause,
    }
}

/// A prover result for `job` whose report is undecided, stating `cause`.
#[must_use]
pub fn undecided_done(job: &ProofJob, cause: Option<ProverCause>) -> ProofDone {
    ProofDone {
        undecided_reason: Some("the test prover did not decide".to_owned()),
        cause,
        ..proof_done(job, Outcome::Undecided, &[])
    }
}

/// The report id of a prover result.
#[must_use]
pub fn proof_id(done: &ProofDone) -> String {
    done.report
        .as_ref()
        .expect("the result carries a report")
        .proof_id
        .as_str()
        .to_owned()
}

/// A provider refusal.
#[must_use]
pub fn rejected() -> AttemptReport {
    AttemptReport::Rejected {
        code: FailureCode::ProviderRejected,
        explanation: Explanation::new("Google refused the change.").expect("an explanation"),
    }
}

/// One ledger row, read with a second connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    pub seq: i64,
    pub operation_type: String,
    pub committed_at_ms: i64,
    pub correlation_id: Option<String>,
}

/// One move to `undecided` in the ledger, read with a second connection.
struct UndecidedMove {
    seq: i64,
    correlation_id: Option<String>,
    request_id: String,
    /// The JSON type of `undecided_cause`; `None` when the member is absent.
    kind: Option<String>,
    cause: Option<String>,
}

/// One audit row, read with a second connection.
#[derive(Clone, Debug, PartialEq)]
pub struct AuditRow {
    pub kind: String,
    pub actor_agent_id: Option<String>,
    pub detail: Value,
}

/// One broker in one data directory, with a clock the test moves.
pub struct Harness {
    tmp: TempDir,
    pub data_dir: PathBuf,
    pub attachments: Attachments,
    broker: Option<Broker>,
    /// The raw wall clock passed to the broker.
    pub now: u64,
    counter: u64,
}

impl Harness {
    /// A new data directory (a child of the temporary directory that does not exist yet) and a
    /// broker started on it at `T0`.
    #[must_use]
    pub fn new(label: &str, attachments: Attachments) -> Harness {
        let tmp = TempDir::new(label);
        let data_dir = tmp.path().join("data");
        let mut h = Harness {
            tmp,
            data_dir,
            attachments,
            broker: None,
            now: T0,
            counter: 0,
        };
        let _ = h.start();
        h
    }

    /// The broker's settings: `Limits::CD_G0` and `RetryPolicy::PLAN`.
    #[must_use]
    pub fn settings() -> Settings {
        Settings::cd_w04("0.1.0 CD-W04 test".to_owned())
    }

    /// Starts a broker on the data directory at `now`.
    pub fn start(&mut self) -> Output {
        assert!(self.broker.is_none(), "a broker is already running");
        let (broker, output) = Broker::start(
            opener(self.data_dir.clone()),
            Harness::settings(),
            self.attachments.clone(),
            self.now,
        )
        .unwrap_or_else(|e| panic!("the broker does not start: {e}"));
        self.broker = Some(broker);
        output
    }

    /// Drops the broker without `quit`: a kill after its last commit (design 7.2).
    pub fn kill(&mut self) {
        self.broker = None;
    }

    /// A kill and a new start on the same directory.
    pub fn restart(&mut self) -> Output {
        self.kill();
        self.start()
    }

    /// End of input (design 5.8).
    pub fn quit(&mut self) -> Output {
        let now = self.now;
        let output = self.broker_mut().quit(now);
        self.broker = None;
        output
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.broker.is_some()
    }

    #[must_use]
    pub fn broker(&self) -> &Broker {
        self.broker.as_ref().expect("the broker is running")
    }

    pub fn broker_mut(&mut self) -> &mut Broker {
        self.broker.as_mut().expect("the broker is running")
    }

    pub fn advance(&mut self, ms: u64) {
        self.now += ms;
    }

    fn next(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }

    pub fn handle(&mut self, input: Input) -> Output {
        let now = self.now;
        self.broker_mut().handle(input, now)
    }

    pub fn tick(&mut self) -> Output {
        self.handle(Input::Tick)
    }

    /// Seeds `agents` and `accounts` (debug builds; design 5.10, 10.4).
    pub fn seed(&mut self, agents: &[&str], accounts: Vec<Account>) -> Output {
        let seed = Seed {
            agents: agents
                .iter()
                .map(|a| SeedAgent {
                    agent_id: id(a),
                    registered_name: label(a),
                    destination: Destination::Local {
                        endpoint: label("in-process"),
                    },
                })
                .collect(),
            accounts,
        };
        let now = self.now;
        self.broker_mut().seed(&seed, now)
    }

    /// Seeds `AGENT` and `ACCOUNT` with `CALENDAR` and the Agent Drafts calendar `DRAFTS`, and
    /// connects `AGENT` on connection 1.
    pub fn setup(&mut self) {
        let _ = self.seed(
            &[AGENT],
            vec![account_with_drafts(ACCOUNT, &[CALENDAR], DRAFTS)],
        );
        let _ = self.connect(1, AGENT);
    }

    // Owner channel.

    /// Sends one owner line with `id`; returns its response and the whole output.
    pub fn send(&mut self, id: &str, method: &str, params: Value) -> (Value, Output) {
        let output = self.handle(Input::OwnerLine(owner_line(id, method, params)));
        let response = response(&output, id);
        (response, output)
    }

    /// Sends one owner request with a fresh id.
    pub fn owner(&mut self, method: &str, params: Value) -> (Value, Output) {
        let id = self.next("owner");
        self.send(&id, method, params)
    }

    /// The result of an owner request that must succeed.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        result(&self.owner(method, params).0)
    }

    /// The error of an owner request that must be refused.
    pub fn refused(&mut self, method: &str, params: Value) -> Value {
        error(&self.owner(method, params).0)
    }

    pub fn snapshot(&mut self) -> Value {
        self.ok("get_snapshot", json!({}))
    }

    /// Subscribes from the snapshot's cursor; returns that cursor.
    pub fn subscribe(&mut self) -> u64 {
        let cursor = self.snapshot()["cursor"].as_u64().expect("a cursor");
        let answer = self.ok("subscribe", json!({"cursor": cursor}));
        assert_eq!(answer["cursor"], json!(cursor));
        cursor
    }

    /// The redacted details of a request.
    pub fn details(&mut self, request_id: &str) -> Value {
        self.ok(
            "get_request_details",
            json!({"request_id": request_id, "redaction": "redacted"}),
        )
    }

    /// The state of a request, from its details.
    pub fn state(&mut self, request_id: &str) -> String {
        self.details(request_id)["request"]["status"]["state"]
            .as_str()
            .expect("a state")
            .to_owned()
    }

    /// The agent `agent_id` in the snapshot.
    pub fn agent_view(&mut self, agent_id: &str) -> Value {
        self.snapshot()["agents"]
            .as_array()
            .expect("agents")
            .iter()
            .find(|a| a["agent_id"] == json!(agent_id))
            .cloned()
            .unwrap_or_else(|| panic!("no agent {agent_id} in the snapshot"))
    }

    /// The live grants in the snapshot.
    pub fn live_grants(&mut self) -> Vec<String> {
        self.snapshot()["grants"]
            .as_array()
            .expect("grants")
            .iter()
            .map(|g| g["grant_id"].as_str().expect("a grant id").to_owned())
            .collect()
    }

    /// Revokes a live grant at its current revision; returns the result.
    pub fn revoke(&mut self, grant_id: &str) -> Value {
        let grants = self.snapshot()["grants"].clone();
        let grant = grants
            .as_array()
            .expect("grants")
            .iter()
            .find(|g| g["grant_id"] == json!(grant_id))
            .cloned()
            .unwrap_or_else(|| panic!("no live grant {grant_id}"));
        self.ok(
            "revoke_grant",
            json!({"grant_id": grant_id, "expected_revision": grant["revision"]}),
        )
    }

    /// Pauses or resumes one agent at its current revision.
    pub fn pause(&mut self, agent_id: &str, paused: bool) -> Value {
        let revision = self.agent_view(agent_id)["revision"].clone();
        self.ok(
            "pause_agent",
            json!({
                "target": {"kind": "agent", "agent_id": agent_id, "expected_revision": revision},
                "paused": paused
            }),
        )
    }

    /// Creates a recipient set; returns its id. Raises the revocation epoch (design 5.6).
    pub fn create_group(&mut self, name: &str, members: &[&str]) -> Value {
        self.ok(
            "save_group",
            json!({
                "expected_revision": 0,
                "change": {
                    "kind": "save",
                    "recipient_set_id": null,
                    "definition": {"name": name, "members": members, "domains": []}
                }
            }),
        )["recipient_set"]
            .clone()
    }

    /// Changes the fence of every proof in flight, through a new recipient set.
    pub fn change_fence(&mut self) {
        let name = self.next("Group");
        let _ = self.create_group(&name, &[]);
    }

    /// Activates the profile `profile_id` at the profiles revision of the snapshot.
    pub fn activate(&mut self, profile_id: &str) -> Value {
        let revision = self.snapshot()["profiles"]["revision"].clone();
        self.ok(
            "save_profile",
            json!({
                "expected_revision": revision,
                "change": {"kind": "activate", "profile_id": profile_id}
            }),
        )
    }

    /// Decides a pending request at its current revision and digest.
    pub fn decide(&mut self, request_id: &str, decision: &str, duration: Value) -> (Value, Output) {
        let details = self.details(request_id);
        let request = &details["request"];
        self.owner(
            "decide_request",
            json!({
                "request_id": request_id,
                "request_digest": request["request_digest"],
                "expected_revision": request["revision"],
                "decision": decision,
                "selected": null,
                "duration": duration
            }),
        )
    }

    // Agent channel.

    pub fn connect(&mut self, connection: u64, agent_id: &str) -> Output {
        self.handle(Input::Agent(AgentInput::Connect {
            connection: ConnectionId(connection),
            agent_id: id(agent_id),
        }))
    }

    pub fn disconnect(&mut self, connection: u64) -> Output {
        self.handle(Input::Agent(AgentInput::Disconnect {
            connection: ConnectionId(connection),
        }))
    }

    /// One agent call; returns the reply to it and the whole output.
    pub fn call(
        &mut self,
        connection: u64,
        call_id: &str,
        method: &str,
        params: Value,
    ) -> (Value, Output) {
        let output = self.send_call(connection, call_id, method, params);
        let reply = agent_replies(&output, connection)
            .into_iter()
            .find(|r| r["id"] == json!(call_id))
            .unwrap_or_else(|| panic!("no reply to {call_id}"));
        (reply, output)
    }

    /// One agent call, whether or not it is answered.
    pub fn send_call(
        &mut self,
        connection: u64,
        call_id: &str,
        method: &str,
        params: Value,
    ) -> Output {
        let line = serde_json::to_vec(&json!({
            "protocol": "fabric-desktop/1",
            "id": call_id,
            "method": method,
            "params": params
        }))
        .expect("a JSON value serializes");
        let request = match decode(Entry::AgentRequest, &line) {
            Ok(Frame::AgentRequest(request)) => request,
            other => panic!("not an agent request: {other:?}"),
        };
        self.handle(Input::Agent(AgentInput::Call {
            connection: ConnectionId(connection),
            request,
        }))
    }

    /// A `propose_create` on `account` and `calendar`; returns the request id.
    pub fn propose_on(
        &mut self,
        connection: u64,
        call_id: &str,
        account: &str,
        calendar: &str,
    ) -> (String, Output) {
        let (reply, output) = self.call(
            connection,
            call_id,
            CREATE,
            create_params(account, calendar, call_id),
        );
        (request_id_of(&reply), output)
    }

    /// A `propose_create` on `ACCOUNT` and `CALENDAR`.
    pub fn propose(&mut self, connection: u64, call_id: &str) -> (String, Output) {
        self.propose_on(connection, call_id, ACCOUNT, CALENDAR)
    }

    /// A status poll of `request_id`.
    pub fn poll(&mut self, connection: u64, call_id: &str, request_id: &str) -> (Value, Output) {
        self.call(
            connection,
            call_id,
            "fabric_request_status",
            json!({"request_id": request_id}),
        )
    }

    // Prover and provider.

    pub fn prove_with(&mut self, done: ProofDone) -> Output {
        self.handle(Input::ProofDone(done))
    }

    pub fn prove(&mut self, job: &ProofJob, outcome: Outcome, grants: &[&str]) -> Output {
        self.prove_with(proof_done(job, outcome, grants))
    }

    pub fn attempt(&mut self, order: &DispatchOrder, report: AttemptReport) -> Output {
        let done = AttemptDone {
            request_id: order.request_id.clone(),
            effect_id: order.effect_id.clone(),
            attempt: order.attempt,
            report,
            finished_at_ms: self.now,
        };
        self.handle(Input::Attempt(done))
    }

    // Flows.

    /// A proposal on `account` whose classification asks the owner: the request waits in
    /// `pending_approval`.
    pub fn pending_on(
        &mut self,
        connection: u64,
        call_id: &str,
        account: &str,
        calendar: &str,
    ) -> String {
        let (request_id, output) = self.propose_on(connection, call_id, account, calendar);
        let job = job_for(&output, &request_id, ProofPurpose::Classification);
        let _ = self.prove(&job, Outcome::NeedApproval, &[]);
        assert_eq!(self.state(&request_id), "pending_approval");
        request_id
    }

    pub fn pending(&mut self, connection: u64, call_id: &str) -> String {
        self.pending_on(connection, call_id, ACCOUNT, CALENDAR)
    }

    /// A pending create on the Agent Drafts calendar, whose card proposes `until_revoked` under
    /// Careful in a run with an Isabelle prover and a provider.
    pub fn pending_lasting(&mut self, connection: u64, call_id: &str) -> String {
        self.pending_on(connection, call_id, ACCOUNT, DRAFTS)
    }

    /// Approves a pending request for `duration`; returns the new grant and the fresh-permit
    /// job the approval started.
    pub fn approve(&mut self, request_id: &str, duration: &str) -> (String, ProofJob) {
        let (response, output) = self.decide(request_id, "approve", json!({"kind": duration}));
        let grant = result(&response)["approval"]["grant_id"]
            .as_str()
            .expect("the approval names its grant")
            .to_owned();
        let job = job_for(&output, request_id, ProofPurpose::FreshPermit);
        (grant, job)
    }

    /// A proposal on `account`, asked and approved for `duration`.
    pub fn approved_on(
        &mut self,
        connection: u64,
        call_id: &str,
        account: &str,
        calendar: &str,
        duration: &str,
    ) -> (String, String, ProofJob) {
        let request_id = self.pending_on(connection, call_id, account, calendar);
        let (grant, job) = self.approve(&request_id, duration);
        (request_id, grant, job)
    }

    /// A proposal asked and approved for `duration`. A lasting duration (`session`,
    /// `until_revoked`) is approved on the Agent Drafts calendar, the only one whose card
    /// proposes it under Careful; `one_request` on `CALENDAR`.
    pub fn approved(
        &mut self,
        connection: u64,
        call_id: &str,
        duration: &str,
    ) -> (String, String, ProofJob) {
        let calendar = if duration == "one_request" {
            CALENDAR
        } else {
            DRAFTS
        };
        self.approved_on(connection, call_id, ACCOUNT, calendar, duration)
    }

    /// Feeds a fresh Permit naming `grants`; returns the one dispatch order that follows.
    pub fn dispatch(&mut self, job: &ProofJob, grants: &[&str]) -> DispatchOrder {
        let output = self.prove(job, Outcome::Permit, grants);
        assert!(output.halt.is_none(), "the permit halted the broker");
        let orders = dispatches(&output);
        assert_eq!(orders.len(), 1, "expected one dispatch order");
        let order = orders.into_iter().next().expect("one order");
        assert_eq!(order.request_id, job.request_id);
        order
    }

    // Store.

    /// The store, opened directly with the test key; the broker must be stopped.
    #[must_use]
    pub fn open_store(&self) -> Store {
        Store::open(OpenOptions {
            data_dir: self.data_dir.clone(),
            key: Box::new(TestKey),
            now_ms: i64::try_from(self.now).expect("a time fits i64"),
        })
        .unwrap_or_else(|e| panic!("the store does not open: {e}"))
    }

    /// One read of the stopped broker's store.
    pub fn read<T>(&mut self, f: impl FnOnce(&ReadTx<'_>) -> Result<T, StoreFault>) -> T {
        assert!(
            self.broker.is_none(),
            "stop the broker before reading its store"
        );
        let store = self.open_store();
        let value = store
            .read(f)
            .unwrap_or_else(|e| panic!("the read failed: {e}"));
        store.close().expect("the store closes");
        value
    }

    /// One write of the stopped broker's store, as the broker's seed writes it.
    pub fn write(&mut self, mutations: Vec<Mutation>) {
        assert!(
            self.broker.is_none(),
            "stop the broker before writing its store"
        );
        let mut store = self.open_store();
        let meta = OpMeta {
            operation_type: "broker.seed",
            actor: Actor::broker(),
            idempotency: None,
            causation_id: None,
            correlation_id: None,
            now_ms: i64::try_from(self.now).expect("a time fits i64"),
        };
        store
            .write(meta, |tx| {
                for mutation in mutations {
                    tx.apply(mutation)?;
                }
                Ok::<(), TxError<()>>(())
            })
            .unwrap_or_else(|e| panic!("the write failed: {e:?}"));
        store.close().expect("the store closes");
    }

    /// Stops the broker and checks the ledger chain, the replay (design section 9) and every move
    /// to `undecided` (c34).
    pub fn verify(&mut self) {
        self.kill();
        self.check_undecided_moves();
        let store = self.open_store();
        store
            .verify_chain()
            .unwrap_or_else(|e| panic!("the chain does not verify: {e}"));
        store
            .verify_replay()
            .unwrap_or_else(|e| panic!("the replay does not match: {e}"));
        store.close().expect("the store closes");
    }

    // A second connection (design 7.2).

    /// A second connection to `broker.db`, keyed with the test key on macOS.
    #[must_use]
    pub fn raw(&self) -> rusqlite::Connection {
        let connection = rusqlite::Connection::open(self.data_dir.join(DATABASE_FILE))
            .expect("open a second connection");
        #[cfg(target_os = "macos")]
        {
            let hex: String = TEST_KEY.iter().map(|b| format!("{b:02x}")).collect();
            connection
                .execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
                .expect("key the second connection");
        }
        #[cfg(not(target_os = "macos"))]
        let _ = TEST_KEY;
        connection
            .busy_timeout(std::time::Duration::from_secs(10))
            .expect("set a busy timeout");
        connection
    }

    /// c34. 5.7: every move to `undecided` in the ledger states a cause this build knows, as text, in a
    /// row correlated to the request it moves, where the owner's details read it back.
    pub fn check_undecided_moves(&self) {
        let connection = self.raw();
        let mut statement = connection
            .prepare(
                "SELECT l.seq, l.correlation_id, json_extract(m.value, '$.request_id'), \
                 json_type(m.value, '$.undecided_cause'), json_extract(m.value, '$.undecided_cause') \
                 FROM ledger AS l, json_each(l.payload, '$.mutations') AS m \
                 WHERE json_extract(m.value, '$.op') = 'request_moved' \
                 AND json_extract(m.value, '$.status.state') = 'undecided' ORDER BY l.seq",
            )
            .expect("read the moves to undecided");
        let moves: Vec<UndecidedMove> = statement
            .query_map([], |row| {
                Ok(UndecidedMove {
                    seq: row.get(0)?,
                    correlation_id: row.get(1)?,
                    request_id: row.get(2)?,
                    kind: row.get(3)?,
                    cause: row.get(4)?,
                })
            })
            .expect("query the moves to undecided")
            .collect::<Result<_, _>>()
            .expect("read the moves to undecided");
        for UndecidedMove {
            seq,
            correlation_id,
            request_id,
            kind,
            cause,
        } in moves
        {
            assert_eq!(
                kind.as_deref(),
                Some("text"),
                "row {seq}: the move of {request_id} to undecided states no cause"
            );
            let cause = cause.unwrap_or_default();
            assert!(
                UndecidedCause::of_name(&cause).is_some(),
                "row {seq}: {request_id} names an unknown cause {cause}"
            );
            assert_eq!(
                correlation_id.as_deref(),
                Some(request_id.as_str()),
                "row {seq}: the move of {request_id} to undecided is correlated elsewhere"
            );
        }
    }

    /// The cause text stored on the newest move of `request_id` to `undecided` in a row
    /// correlated to it; `None` when there is none or it is not text.
    #[must_use]
    pub fn stored_cause(&self, request_id: &str) -> Option<String> {
        self.raw()
            .query_row(
                "SELECT CASE WHEN json_type(m.value, '$.undecided_cause') = 'text' \
                 THEN json_extract(m.value, '$.undecided_cause') END \
                 FROM ledger AS l, json_each(l.payload, '$.mutations') AS m \
                 WHERE l.correlation_id = ?1 \
                 AND json_extract(m.value, '$.op') = 'request_moved' \
                 AND json_extract(m.value, '$.request_id') = ?1 \
                 AND json_extract(m.value, '$.status.state') = 'undecided' \
                 ORDER BY l.seq DESC, m.key DESC LIMIT 1",
                [request_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
            .expect("read the stored cause")
    }

    /// Inserts a forged ledger row through a second connection while the broker runs: `payload`
    /// as its payload text, correlated to `correlation_id`, actor broker, its own operation id and
    /// envelope hash, a sequence far above the head, and the last row's commit time, so the
    /// broker's next commit is not refused for time. Nothing checks it: the chain does not verify
    /// afterwards, so a test that forges ends without `verify`.
    pub fn forge_ledger_row(&self, payload: &str, correlation_id: &str) {
        let connection = self.raw();
        let (head, committed_at_ms): (i64, i64) = connection
            .query_row(
                "SELECT max(seq), max(committed_at_ms) FROM ledger",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read the ledger head");
        let seq = head + 1_000_000;
        let operation_id = format!("op-forged-{seq}");
        let payload_hash = fabric_desktop_contracts::digest::sha256_hex(payload);
        let envelope_hash =
            fabric_desktop_contracts::digest::sha256_hex(&format!("forged\n{operation_id}"));
        connection
            .execute(
                "INSERT INTO ledger (seq, operation_id, operation_type, payload_version, \
                 committed_at_ms, run_id, actor_kind, actor_agent_id, idempotency_scope, \
                 idempotency_key, causation_id, correlation_id, payload, payload_hash, \
                 previous_hash, envelope_hash) \
                 VALUES (?1, ?2, 'request.proof', 1, ?3, NULL, 'broker', NULL, NULL, NULL, NULL, \
                 ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    seq,
                    operation_id,
                    committed_at_ms,
                    correlation_id,
                    payload,
                    payload_hash,
                    "0".repeat(64),
                    envelope_hash
                ],
            )
            .expect("insert the forged ledger row");
    }

    /// A trigger that fails every ledger insert matching `when` (SQL on `NEW`).
    pub fn inject_when(&self, name: &str, when: &str) {
        self.raw()
            .execute_batch(&format!(
                "CREATE TRIGGER fault_{name} BEFORE INSERT ON ledger WHEN {when} \
                 BEGIN SELECT RAISE(ABORT, 'injected'); END;"
            ))
            .expect("create the fault trigger");
    }

    /// A trigger that fails every commit of `operation_type` (design 7.2).
    pub fn inject_fault(&self, operation_type: &str) {
        let name: String = operation_type
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.inject_when(&name, &format!("NEW.operation_type = '{operation_type}'"));
    }

    /// Drops every fault trigger.
    pub fn clear_faults(&self) {
        let connection = self.raw();
        let names: Vec<String> = {
            let mut statement = connection
                .prepare(
                    "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND name LIKE 'fault_%'",
                )
                .expect("list the fault triggers");
            statement
                .query_map([], |row| row.get(0))
                .expect("query the fault triggers")
                .collect::<Result<_, _>>()
                .expect("read the fault triggers")
        };
        for name in names {
            connection
                .execute_batch(&format!("DROP TRIGGER {name};"))
                .expect("drop a fault trigger");
        }
    }

    /// Every ledger row, in order.
    #[must_use]
    pub fn ledger(&self) -> Vec<LedgerRow> {
        let connection = self.raw();
        let mut statement = connection
            .prepare(
                "SELECT seq, operation_type, committed_at_ms, correlation_id FROM ledger ORDER BY seq",
            )
            .expect("read the ledger");
        statement
            .query_map([], |row| {
                Ok(LedgerRow {
                    seq: row.get(0)?,
                    operation_type: row.get(1)?,
                    committed_at_ms: row.get(2)?,
                    correlation_id: row.get(3)?,
                })
            })
            .expect("query the ledger")
            .collect::<Result<_, _>>()
            .expect("read the ledger rows")
    }

    /// The highest ledger sequence, 0 when empty.
    #[must_use]
    pub fn ledger_head(&self) -> i64 {
        self.raw()
            .query_row("SELECT coalesce(max(seq), 0) FROM ledger", [], |row| {
                row.get(0)
            })
            .expect("read the ledger head")
    }

    /// Every audit row, in order.
    #[must_use]
    pub fn audit(&self) -> Vec<AuditRow> {
        let connection = self.raw();
        let mut statement = connection
            .prepare("SELECT kind, actor_agent_id, detail FROM audit ORDER BY audit_id")
            .expect("read the audit table");
        statement
            .query_map([], |row| {
                let detail: String = row.get(2)?;
                Ok(AuditRow {
                    kind: row.get(0)?,
                    actor_agent_id: row.get(1)?,
                    detail: serde_json::from_str(&detail).unwrap_or(Value::Null),
                })
            })
            .expect("query the audit table")
            .collect::<Result<_, _>>()
            .expect("read the audit rows")
    }

    /// The void reason of an authorization (column `void_reason`, section 3.5).
    #[must_use]
    pub fn void_reason(&self, authorization_id: &str) -> Option<String> {
        self.raw()
            .query_row(
                "SELECT void_reason FROM authorizations WHERE authorization_id = ?1",
                [authorization_id],
                |row| row.get(0),
            )
            .expect("read the authorization")
    }

    /// The number of attempt rows of the store.
    #[must_use]
    pub fn attempt_rows(&self) -> i64 {
        self.raw()
            .query_row("SELECT count(*) FROM attempts", [], |row| row.get(0))
            .expect("count the attempts")
    }
}
