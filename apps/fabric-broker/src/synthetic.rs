//! The synthetic agent and its classifier (design 5.10). Debug builds only.
//!
//! They let a person see requests before an agent can connect (CD-W06), with or without the
//! rule prover: the agent is an in-process caller of the same agent entry an MCP bridge will
//! use, so its requests are stored, numbered and survive a restart like any other. The
//! broker starts it only with `FABRIC_BROKER_SYNTHETIC_AGENT=1` and the test key, so a synthetic
//! record never reaches a store opened with the Keychain.
//!
//! - [`SyntheticClassifier`] is the attached prover of a synthetic run without Isabelle. It can
//!   answer only `need_approval` (for a classification) or `undecided` (for a fresh permit), so
//!   nothing it says can authorize, and the core refuses a permit from any prover that is not
//!   Isabelle anyway.
//! - [`SyntheticAgent`] does nothing until the owner's page has subscribed once. Then it enrolls
//!   its agent and account, connects, proposes an event one second later, polls the request's
//!   status once a second while it is open, and proposes again one second after it ended: two
//!   proposals per run at most.
//! - [`SyntheticAgent::practice`] is the same agent on the practice calendar of a run with the
//!   rule prover: it proposes only while the prover is ready, and runs the
//!   practice script, a create (A), a delete of the event Fabric did not make (B), an update (C)
//!   and a delete (D) of A's event, four proposals at most.

use fabric_broker_core::ports::{
    AgentInput, ConnectionId, ProofDone, ProofJob, Prover, ProverIdentity, ProverKind,
};
use fabric_broker_core::{Broker, Input, Output, Seed, SeedAgent, format_timestamp, ids};
use fabric_desktop_contracts::agent::{AgentRequest, AgentResponse};
use fabric_desktop_contracts::lifecycle::RequestState;
use fabric_desktop_contracts::owner::{
    Account, Destination, ProofPurpose, ProofReport, ProverHealth, Ruling,
};
use fabric_desktop_contracts::scalar::{Id, Label};
use fabric_desktop_contracts::{Entry, Frame, canonical, decode, decode_record};
use serde_json::{Value, json};

use crate::practice::{FOREIGN_EVENT_ID, PracticeCalendar, PracticeEvent};

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
/// What the synthetic agent proposes at most in one run without the practice calendar.
pub const PROPOSALS_PER_RUN: u32 = 2;
/// What the synthetic agent proposes at most in one practice run: the four steps of the script.
pub const PRACTICE_PROPOSALS: u32 = 4;
/// The wait after connecting before the first proposal, after a request ended before the next,
/// and between two status polls.
pub const PACE_MS: u64 = 1_000;

/// The prover version a synthetic report names.
const PROVER_VERSION: &str = "synthetic (no prover)";
const RULE_EXPLANATION: &str = "Synthetic: this test build runs no rule prover, so no grant can be shown to cover the request. A real broker states here why the current grants are not enough.";
const UNDECIDED_REASON: &str =
    "The synthetic classifier never permits; this test build runs no rule prover.";
const TASK: &str = "Please hold an hour to plan next week. (Synthetic: the test build's synthetic agent wrote this, not a real agent.)";
const MOVE_TASK: &str = "Please move the planning block half an hour later. (Synthetic: the test build's synthetic agent wrote this, not a real agent.)";
const DELETE_TASK: &str = "Please remove this event. (Synthetic: the test build's synthetic agent wrote this, not a real agent.)";
/// The stderr line when the practice script ends.
const SCRIPT_FINISHED: &str = "synthetic agent: script finished";
const HOUR_MS: u64 = 3_600_000;
/// How far the update of step C moves A's event.
const MOVE_MS: u64 = 30 * 60_000;

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

/// A step of the practice script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Create an event.
    A,
    /// Delete the event Fabric did not make: blocked.
    B,
    /// Move A's event half an hour later.
    C,
    /// Delete A's event.
    D,
}

/// How a step's request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// In this state: terminal, `applied`, or `outcome_unknown`.
    In(RequestState),
    /// The proposal or a poll was refused, so the agent does not know.
    Refused,
}

/// The practice script's progress.
#[derive(Debug)]
struct Practice {
    /// The practice calendar; the agent reads the ids and revisions of its events from it
    /// (test-build knowledge: no read result reaches an agent yet, D33).
    calendar: PracticeCalendar,
    /// The last prover health; proposals wait for `Ready`.
    health: ProverHealth,
    /// The step proposed next; `None` once no step is left.
    next: Option<Step>,
    /// The step whose proposal or request is open.
    current: Option<Step>,
    /// A's start, computed for its proposal.
    a_start_ms: Option<u64>,
    /// A's request, from the reply to its proposal.
    a_request: Option<Id>,
    /// How A ended.
    a_ended: Option<Ended>,
    /// True once `SCRIPT_FINISHED` was written.
    finished: bool,
}

impl Practice {
    /// A's practice event and A's start, when A was applied and its event is still there.
    fn a_event(&self) -> Option<(PracticeEvent, u64)> {
        if self.a_ended != Some(Ended::In(RequestState::Applied)) {
            return None;
        }
        let event = self
            .calendar
            .event_of_request(self.a_request.as_ref()?)
            .filter(|e| !e.deleted)?;
        Some((event, self.a_start_ms?))
    }

    /// How A ended, for the stderr line.
    fn a_ended_text(&self) -> &'static str {
        match self.a_ended {
            Some(Ended::In(state)) => state.as_str(),
            Some(Ended::Refused) => "without a request",
            None => "open",
        }
    }
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
    /// What the agent proposes at most in this run.
    limit: u32,
    /// The practice script, in a practice run.
    practice: Option<Practice>,
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
            limit: PROPOSALS_PER_RUN,
            practice: None,
            now_ms: 0,
        }
    }

    /// The practice script (A to D, in the module documentation) on `calendar`; at most four
    /// proposals.
    #[must_use]
    pub fn practice(calendar: PracticeCalendar) -> SyntheticAgent {
        SyntheticAgent {
            limit: PRACTICE_PROPOSALS,
            practice: Some(Practice {
                calendar,
                health: ProverHealth::Starting,
                next: Some(Step::A),
                current: None,
                a_start_ms: None,
                a_request: None,
                a_ended: None,
                finished: false,
            }),
            ..SyntheticAgent::new()
        }
    }

    /// The last prover health; practice mode proposes only while it is `Ready`.
    pub fn set_prover_health(&mut self, health: ProverHealth) {
        if let Some(practice) = &mut self.practice {
            practice.health = health;
        }
    }

    /// False in a practice run while the prover is not ready.
    fn may_propose(&self) -> bool {
        self.practice
            .as_ref()
            .is_none_or(|p| p.health == ProverHealth::Ready)
    }

    /// True while the agent has a proposal left to make: under the limit without the practice
    /// calendar, until the script finished with it.
    fn more_to_propose(&self) -> bool {
        match &self.practice {
            None => self.proposals < self.limit,
            Some(practice) => !practice.finished,
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
            Waiting::Nothing if self.may_propose() => self.next_proposal_ms,
            Waiting::Nothing => None,
            Waiting::Proposal(_) => None,
            Waiting::Request(_) => self.next_poll_ms,
        }
    }

    /// Nothing until `broker.subscribed()`; then seeds and connects, proposes when due and it has no open request
    /// (at most 2 per run; in a practice run the script, only while the prover is ready), and polls
    /// `fabric_request_status` once a second while one is open.
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
                if due(self.next_proposal_ms) && self.more_to_propose() && self.may_propose() =>
            {
                if let Some(call) = self.propose(now_ms, &mut out.stderr) {
                    merge(&mut out, broker.handle(self.call(call), now_ms));
                }
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

    /// The next proposal, now due; `None` when the practice script has no step left, after its
    /// stderr lines.
    fn propose(&mut self, now_ms: u64, stderr: &mut Vec<String>) -> Option<AgentRequest> {
        self.next_proposal_ms = None;
        let call_id = ids::new_id("synthetic-call");
        let call = if self.practice.is_some() {
            self.practice_step(&call_id, now_ms, stderr)?
        } else {
            proposal(&call_id, now_ms)
        };
        self.proposals += 1;
        self.waiting = Waiting::Proposal(call_id);
        Some(call)
    }

    /// The proposal of the practice script's next step. C and D need A
    /// applied and its event there; without them they are skipped and B still runs.
    fn practice_step(
        &mut self,
        call_id: &Id,
        now_ms: u64,
        stderr: &mut Vec<String>,
    ) -> Option<AgentRequest> {
        let at_limit = self.proposals >= self.limit;
        let practice = self.practice.as_mut()?;
        if at_limit {
            practice.next = None;
        }
        while let Some(step) = practice.next {
            match step {
                Step::A => {
                    practice.next = Some(Step::B);
                    practice.current = Some(Step::A);
                    practice.a_start_ms = Some(create_start_ms(now_ms));
                    return Some(proposal(call_id, now_ms));
                }
                Step::B => {
                    practice.next = Some(Step::C);
                    practice.current = Some(Step::B);
                    let revision = practice
                        .calendar
                        .event(FOREIGN_EVENT_ID)
                        .map_or_else(|| "1".to_owned(), |e| e.revision);
                    return Some(delete_proposal(call_id, FOREIGN_EVENT_ID, &revision));
                }
                Step::C => match practice.a_event() {
                    Some((event, start_ms)) => {
                        practice.next = Some(Step::D);
                        practice.current = Some(Step::C);
                        return Some(update_proposal(call_id, &event, start_ms + MOVE_MS));
                    }
                    None => {
                        stderr.push(format!(
                            "synthetic agent: step A ended {}; steps C and D are skipped",
                            practice.a_ended_text()
                        ));
                        practice.next = None;
                    }
                },
                Step::D => {
                    practice.next = None;
                    let event = practice
                        .a_request
                        .as_ref()
                        .and_then(|request| practice.calendar.event_of_request(request))
                        .filter(|e| !e.deleted);
                    if let Some(event) = event {
                        practice.current = Some(Step::D);
                        return Some(delete_proposal(call_id, &event.event_id, &event.revision));
                    }
                }
            }
        }
        if !practice.finished {
            practice.finished = true;
            stderr.push(SCRIPT_FINISHED.to_owned());
        }
        None
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
            // the handshake is not implemented in this build (contract schema version 3); the
            // synthetic agent sends none, so an answer to one is taken as a refusal
            AgentResponse::Hello { id, .. } | AgentResponse::Prove { id, .. } => (Some(id), None),
        };
        let practice = self.practice.is_some();
        // In a practice run `applied` and `outcome_unknown` end the wait too: they stay open
        // until reconciliation exists (CD-W05).
        let open = |status: &fabric_desktop_contracts::agent::RequestStatus| {
            let state = status.status.state();
            !(state.is_terminal()
                || practice
                    && matches!(state, RequestState::Applied | RequestState::OutcomeUnknown))
        };
        match (&self.waiting, status) {
            (Waiting::Proposal(sent), Some(status)) if Some(sent) == call_id => {
                if let Some(practice) = &mut self.practice
                    && practice.current == Some(Step::A)
                {
                    practice.a_request = Some(status.request_id.clone());
                }
                if open(status) {
                    self.waiting = Waiting::Request(status.request_id.clone());
                    self.next_poll_ms = Some(self.now_ms + PACE_MS);
                } else {
                    self.ended(Ended::In(status.status.state()));
                }
            }
            (Waiting::Proposal(sent), None) if Some(sent) == call_id => {
                self.ended(Ended::Refused);
            }
            (Waiting::Request(request_id), Some(status)) if &status.request_id == request_id => {
                if !open(status) {
                    self.ended(Ended::In(status.status.state()));
                }
            }
            // A poll refused (the request is not found, the agent is paused): stop waiting.
            (Waiting::Request(_), None) => self.ended(Ended::Refused),
            _ => {}
        }
    }

    /// The open request ended, or the proposal made none: the next one comes after a pause.
    fn ended(&mut self, how: Ended) {
        if let Some(practice) = &mut self.practice
            && practice.current.take() == Some(Step::A)
        {
            practice.a_ended = Some(how);
        }
        self.waiting = Waiting::Nothing;
        self.next_poll_ms = None;
        self.next_proposal_ms = self.more_to_propose().then_some(self.now_ms + PACE_MS);
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

/// The start of the block a create proposes at `now_ms`: a week from now, on the hour.
fn create_start_ms(now_ms: u64) -> u64 {
    (now_ms / HOUR_MS + 7 * 24) * HOUR_MS
}

/// An event time at `ms` in UTC.
fn utc_time(ms: u64) -> Value {
    let text = format_timestamp(ms);
    // "YYYY-MM-DDTHH:MM:SS.mmmZ" -> "YYYY-MM-DDTHH:MM:SS+00:00".
    json!({"date_time": format!("{}+00:00", &text.as_str()[..19]), "zone": "Etc/UTC"})
}

/// A `propose_create` of a one-hour block a week from now, on the hour, in UTC.
fn proposal(call_id: &Id, now_ms: u64) -> AgentRequest {
    let start_ms = create_start_ms(now_ms);
    agent_request(
        call_id,
        "fabric_calendar_propose_create",
        json!({
            "account_id": ACCOUNT_ID,
            "calendar_id": CALENDAR_ID,
            "event": {
                "title": TITLE,
                "start": utc_time(start_ms),
                "end": utc_time(start_ms + HOUR_MS),
                "description": null,
                "location": null
            },
            "task": TASK
        }),
    )
}

/// A `propose_update` that moves `event` to a one-hour block from `start_ms`, notifying nobody.
fn update_proposal(call_id: &Id, event: &PracticeEvent, start_ms: u64) -> AgentRequest {
    agent_request(
        call_id,
        "fabric_calendar_propose_update",
        json!({
            "account_id": ACCOUNT_ID,
            "calendar_id": CALENDAR_ID,
            "event": {"event_id": event.event_id, "revision": event.revision},
            "changes": {
                "title": null,
                "start": utc_time(start_ms),
                "end": utc_time(start_ms + HOUR_MS),
                "description": null,
                "location": null
            },
            "send_updates": "none",
            "task": MOVE_TASK
        }),
    )
}

/// A `propose_delete` of the event `event_id` at `revision`, notifying nobody.
fn delete_proposal(call_id: &Id, event_id: &str, revision: &str) -> AgentRequest {
    agent_request(
        call_id,
        "fabric_calendar_propose_delete",
        json!({
            "account_id": ACCOUNT_ID,
            "calendar_id": CALENDAR_ID,
            "event": {"event_id": event_id, "revision": revision},
            "send_updates": "none",
            "task": DELETE_TASK
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
    use crate::practice::testing::{RESERVED, order};
    use crate::practice::{Faults, PracticeCalendar};
    use fabric_broker_core::ports::{AttemptReport, Provider};
    use fabric_desktop_contracts::agent::{Action, AgentCall};

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

    const NOW_MS: u64 = 1_795_190_400_123;

    /// An agent that has seeded and connected, with a proposal due at `NOW_MS`.
    fn started(mut agent: SyntheticAgent) -> SyntheticAgent {
        agent.started = true;
        agent.connected = true;
        agent.now_ms = NOW_MS;
        agent.next_proposal_ms = Some(NOW_MS);
        agent
    }

    fn practice_agent(calendar: &PracticeCalendar) -> SyntheticAgent {
        let mut agent = started(SyntheticAgent::practice(calendar.clone()));
        agent.set_prover_health(ProverHealth::Ready);
        agent
    }

    /// What `tick` does when a proposal is due, without a broker: the call and the stderr lines.
    fn propose(agent: &mut SyntheticAgent) -> (Option<AgentRequest>, Vec<String>) {
        let due = agent.next_due_ms().expect("a proposal is due");
        assert!(matches!(agent.waiting, Waiting::Nothing));
        assert!(agent.more_to_propose() && agent.may_propose());
        agent.now_ms = due;
        let mut stderr = Vec::new();
        let call = agent.propose(due, &mut stderr);
        (call, stderr)
    }

    fn proposed(agent: &mut SyntheticAgent) -> AgentRequest {
        let (call, stderr) = propose(agent);
        assert_eq!(stderr, Vec::<String>::new());
        call.expect("a proposal")
    }

    fn status(request_id: &str, state: Value) -> Value {
        json!({
            "request_id": request_id,
            "updated_at": "2026-11-20T16:00:01.000Z",
            "status": state
        })
    }

    fn state(name: &str) -> Value {
        json!({"state": name})
    }

    fn pending() -> Value {
        json!({"state": "pending_approval", "approval_expires_at": "2026-11-20T16:10:00.000Z"})
    }

    fn blocked() -> Value {
        json!({
            "state": "blocked",
            "denial": {
                "rule": "fabric_created",
                "explanation": "Only events that Fabric created may be deleted."
            }
        })
    }

    fn response(call_id: &Id, method: &str, request_id: &str, state: Value) -> Frame {
        Frame::AgentResponse(
            serde_json::from_value(json!({
                "protocol": "fabric-desktop/1",
                "id": call_id,
                "method": method,
                "result": {"request": status(request_id, state), "result": null}
            }))
            .expect("a valid agent response"),
        )
    }

    /// The broker's reply to `call`: request `request_id` in `state`.
    fn answer(agent: &mut SyntheticAgent, call: &AgentRequest, request_id: &str, state: Value) {
        let frame = response(&call.id, call.call.method(), request_id, state);
        agent.read_reply(&frame);
    }

    /// The broker's reply to a status poll of `request_id`.
    fn poll(agent: &mut SyntheticAgent, request_id: &str, state: Value) {
        assert!(matches!(&agent.waiting, Waiting::Request(r) if r.as_str() == request_id));
        let frame = response(&id("poll-1"), "fabric_request_status", request_id, state);
        agent.read_reply(&frame);
    }

    fn action_of(call: &AgentRequest) -> Action {
        match &call.call {
            AgentCall::Action(action) => action.clone(),
            AgentCall::RequestStatus(_) | AgentCall::Hello(_) | AgentCall::Prove(_) => {
                panic!("a proposal is an action")
            }
        }
    }

    fn params(call: &AgentRequest) -> Value {
        Frame::AgentRequest(call.clone()).to_value()["params"].clone()
    }

    /// Carries out `call` on the practice calendar as an authorized request would be.
    fn carry_out(calendar: &PracticeCalendar, call: &AgentRequest, request_id: &str) {
        let effect = format!("effect-of-{request_id}");
        let report = calendar
            .provider()
            .dispatch(&order(request_id, &effect, action_of(call)))
            .report;
        assert!(
            matches!(report, AttemptReport::Applied { .. }),
            "{report:?}"
        );
    }

    #[test]
    fn the_practice_script_is_create_block_update_delete() {
        let calendar = PracticeCalendar::new(NOW_MS, Faults::default());
        let mut agent = practice_agent(&calendar);

        // A: the create of today's proposal, asked, approved and applied.
        let a = proposed(&mut agent);
        assert_eq!(
            Frame::AgentRequest(a.clone()).to_value()["params"],
            Frame::AgentRequest(proposal(&a.id, NOW_MS)).to_value()["params"]
        );
        answer(&mut agent, &a, "request-a", pending());
        carry_out(&calendar, &a, "request-a");
        poll(&mut agent, "request-a", state("applied"));

        // B: the delete of the event Fabric did not make, blocked.
        let b = proposed(&mut agent);
        assert_eq!(b.call.method(), "fabric_calendar_propose_delete");
        assert_eq!(params(&b)["event"]["event_id"], FOREIGN_EVENT_ID);
        assert_eq!(params(&b)["event"]["revision"], "1");
        answer(&mut agent, &b, "request-b", blocked());

        // C: A's event half an hour later.
        let c = proposed(&mut agent);
        assert_eq!(c.call.method(), "fabric_calendar_propose_update");
        assert_eq!(params(&c)["event"]["event_id"], RESERVED);
        assert_eq!(params(&c)["event"]["revision"], "1");
        assert_eq!(params(&c)["changes"]["title"], Value::Null);
        assert_eq!(
            params(&c)["changes"]["start"]["date_time"],
            "2026-11-27T16:30:00+00:00"
        );
        assert_eq!(
            params(&c)["changes"]["end"]["date_time"],
            "2026-11-27T17:30:00+00:00"
        );
        answer(&mut agent, &c, "request-c", pending());
        carry_out(&calendar, &c, "request-c");
        poll(&mut agent, "request-c", state("applied"));

        // D: A's event deleted at revision 2.
        let d = proposed(&mut agent);
        assert_eq!(d.call.method(), "fabric_calendar_propose_delete");
        assert_eq!(params(&d)["event"]["event_id"], RESERVED);
        assert_eq!(params(&d)["event"]["revision"], "2");
        answer(&mut agent, &d, "request-d", pending());
        carry_out(&calendar, &d, "request-d");
        poll(&mut agent, "request-d", state("applied"));

        let (call, stderr) = propose(&mut agent);
        assert!(call.is_none());
        assert_eq!(stderr, vec![SCRIPT_FINISHED.to_owned()]);
        assert_eq!(agent.proposals, PRACTICE_PROPOSALS);
        assert_eq!(agent.next_due_ms(), None);
        assert!(calendar.event(RESERVED).expect("a tombstone").deleted);
        assert!(!calendar.event(FOREIGN_EVENT_ID).expect("seeded").deleted);
    }

    #[test]
    fn it_proposes_only_while_the_prover_is_ready() {
        let calendar = PracticeCalendar::new(NOW_MS, Faults::default());
        let mut agent = started(SyntheticAgent::practice(calendar));
        // A new practice agent takes the prover as starting.
        assert_eq!(agent.next_due_ms(), None);
        agent.set_prover_health(ProverHealth::Unavailable);
        assert_eq!(agent.next_due_ms(), None);
        assert!(!agent.may_propose());
        agent.set_prover_health(ProverHealth::Ready);
        assert_eq!(agent.next_due_ms(), Some(NOW_MS));

        // An open request is still polled while the prover is away.
        let a = proposed(&mut agent);
        answer(&mut agent, &a, "request-a", pending());
        agent.set_prover_health(ProverHealth::Starting);
        assert_eq!(agent.next_due_ms(), Some(NOW_MS + PACE_MS));
        poll(&mut agent, "request-a", state("undecided"));
        assert_eq!(agent.next_due_ms(), None);
        agent.set_prover_health(ProverHealth::Ready);
        assert_eq!(agent.next_due_ms(), Some(NOW_MS + PACE_MS));
    }

    #[test]
    fn applied_counts_as_done() {
        for done in ["applied", "outcome_unknown"] {
            let calendar = PracticeCalendar::new(NOW_MS, Faults::default());
            let mut agent = practice_agent(&calendar);
            let a = proposed(&mut agent);
            answer(&mut agent, &a, "request-a", state("queued"));
            poll(&mut agent, "request-a", state("dispatching"));
            assert!(matches!(agent.waiting, Waiting::Request(_)));
            poll(&mut agent, "request-a", state(done));
            assert!(matches!(agent.waiting, Waiting::Nothing), "{done}");
            assert_eq!(agent.next_due_ms(), Some(NOW_MS + PACE_MS));
        }
    }

    #[test]
    fn an_unexpected_end_of_a_skips_c_and_d() {
        let calendar = PracticeCalendar::new(NOW_MS, Faults::default());
        let mut agent = practice_agent(&calendar);
        let a = proposed(&mut agent);
        answer(&mut agent, &a, "request-a", pending());
        poll(&mut agent, "request-a", state("undecided"));
        let b = proposed(&mut agent);
        assert_eq!(params(&b)["event"]["event_id"], FOREIGN_EVENT_ID);
        answer(&mut agent, &b, "request-b", blocked());
        let (call, stderr) = propose(&mut agent);
        assert!(call.is_none());
        assert_eq!(
            stderr,
            vec![
                "synthetic agent: step A ended undecided; steps C and D are skipped".to_owned(),
                SCRIPT_FINISHED.to_owned(),
            ]
        );
        assert_eq!(agent.proposals, 2);
        assert_eq!(agent.next_due_ms(), None);

        // A applied, but its practice event is not there: C and D are skipped too.
        let calendar = PracticeCalendar::new(NOW_MS, Faults::default());
        let mut agent = practice_agent(&calendar);
        let a = proposed(&mut agent);
        answer(&mut agent, &a, "request-a", state("applied"));
        let b = proposed(&mut agent);
        answer(&mut agent, &b, "request-b", blocked());
        let (call, stderr) = propose(&mut agent);
        assert!(call.is_none());
        assert_eq!(
            stderr[0],
            "synthetic agent: step A ended applied; steps C and D are skipped"
        );
        assert_eq!(calendar.dispatches(), 0);
    }

    #[test]
    fn without_practice_the_w04_script_is_unchanged() {
        let mut agent = started(SyntheticAgent::new());
        assert_eq!(agent.limit, PROPOSALS_PER_RUN);
        // The prover's health does not hold the W04 agent back.
        agent.set_prover_health(ProverHealth::Unavailable);
        assert_eq!(agent.next_due_ms(), Some(NOW_MS));

        let first = proposed(&mut agent);
        assert_eq!(params(&first), params(&proposal(&first.id, NOW_MS)));
        answer(&mut agent, &first, "request-1", pending());
        // `applied` is open for the W04 agent: it keeps polling.
        poll(&mut agent, "request-1", state("applied"));
        assert!(matches!(agent.waiting, Waiting::Request(_)));
        poll(&mut agent, "request-1", state("undecided"));
        assert_eq!(agent.next_due_ms(), Some(NOW_MS + PACE_MS));

        let second = proposed(&mut agent);
        assert_eq!(second.call.method(), "fabric_calendar_propose_create");
        answer(&mut agent, &second, "request-2", state("undecided"));
        assert_eq!(agent.proposals, PROPOSALS_PER_RUN);
        assert_eq!(agent.next_due_ms(), None);
        assert!(!agent.more_to_propose());
    }

    #[test]
    fn practice_updates_and_deletes_notify_nobody() {
        let event = PracticeEvent {
            event_id: RESERVED.to_owned(),
            revision: "1".to_owned(),
            fabric_created: true,
            request_id: Some(id("request-a")),
            deleted: false,
        };
        let update = update_proposal(&id("call-1"), &event, NOW_MS);
        let delete = delete_proposal(&id("call-2"), RESERVED, "2");
        let foreign = delete_proposal(&id("call-3"), FOREIGN_EVENT_ID, "1");
        for call in [update, delete, foreign] {
            assert_eq!(params(&call)["send_updates"], "none");
            assert_eq!(params(&call)["account_id"], ACCOUNT_ID);
            assert_eq!(params(&call)["calendar_id"], CALENDAR_ID);
        }
    }
}
