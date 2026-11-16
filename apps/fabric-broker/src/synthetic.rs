//! The synthetic agent and its classifier (design 5.10). Debug builds only.
//!
//! They let a person see requests before an agent can connect (CD-W06) and before a rule prover
//! runs (CD-W03 wiring): the agent is an in-process caller of the same agent entry an MCP bridge
//! will use, so its requests are stored, numbered and survive a restart like any other. The
//! broker starts it only with `FABRIC_BROKER_SYNTHETIC_AGENT=1` and the test key, so a synthetic
//! record never reaches a store opened with the Keychain.
//!
//! - [`SyntheticClassifier`] is the attached prover. It can answer only `need_approval` (for a
//!   classification) or `undecided` (for a fresh permit), so nothing it says can authorize, and
//!   the core refuses a permit from any prover that is not Isabelle anyway.
//! - [`SyntheticAgent`] does nothing until the owner's page has subscribed once. Then it enrolls
//!   its agent and account, connects, proposes an event one second later, polls the request's
//!   status once a second while it is open, and proposes again one second after it ended: two
//!   proposals per run at most.

use fabric_broker_core::ports::{
    AgentInput, ConnectionId, ProofDone, ProofJob, Prover, ProverIdentity, ProverKind,
};
use fabric_broker_core::{Broker, Input, Output, Seed, SeedAgent, format_timestamp, ids};
use fabric_desktop_contracts::agent::{AgentRequest, AgentResponse};
use fabric_desktop_contracts::owner::{
    Account, Destination, ProofPurpose, ProofReport, ProverHealth, Ruling,
};
use fabric_desktop_contracts::scalar::{Id, Label};
use fabric_desktop_contracts::{Entry, Frame, canonical, decode, decode_record};
use serde_json::{Value, json};

/// The synthetic agent's id.
pub const AGENT_ID: &str = "synthetic-agent";
/// Its enrolled name, as the owner sees it.
pub const AGENT_NAME: &str = "Synthetic agent (test build)";
/// The synthetic account.
pub const ACCOUNT_ID: &str = "synthetic-account";
/// The account's one calendar, selected for agents.
pub const CALENDAR_ID: &str = "synthetic-calendar";
/// The title of every event the synthetic agent proposes. Tests look for it where no event
/// content may appear (stdout notices, stderr).
pub const TITLE: &str = "Synthetic planning block (test build, marker 5f1c)";
/// The rule a synthetic classification names.
pub const RULE: &str = "synthetic_no_rule_theory";
/// What the synthetic agent proposes at most in one run.
pub const PROPOSALS_PER_RUN: u32 = 2;
/// The wait after connecting before the first proposal, after a request ended before the next,
/// and between two status polls.
pub const PACE_MS: u64 = 1_000;

/// The prover version a synthetic report names.
const PROVER_VERSION: &str = "synthetic (no prover)";
const RULE_EXPLANATION: &str = "Synthetic: this test build runs no rule prover, so no grant can be shown to cover the request. A real broker states here why the current grants are not enough.";
const UNDECIDED_REASON: &str =
    "The synthetic classifier never permits; this test build runs no rule prover.";
const TASK: &str = "Please hold an hour to plan next week. (Synthetic: the test build's synthetic agent wrote this, not a real agent.)";

/// What the synthetic classifier can answer. It has no permit, by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyntheticVerdict {
    NeedApproval,
    Undecided,
}

/// The prover of a synthetic run: a classification asks the owner, a fresh permit is undecided.
#[derive(Clone, Copy, Debug, Default)]
pub struct SyntheticClassifier;

impl SyntheticClassifier {
    /// The verdict for a proof of `purpose`.
    fn verdict(purpose: ProofPurpose) -> SyntheticVerdict {
        match purpose {
            ProofPurpose::Classification => SyntheticVerdict::NeedApproval,
            ProofPurpose::FreshPermit => SyntheticVerdict::Undecided,
        }
    }
}

impl Prover for SyntheticClassifier {
    /// Kind `synthetic`; the owner sees the prover as unavailable.
    fn identity(&self) -> ProverIdentity {
        ProverIdentity {
            kind: ProverKind::Synthetic,
            health: ProverHealth::Unavailable,
        }
    }

    /// A report with no rule verdicts, a ruling for a classification, and no authorizing grant.
    fn prove(&mut self, job: &ProofJob) -> ProofDone {
        let verdict = SyntheticClassifier::verdict(job.purpose);
        let proof_id = ids::new_id("synthetic-proof");
        let (outcome, failing_rule) = match verdict {
            SyntheticVerdict::NeedApproval => ("need_approval", Some(RULE)),
            SyntheticVerdict::Undecided => ("undecided", None),
        };
        // What the classifier looked at: the job, not the facts, which it does not read.
        let context = canonical(&json!({
            "job_id": job.job_id,
            "request_id": job.request_id,
            "purpose": job.purpose,
            "action_digest": job.action_digest,
        }));
        let report: ProofReport = decode_record(&bytes(&json!({
            "proof_id": proof_id,
            "request_id": job.request_id,
            "purpose": job.purpose,
            "outcome": outcome,
            "failing_rule": failing_rule,
            "rule_verdicts": [],
            "isabelle_version": PROVER_VERSION,
            "theory_hash": sha256(PROVER_VERSION),
            "context_digest": sha256(&context),
            "action_digest": job.action_digest,
            "started_at": format_timestamp(job.started_at_ms),
            "elapsed_ms": 0
        })))
        .expect(VALID);
        let ruling = failing_rule.map(|rule| {
            decode_record::<Ruling>(&bytes(&json!({
                "rule": rule,
                "explanation": RULE_EXPLANATION,
                "proof_id": proof_id
            })))
            .expect(VALID)
        });
        ProofDone {
            job_id: job.job_id,
            request_id: job.request_id.clone(),
            purpose: job.purpose,
            report: Some(report),
            ruling,
            authorizing_grants: Vec::new(),
            undecided_reason: match verdict {
                SyntheticVerdict::NeedApproval => None,
                SyntheticVerdict::Undecided => Some(UNDECIDED_REASON.to_owned()),
            },
            check_theory_text: None,
        }
    }
}

/// `sha256:<hex>` of a text.
fn sha256(text: &str) -> String {
    format!(
        "sha256:{}",
        fabric_desktop_contracts::digest::sha256_hex(text)
    )
}

/// The bytes of a contract record built from fixed synthetic values, for `decode_record`.
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a JSON value serializes")
}

/// What `decode_record` says of a record built from fixed synthetic values.
const VALID: &str = "the synthetic values make a valid contract record";

fn id(text: &str) -> Id {
    Id::new(text).expect("a valid synthetic id")
}

/// The request the synthetic agent is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Waiting {
    /// Nothing is open.
    Nothing,
    /// A proposal was sent with this call id; its reply names the request.
    Proposal(Id),
    /// This request is open.
    Request(Id),
}

/// The in-process synthetic agent of a debug build.
#[derive(Debug)]
pub struct SyntheticAgent {
    /// True once the seed and the connection were made.
    started: bool,
    /// False after the broker closed the connection; the agent then stops for the run.
    connected: bool,
    waiting: Waiting,
    /// When the next proposal is due.
    next_proposal_ms: Option<u64>,
    /// When the next status poll is due.
    next_poll_ms: Option<u64>,
    /// Proposals made in this run.
    proposals: u32,
    /// The time of the last tick, for what a reply schedules.
    now_ms: u64,
}

impl Default for SyntheticAgent {
    fn default() -> SyntheticAgent {
        SyntheticAgent::new()
    }
}

impl SyntheticAgent {
    /// The agent's one connection.
    pub const CONNECTION: ConnectionId = ConnectionId(1);

    /// An agent that has done nothing yet.
    #[must_use]
    pub fn new() -> SyntheticAgent {
        SyntheticAgent {
            started: false,
            connected: false,
            waiting: Waiting::Nothing,
            next_proposal_ms: None,
            next_poll_ms: None,
            proposals: 0,
            now_ms: 0,
        }
    }

    /// The synthetic agent and account; created once, then found again (`broker.seed`).
    #[must_use]
    pub fn seed() -> Seed {
        let account: Account = decode_record(&bytes(&json!({
            "account_id": ACCOUNT_ID,
            "revision": 1,
            "state": "connected",
            "email": "synthetic@example.invalid",
            "calendars": [{
                "calendar_id": CALENDAR_ID,
                "name": "Synthetic calendar (test build)",
                "access": "owner",
                "selected": true,
                "agent_drafts": false
            }]
        })))
        .expect(VALID);
        Seed {
            agents: vec![SeedAgent {
                agent_id: id(AGENT_ID),
                registered_name: Label::new(AGENT_NAME).expect("a valid synthetic label"),
                destination: Destination::Local {
                    endpoint: Label::new("in-process").expect("a valid synthetic label"),
                },
            }],
            accounts: vec![account],
        }
    }

    /// When the agent has something to do next, if anything.
    #[must_use]
    pub fn next_due_ms(&self) -> Option<u64> {
        if !self.started || !self.connected {
            return None;
        }
        match self.waiting {
            Waiting::Nothing => self.next_proposal_ms,
            Waiting::Proposal(_) => None,
            Waiting::Request(_) => self.next_poll_ms,
        }
    }

    /// Nothing until `broker.subscribed()`; then seeds and connects, proposes when due and it has no open request
    /// (at most 2 per run), and polls `fabric_request_status` once a second while one is open.
    pub fn tick(&mut self, broker: &mut Broker, now_ms: u64) -> Output {
        self.now_ms = now_ms;
        let mut out = Output::default();
        if !broker.subscribed() || !broker.store_ready() {
            return out;
        }
        if !self.started {
            self.started = true;
            self.connected = true;
            merge(&mut out, broker.seed(&SyntheticAgent::seed(), now_ms));
            let connect = AgentInput::Connect {
                connection: SyntheticAgent::CONNECTION,
                agent_id: id(AGENT_ID),
            };
            merge(&mut out, broker.handle(Input::Agent(connect), now_ms));
            self.next_proposal_ms = Some(now_ms + PACE_MS);
        }
        self.take_replies(&mut out);
        if out.halt.is_some() || !self.connected {
            return out;
        }
        let due = |at: Option<u64>| at.is_some_and(|at| at <= now_ms);
        match self.waiting.clone() {
            Waiting::Nothing
                if due(self.next_proposal_ms) && self.proposals < PROPOSALS_PER_RUN =>
            {
                self.proposals += 1;
                self.next_proposal_ms = None;
                let call_id = ids::new_id("synthetic-call");
                self.waiting = Waiting::Proposal(call_id.clone());
                let call = proposal(&call_id, now_ms);
                merge(&mut out, broker.handle(self.call(call), now_ms));
            }
            Waiting::Request(request_id) if due(self.next_poll_ms) => {
                self.next_poll_ms = Some(now_ms + PACE_MS);
                let poll = status_poll(&ids::new_id("synthetic-poll"), &request_id);
                merge(&mut out, broker.handle(self.call(poll), now_ms));
            }
            _ => {}
        }
        self.take_replies(&mut out);
        out
    }

    fn call(&self, request: AgentRequest) -> Input {
        Input::Agent(AgentInput::Call {
            connection: SyntheticAgent::CONNECTION,
            request,
        })
    }

    /// Takes the replies on `CONNECTION` out of `output.agent` (request ids, statuses).
    pub fn take_replies(&mut self, output: &mut Output) {
        if output.close.contains(&SyntheticAgent::CONNECTION) {
            output.close.retain(|c| *c != SyntheticAgent::CONNECTION);
            self.connected = false;
        }
        let replies: Vec<Frame> = {
            let (mine, others) = std::mem::take(&mut output.agent)
                .into_iter()
                .partition(|r| r.connection == SyntheticAgent::CONNECTION);
            output.agent = others;
            mine.into_iter().map(|r| r.frame).collect::<Vec<_>>()
        };
        for frame in replies {
            self.read_reply(&frame);
        }
    }

    /// Learns the request a proposal made and whether it is still open.
    fn read_reply(&mut self, frame: &Frame) {
        let Frame::AgentResponse(response) = frame else {
            return;
        };
        let (call_id, status) = match response {
            AgentResponse::Result { id, result, .. } => (Some(id), Some(&result.request)),
            AgentResponse::Error { id, .. } => (id.as_ref(), None),
        };
        let open = |status: &fabric_desktop_contracts::agent::RequestStatus| {
            !status.status.state().is_terminal()
        };
        match (&self.waiting, status) {
            (Waiting::Proposal(sent), Some(status)) if Some(sent) == call_id => {
                if open(status) {
                    self.waiting = Waiting::Request(status.request_id.clone());
                    self.next_poll_ms = Some(self.now_ms + PACE_MS);
                } else {
                    self.ended();
                }
            }
            (Waiting::Proposal(sent), None) if Some(sent) == call_id => self.ended(),
            (Waiting::Request(request_id), Some(status)) if &status.request_id == request_id => {
                if !open(status) {
                    self.ended();
                }
            }
            // A poll refused (the request is not found, the agent is paused): stop waiting.
            (Waiting::Request(_), None) => self.ended(),
            _ => {}
        }
    }

    /// The open request ended, or the proposal made none: the next one comes after a pause.
    fn ended(&mut self) {
        self.waiting = Waiting::Nothing;
        self.next_poll_ms = None;
        self.next_proposal_ms =
            (self.proposals < PROPOSALS_PER_RUN).then_some(self.now_ms + PACE_MS);
    }
}

/// Appends `next` to `out`.
fn merge(out: &mut Output, next: Output) {
    out.owner.extend(next.owner);
    out.agent.extend(next.agent);
    out.close.extend(next.close);
    out.jobs.extend(next.jobs);
    out.stderr.extend(next.stderr);
    if out.halt.is_none() {
        out.halt = next.halt;
    }
}

/// One agent request frame from its parts.
///
/// # Panics
///
/// When the frame is not a valid agent request, which the fixed values rule out.
fn agent_request(call_id: &Id, method: &str, params: Value) -> AgentRequest {
    let line = serde_json::to_vec(&json!({
        "protocol": "fabric-desktop/1",
        "id": call_id,
        "method": method,
        "params": params
    }))
    .expect("a JSON value serializes");
    match decode(Entry::AgentRequest, &line) {
        Ok(Frame::AgentRequest(request)) => request,
        _ => panic!("the synthetic values make a valid agent request"),
    }
}

/// A `propose_create` of a one-hour block a week from now, on the hour, in UTC.
fn proposal(call_id: &Id, now_ms: u64) -> AgentRequest {
    const HOUR_MS: u64 = 3_600_000;
    let start_ms = (now_ms / HOUR_MS + 7 * 24) * HOUR_MS;
    let at = |ms: u64| {
        let text = format_timestamp(ms);
        // "YYYY-MM-DDTHH:MM:SS.mmmZ" -> "YYYY-MM-DDTHH:MM:SS+00:00".
        format!("{}+00:00", &text.as_str()[..19])
    };
    agent_request(
        call_id,
        "fabric_calendar_propose_create",
        json!({
            "account_id": ACCOUNT_ID,
            "calendar_id": CALENDAR_ID,
            "event": {
                "title": TITLE,
                "start": {"date_time": at(start_ms), "zone": "Etc/UTC"},
                "end": {"date_time": at(start_ms + HOUR_MS), "zone": "Etc/UTC"},
                "description": null,
                "location": null
            },
            "task": TASK
        }),
    )
}

/// A `fabric_request_status` of `request_id`.
fn status_poll(call_id: &Id, request_id: &Id) -> AgentRequest {
    agent_request(
        call_id,
        "fabric_request_status",
        json!({"request_id": request_id}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proposal_is_a_valid_create_on_the_synthetic_calendar() {
        let request = proposal(&id("call-1"), 1_795_190_400_123);
        let value = Frame::AgentRequest(request).to_value();
        assert_eq!(value["method"], "fabric_calendar_propose_create");
        assert_eq!(value["params"]["event"]["title"], TITLE);
        assert_eq!(
            value["params"]["event"]["start"]["date_time"],
            "2026-11-27T16:00:00+00:00"
        );
        assert_eq!(
            value["params"]["event"]["end"]["date_time"],
            "2026-11-27T17:00:00+00:00"
        );
    }

    #[test]
    fn the_seed_names_the_synthetic_agent_and_account() {
        let seed = SyntheticAgent::seed();
        assert_eq!(seed.agents.len(), 1);
        assert_eq!(seed.agents[0].agent_id.as_str(), AGENT_ID);
        assert_eq!(seed.accounts.len(), 1);
        assert_eq!(seed.accounts[0].account_id.as_str(), ACCOUNT_ID);
        assert_eq!(seed.accounts[0].calendars.len(), 1);
    }

    #[test]
    fn the_classifier_never_answers_a_permit() {
        assert_eq!(
            SyntheticClassifier::verdict(ProofPurpose::Classification),
            SyntheticVerdict::NeedApproval
        );
        assert_eq!(
            SyntheticClassifier::verdict(ProofPurpose::FreshPermit),
            SyntheticVerdict::Undecided
        );
    }

    #[test]
    fn a_new_agent_has_nothing_due() {
        assert_eq!(SyntheticAgent::new().next_due_ms(), None);
    }
}
