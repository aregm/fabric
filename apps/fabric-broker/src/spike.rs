//! The state machine of the CD-W02 spike broker: the owner channel of `fabric-desktop/1`, with
//! synthetic state and a fixed script of synthetic requests.
//!
//! It is pure: it reads no clock and does no input or output. The caller passes Unix time in
//! milliseconds, which must never go back, and writes the frames it gets back in order.

use std::collections::HashMap;

use fabric_desktop_contracts::agent::{
    Action, NewEvent, ProposeCreateArguments, ProposeInvitationArguments,
};
use fabric_desktop_contracts::common::{
    Addresses, CalendarAccess, CalendarRef, EventRef, EventTime, SendUpdates,
};
use fabric_desktop_contracts::error::{
    ErrorCode, InvalidParams, OwnerError, OwnerFault, UnsupportedProtocol, UnsupportedSchemaVersion,
};
use fabric_desktop_contracts::owner::{
    Account, AccountState, Agent, Approval, ApprovalCard, Approved, BuiltInProfile, Calendar,
    Change, DecideRequestParams, DecideRequestResult, Denied, Destination, Duration, DurationKind,
    GetRequestDetailsParams, Grant, GrantOrigin, GrantTerm, Health, HelloParams, HelloResult,
    LifecycleStatus, OwnerCall, OwnerMethod, OwnerNotice, OwnerRequest, OwnerResponse, OwnerResult,
    Party, PolicyVersion, Preferences, PreferencesSettings, PrivateDetails, Profile, Profiles,
    ProverHealth, RecipientRule, Redaction, Request, RequestDecision, RequestDetails, Ruling,
    Scope, Session, Snapshot, StoreHealth, SubscribeParams, SubscribeResult,
};
use fabric_desktop_contracts::scalar::{
    Count, Digest, Email, IanaZone, Id, Label, List, MethodText, NameToken, OffsetDateTime,
    PositiveCount, ProviderRef, Task, Text, Timestamp,
};
use fabric_desktop_contracts::{
    Decision, Entry, Frame, Layer, Limits, Refusal, RequestState, SCHEMA_VERSION, action_digest,
    approval_digest, decode, parse_strict, request_digest,
};
use serde_json::Value;

use crate::time::format_timestamp;

/// The broker's name in the `hello` result.
pub const BROKER_NAME: &str = "fabric-broker-spike";

/// How many command answers the spike keeps for deduplication. A full table refuses new
/// commands with `internal` rather than forget an answer.
pub const COMMAND_TABLE_CAPACITY: usize = 1024;

/// The script, in milliseconds after the first successful `subscribe`.
pub const AGENT_AT_MS: u64 = 200;
pub const ACCOUNT_AT_MS: u64 = 400;
pub const FIRST_REQUEST_AT_MS: u64 = 1000;

/// The steps of a request, in milliseconds after it was received.
pub const VALIDATING_AFTER_MS: u64 = 200;
pub const PROVING_CLASSIFICATION_AFTER_MS: u64 = 500;
pub const PENDING_APPROVAL_AFTER_MS: u64 = 2000;

/// The steps after an approval, in milliseconds after the decision.
pub const PROVING_FRESH_PERMIT_AFTER_MS: u64 = 300;
pub const UNDECIDED_AFTER_MS: u64 = 1500;

/// The second request is received this long after the first one reaches a terminal state.
pub const NEXT_REQUEST_AFTER_MS: u64 = 1000;

/// An approval card expires 24 hours after the request was received (README section 15).
pub const APPROVAL_EXPIRY_MS: u64 = 86_400_000;

/// The synthetic ids.
pub const AGENT_ID: &str = "spike-agent";
pub const SESSION_ID: &str = "spike-session";
pub const ACCOUNT_ID: &str = "spike-account";
pub const CALENDAR_ID: &str = "spike-calendar";
/// The script makes this many requests, `spike-request-1` and `spike-request-2`.
pub const REQUEST_COUNT: usize = 2;

const RULE: &str = "spike_no_rule_theory";
const RULE_EXPLANATION: &str = "Synthetic: the spike broker has no rule theory. A real broker states here why the current grant is not enough.";
const PROOF_ID: &str = "spike-no-proof";

/// The text of the decision-free messages.
const MALFORMED_MESSAGE: &str =
    "The frame is not one strict JSON object of at most 1,048,576 bytes.";
const OVERSIZED_MESSAGE: &str = "The frame is longer than 1,048,576 bytes.";

fn id(text: &str) -> Id {
    Id::new(text).expect("a valid synthetic id")
}

fn label(text: &str) -> Label {
    Label::new(text).expect("a valid synthetic label")
}

fn task(text: &str) -> Task {
    Task::new(text).expect("a valid synthetic task")
}

fn email(text: &str) -> Email {
    Email::new(text).expect("a valid synthetic address")
}

fn event_time(date_time: &str) -> EventTime {
    EventTime {
        date_time: OffsetDateTime::new(date_time).expect("a valid synthetic time"),
        zone: IanaZone::new("Europe/Berlin").expect("a valid zone"),
    }
}

fn method_text(method: OwnerMethod) -> MethodText {
    MethodText::new(method.as_str()).expect("a method name is 1 to 100 code points")
}

fn plain(state: RequestState) -> LifecycleStatus {
    LifecycleStatus::Plain(
        state
            .plain()
            .expect("the state carries nothing but the state"),
    )
}

fn error_frame(
    id: Option<Id>,
    method: Option<MethodText>,
    fault: OwnerFault,
    message: &str,
) -> Frame {
    Frame::OwnerResponse(OwnerResponse::Error {
        id,
        method,
        error: OwnerError {
            fault,
            message: Text::new(message).expect("a message of 1 to 1000 code points"),
            correlation_id: None,
        },
    })
}

fn result_frame(id: Id, result: OwnerResult) -> Frame {
    Frame::OwnerResponse(OwnerResponse::Result { id, result })
}

/// One timed step of the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Agent,
    Account,
    /// Request `index` (from 0) is received.
    Receive(usize),
    /// Request `index` moves to a state that the script reaches by itself.
    Move(usize, RequestState),
}

#[derive(Clone, Copy, Debug)]
struct Scheduled {
    at: u64,
    step: Step,
}

/// One synthetic request and everything the owner may open about it.
#[derive(Clone, Debug)]
struct SpikeRequest {
    request: Request,
    received_ms: u64,
    action: Action,
    task: Task,
    scope: Scope,
    card: Option<ApprovalCard>,
    approval: Option<Approval>,
    /// Every state the request went through, in order.
    states: Vec<RequestState>,
}

impl SpikeRequest {
    fn state(&self) -> RequestState {
        self.request.status.state()
    }
}

/// The first answer to a command, kept to answer the same command again.
#[derive(Clone, Debug)]
struct StoredCommand {
    digest: Digest,
    answer: Frame,
}

/// The spike broker's whole state.
#[derive(Debug, Default)]
pub struct Spike {
    /// Every notice made; notice `n` has sequence `n + 1`.
    notices: Vec<OwnerNotice>,
    subscribed: bool,
    script_started: bool,
    scheduled: Vec<Scheduled>,
    agent: Option<Agent>,
    account: Option<Account>,
    requests: Vec<SpikeRequest>,
    grants: Vec<Grant>,
    commands: HashMap<String, StoredCommand>,
}

impl Spike {
    /// A broker in its first-run state, before any `subscribe`.
    #[must_use]
    pub fn new() -> Self {
        Spike::default()
    }

    /// Answers one line read from stdin, without its line feed. Script steps that are due at
    /// `now_ms` run first, so their notices come before the answer.
    pub fn handle_line(&mut self, line: &[u8], now_ms: u64) -> Vec<Frame> {
        let mut out = self.tick(now_ms);
        match decode(Entry::OwnerRequest, line) {
            Ok(Frame::OwnerRequest(request)) => self.answer(request, now_ms, &mut out),
            Ok(_) => out.push(error_frame(
                None,
                None,
                OwnerFault::Internal,
                "The spike broker decoded an owner request as another kind of frame.",
            )),
            Err(refusal) => out.push(refusal_answer(line, &refusal)),
        }
        out
    }

    /// Answers a line that was longer than `MAX_FRAME_BYTES`.
    pub fn handle_oversized(&mut self) -> Vec<Frame> {
        vec![error_frame(
            None,
            None,
            OwnerFault::MalformedFrame,
            OVERSIZED_MESSAGE,
        )]
    }

    /// Runs every script step due at `now_ms`, in time order.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(index) = self.next_due_index(now_ms) {
            let scheduled = self.scheduled.remove(index);
            self.run(scheduled, &mut out);
        }
        out
    }

    /// When the next script step is due, if one is scheduled.
    #[must_use]
    pub fn next_due_ms(&self) -> Option<u64> {
        self.scheduled.iter().map(|s| s.at).min()
    }

    fn next_due_index(&self, now_ms: u64) -> Option<usize> {
        self.scheduled
            .iter()
            .enumerate()
            .filter(|(_, s)| s.at <= now_ms)
            .min_by_key(|&(index, s)| (s.at, index))
            .map(|(index, _)| index)
    }

    fn schedule(&mut self, at: u64, step: Step) {
        self.scheduled.push(Scheduled { at, step });
    }

    fn last_sequence(&self) -> u64 {
        self.notices.len() as u64
    }

    /// Makes a notice with the next sequence; a subscriber gets it at once.
    fn notify(&mut self, change: Change, out: &mut Vec<Frame>) -> PositiveCount {
        let sequence = PositiveCount::of(self.last_sequence() + 1);
        let notice = OwnerNotice { sequence, change };
        if self.subscribed {
            out.push(Frame::OwnerNotice(notice.clone()));
        }
        self.notices.push(notice);
        sequence
    }

    fn run(&mut self, scheduled: Scheduled, out: &mut Vec<Frame>) {
        let at = scheduled.at;
        match scheduled.step {
            Step::Agent => {
                let agent = synthetic_agent(at);
                self.agent = Some(agent.clone());
                self.notify(Change::Agent(agent), out);
            }
            Step::Account => {
                let account = synthetic_account();
                self.account = Some(account.clone());
                self.notify(Change::Account(account), out);
            }
            Step::Receive(index) => self.receive(index, at, out),
            Step::Move(index, state) => {
                let status = if state == RequestState::PendingApproval {
                    self.open_card(index)
                } else {
                    plain(state)
                };
                self.move_request(index, status, at, out);
            }
        }
    }

    fn receive(&mut self, index: usize, at: u64, out: &mut Vec<Frame>) {
        debug_assert_eq!(index, self.requests.len());
        let (action, task, scope) = synthetic_action(index);
        let agent_id = id(AGENT_ID);
        let stamp = format_timestamp(at);
        let request = Request {
            request_id: id(&format!("spike-request-{}", index + 1)),
            revision: PositiveCount::of(1),
            request_digest: request_digest(&agent_id, &action, &scope),
            agent_id,
            tool: action.tool(),
            account_id: Some(id(ACCOUNT_ID)),
            received_at: stamp.clone(),
            updated_at: stamp,
            status: plain(RequestState::Received),
        };
        self.requests.push(SpikeRequest {
            request: request.clone(),
            received_ms: at,
            action,
            task,
            scope,
            card: None,
            approval: None,
            states: vec![RequestState::Received],
        });
        self.notify(Change::Request(request), out);
        self.schedule(
            at + VALIDATING_AFTER_MS,
            Step::Move(index, RequestState::Validating),
        );
        self.schedule(
            at + PROVING_CLASSIFICATION_AFTER_MS,
            Step::Move(index, RequestState::ProvingClassification),
        );
        self.schedule(
            at + PENDING_APPROVAL_AFTER_MS,
            Step::Move(index, RequestState::PendingApproval),
        );
    }

    /// Puts the approval card on request `index` and gives the status that carries its expiry.
    fn open_card(&mut self, index: usize) -> LifecycleStatus {
        let entry = &mut self.requests[index];
        let expires_at = format_timestamp(entry.received_ms + APPROVAL_EXPIRY_MS);
        entry.card = Some(ApprovalCard {
            scope: entry.scope.clone(),
            proposed_duration: Duration {
                kind: DurationKind::OneRequest,
            },
            insufficiency: Ruling {
                rule: NameToken::new(RULE).expect("a valid rule name"),
                explanation: Text::new(RULE_EXPLANATION).expect("a valid explanation"),
                proof_id: id(PROOF_ID),
            },
            expires_at: expires_at.clone(),
        });
        LifecycleStatus::PendingApproval {
            approval_expires_at: expires_at,
        }
    }

    /// Moves request `index` to `status` at `at`: a new revision and a notice. The second
    /// request is scheduled when the first reaches a terminal state.
    fn move_request(
        &mut self,
        index: usize,
        status: LifecycleStatus,
        at: u64,
        out: &mut Vec<Frame>,
    ) -> PositiveCount {
        let state = status.state();
        let entry = &mut self.requests[index];
        debug_assert!(entry.state().can_transition_to(state));
        entry.request.revision = PositiveCount::of(entry.request.revision.get() + 1);
        entry.request.updated_at = format_timestamp(at);
        entry.request.status = status;
        entry.states.push(state);
        let request = entry.request.clone();
        let sequence = self.notify(Change::Request(request), out);
        if state.is_terminal() && index + 1 < REQUEST_COUNT && self.requests.len() == index + 1 {
            self.schedule(at + NEXT_REQUEST_AFTER_MS, Step::Receive(index + 1));
        }
        sequence
    }

    fn find(&self, request_id: &Id) -> Option<usize> {
        self.requests
            .iter()
            .position(|r| &r.request.request_id == request_id)
    }

    fn answer(&mut self, request: OwnerRequest, now_ms: u64, out: &mut Vec<Frame>) {
        let frame_id = request.id.clone();
        match &request.call {
            OwnerCall::Hello(params) => out.push(hello(frame_id, params)),
            OwnerCall::GetSnapshot(_) => {
                out.push(result_frame(
                    frame_id,
                    OwnerResult::GetSnapshot(self.snapshot()),
                ));
            }
            OwnerCall::Subscribe(params) => self.subscribe(frame_id, params, now_ms, out),
            OwnerCall::GetRequestDetails(params) => out.push(self.details(frame_id, params)),
            OwnerCall::DecideRequest(params) => {
                self.decide_command(&request, params, now_ms, out);
            }
            OwnerCall::ConnectGoogle(_)
            | OwnerCall::SaveProfile(_)
            | OwnerCall::SavePreferences(_)
            | OwnerCall::SaveGroup(_)
            | OwnerCall::PauseAgent(_)
            | OwnerCall::RevokeGrant(_)
            | OwnerCall::GetProof(_)
            | OwnerCall::DisconnectGoogle(_) => {
                let method = request.call.method();
                out.push(error_frame(
                    Some(frame_id),
                    Some(method_text(method)),
                    OwnerFault::UnknownMethod,
                    &format!(
                        "The spike broker of CD-W02 does not implement {}.",
                        method.as_str()
                    ),
                ));
            }
        }
    }

    /// The state now. Before the script starts it is the first-run snapshot.
    fn snapshot(&self) -> Snapshot {
        let open = self
            .requests
            .iter()
            .filter(|r| !r.state().is_terminal())
            .map(|r| r.request.clone())
            .collect();
        let grants = self
            .grants
            .iter()
            .filter(|g| g.ended.is_none())
            .cloned()
            .collect();
        Snapshot {
            cursor: Count::of(self.last_sequence()),
            // No prover runs in the spike; it has no store, and `locked` is the store health
            // that promises nothing durable.
            health: Health {
                prover: ProverHealth::Unavailable,
                store: StoreHealth::Locked,
            },
            profiles: Profiles {
                revision: Count::of(0),
                active_profile_id: id(BuiltInProfile::Careful.profile_id()),
                items: List::new(vec![
                    Profile::BuiltIn(BuiltInProfile::Careful),
                    Profile::BuiltIn(BuiltInProfile::Flexible),
                ])
                .expect("two profiles"),
            },
            accounts: List::new(self.account.iter().cloned().collect()).expect("any list"),
            agents: List::new(self.agent.iter().cloned().collect()).expect("any list"),
            recipient_sets: List::new(Vec::new()).expect("any list"),
            preferences: Preferences {
                revision: Count::of(0),
                settings: PreferencesSettings {
                    zone: None,
                    workday: None,
                    protected_times: List::new(Vec::new()).expect("an empty list"),
                    uninterrupted: None,
                    buffer: None,
                    warning_calendars: List::new(Vec::new()).expect("an empty list"),
                },
            },
            requests: List::new(open).expect("any list"),
            grants: List::new(grants).expect("any list"),
        }
    }

    fn subscribe(
        &mut self,
        frame_id: Id,
        params: &SubscribeParams,
        now_ms: u64,
        out: &mut Vec<Frame>,
    ) {
        let cursor = params.cursor.get();
        if cursor > self.last_sequence() {
            out.push(error_frame(
                Some(frame_id),
                Some(method_text(OwnerMethod::Subscribe)),
                OwnerFault::SnapshotRequired,
                "The cursor is ahead of the last notice, so take a snapshot first.",
            ));
            return;
        }
        out.push(result_frame(
            frame_id,
            OwnerResult::Subscribe(SubscribeResult {
                cursor: params.cursor,
            }),
        ));
        // One subscriber: this subscription replaces any earlier one and starts at `cursor`.
        self.subscribed = true;
        let start = usize::try_from(cursor).expect("a cursor within the history");
        out.extend(
            self.notices[start..]
                .iter()
                .cloned()
                .map(Frame::OwnerNotice),
        );
        if !self.script_started {
            self.script_started = true;
            self.schedule(now_ms + AGENT_AT_MS, Step::Agent);
            self.schedule(now_ms + ACCOUNT_AT_MS, Step::Account);
            self.schedule(now_ms + FIRST_REQUEST_AT_MS, Step::Receive(0));
        }
    }

    fn details(&self, frame_id: Id, params: &GetRequestDetailsParams) -> Frame {
        let Some(index) = self.find(&params.request_id) else {
            return error_frame(
                Some(frame_id),
                Some(method_text(OwnerMethod::GetRequestDetails)),
                OwnerFault::NotFound,
                "The spike broker has no request with this id.",
            );
        };
        let entry = &self.requests[index];
        let private = (params.redaction == Redaction::Full).then(|| PrivateDetails {
            action: entry.action.clone(),
            before: None,
            warnings: List::new(Vec::new()).expect("an empty list"),
        });
        result_frame(
            frame_id,
            OwnerResult::GetRequestDetails(RequestDetails {
                request: entry.request.clone(),
                redaction: params.redaction,
                private,
                card: entry.card.clone(),
                approval: entry.approval.clone(),
                proof_ids: List::new(Vec::new()).expect("an empty list"),
                authorization: None,
                attempts: List::new(Vec::new()).expect("an empty list"),
                provider_event: None,
            }),
        )
    }

    /// `decide_request` as a command: deduplicated by the frame id and the command digest.
    fn decide_command(
        &mut self,
        request: &OwnerRequest,
        params: &DecideRequestParams,
        now_ms: u64,
        out: &mut Vec<Frame>,
    ) {
        let digest = request
            .command_digest()
            .expect("decide_request is a command");
        if let Some(stored) = self.commands.get(request.id.as_str()) {
            if stored.digest == digest {
                out.push(stored.answer.clone());
            } else {
                out.push(error_frame(
                    Some(request.id.clone()),
                    Some(method_text(OwnerMethod::DecideRequest)),
                    OwnerFault::CommandReused,
                    "This command id was already used for a different command.",
                ));
            }
            return;
        }
        if self.commands.len() >= COMMAND_TABLE_CAPACITY {
            out.push(error_frame(
                Some(request.id.clone()),
                Some(method_text(OwnerMethod::DecideRequest)),
                OwnerFault::Internal,
                "The spike broker's command table is full, so it takes no new command.",
            ));
            return;
        }
        let answer = self.decide(&request.id, params, now_ms, out);
        self.commands.insert(
            request.id.as_str().to_owned(),
            StoredCommand {
                digest,
                answer: answer.clone(),
            },
        );
        out.push(answer);
    }

    /// Makes the decision and returns the answer to store. Notices go to `out` first.
    fn decide(
        &mut self,
        command_id: &Id,
        params: &DecideRequestParams,
        now_ms: u64,
        out: &mut Vec<Frame>,
    ) -> Frame {
        let refuse = |fault, message: &str| {
            error_frame(
                Some(command_id.clone()),
                Some(method_text(OwnerMethod::DecideRequest)),
                fault,
                message,
            )
        };
        let Some(index) = self.find(&params.request_id) else {
            return refuse(
                OwnerFault::NotFound,
                "The spike broker has no request with this id.",
            );
        };
        let entry = &self.requests[index];
        if entry.state() != RequestState::PendingApproval
            || params.expected_revision.get() != entry.request.revision.get()
            || params.request_digest != entry.request.request_digest
        {
            return refuse(
                OwnerFault::StaleRequest,
                "The request is not the one that was shown, so review it again before deciding.",
            );
        }
        let request_digest = entry.request.request_digest.clone();
        let decided_at = format_timestamp(now_ms);
        let (approval, sequence) = match &params.decision {
            RequestDecision::Approve {
                selected: Some(_), ..
            } => {
                return refuse(
                    OwnerFault::InvalidParams(InvalidParams {
                        pointer: Text::new("/params/selected").expect("a short pointer"),
                    }),
                    "The spike broker does not narrow a scope, so selected must be null.",
                );
            }
            RequestDecision::Approve {
                selected: None,
                duration,
            } => {
                let scope = entry.scope.clone();
                let approval_digest = approval_digest(
                    &params.request_id,
                    &request_digest,
                    &Decision::Approve {
                        scope: scope.clone(),
                        duration: *duration,
                    },
                );
                let grant = self.grant(index, *duration, &approval_digest, &decided_at);
                let grant_id = grant.grant_id.clone();
                self.grants.push(grant.clone());
                self.notify(Change::Grant(grant), out);
                let sequence = self.move_request(index, plain(RequestState::Approved), now_ms, out);
                self.schedule(
                    now_ms + PROVING_FRESH_PERMIT_AFTER_MS,
                    Step::Move(index, RequestState::ProvingFreshPermit),
                );
                // No prover runs, so a fresh permit is never proved and nothing is dispatched.
                self.schedule(
                    now_ms + UNDECIDED_AFTER_MS,
                    Step::Move(index, RequestState::Undecided),
                );
                let approval = Approval::Approve(Approved {
                    command_id: command_id.clone(),
                    decided_at,
                    approval_digest,
                    scope,
                    duration: *duration,
                    // The scope is approved as asked, so the effective digest is the request's.
                    effective_request_digest: request_digest,
                    grant_id,
                });
                (approval, sequence)
            }
            RequestDecision::Deny => {
                let approval_digest =
                    approval_digest(&params.request_id, &request_digest, &Decision::Deny);
                let sequence = self.move_request(index, plain(RequestState::Denied), now_ms, out);
                let approval = Approval::Deny(Denied {
                    command_id: command_id.clone(),
                    decided_at,
                    approval_digest,
                });
                (approval, sequence)
            }
        };
        let entry = &mut self.requests[index];
        entry.approval = Some(approval.clone());
        result_frame(
            command_id.clone(),
            OwnerResult::DecideRequest(DecideRequestResult {
                commit_sequence: sequence,
                request: entry.request.clone(),
                approval,
            }),
        )
    }

    /// The grant an approval of request `index` for `duration` creates.
    fn grant(
        &self,
        index: usize,
        duration: Duration,
        approval_digest: &Digest,
        created_at: &Timestamp,
    ) -> Grant {
        let entry = &self.requests[index];
        let term = match duration.kind {
            DurationKind::OneRequest => GrantTerm::OneRequest {
                action_digest: action_digest(&entry.action),
            },
            DurationKind::Session => GrantTerm::Session {
                session_id: id(SESSION_ID),
            },
            DurationKind::UntilRevoked => GrantTerm::UntilRevoked,
        };
        Grant {
            grant_id: id(&format!("spike-grant-{}", self.grants.len() + 1)),
            revision: PositiveCount::of(1),
            agent_id: entry.request.agent_id.clone(),
            purpose: entry.task.clone(),
            tools: List::new(vec![entry.action.tool()]).expect("one tool"),
            scope: entry.scope.clone(),
            term,
            origin: GrantOrigin {
                request_id: entry.request.request_id.clone(),
                approval_digest: approval_digest.clone(),
            },
            policy: PolicyVersion {
                profile_id: id(BuiltInProfile::Careful.profile_id()),
                version: PositiveCount::of(BuiltInProfile::Careful.version()),
            },
            revocation_epoch: Count::of(0),
            created_at: created_at.clone(),
            ended: None,
        }
    }
}

fn hello(frame_id: Id, params: &HelloParams) -> Frame {
    if params.schema_version.get() != SCHEMA_VERSION {
        return error_frame(
            Some(frame_id),
            Some(method_text(OwnerMethod::Hello)),
            OwnerFault::UnsupportedSchemaVersion(UnsupportedSchemaVersion {
                received: params.schema_version,
                supported: PositiveCount::of(SCHEMA_VERSION),
            }),
            "The spike broker speaks schema version 1 only.",
        );
    }
    result_frame(
        frame_id,
        OwnerResult::Hello(HelloResult {
            broker: Party {
                name: label(BROKER_NAME),
                build: label(&format!("{} CD-W02 spike", env!("CARGO_PKG_VERSION"))),
            },
            schema_version: PositiveCount::of(SCHEMA_VERSION),
            capabilities: List::new(Vec::new()).expect("no capabilities"),
            limits: Limits::CD_G0,
        }),
    )
}

/// The answer to a frame that `decode` refused, by the layer it was refused at (README
/// section 5).
fn refusal_answer(line: &[u8], refusal: &Refusal) -> Frame {
    if refusal.layer == Layer::Frame {
        return error_frame(None, None, OwnerFault::MalformedFrame, MALFORMED_MESSAGE);
    }
    // The frame layer passed, so the line is one strict JSON object.
    let value = parse_strict(line).ok();
    let member = |name: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let frame_id = member("id").and_then(|s| Id::new(s).ok());
    let method = member("method").and_then(|s| MethodText::new(s).ok());
    let (fault, message) = match refusal.layer {
        Layer::Protocol => (
            OwnerFault::UnsupportedProtocol(UnsupportedProtocol {
                received: member("protocol").and_then(|s| Text::new(s).ok()),
            }),
            "The frame does not name the protocol fabric-desktop/1.",
        ),
        _ if refusal.code == ErrorCode::UnknownMethod => (
            OwnerFault::UnknownMethod,
            "The method is not an owner method of fabric-desktop/1.",
        ),
        // The contract crate does not say which value failed, so the pointer names the
        // whole of `params`.
        _ => (
            OwnerFault::InvalidParams(InvalidParams {
                pointer: Text::new("/params").expect("a short pointer"),
            }),
            "The frame does not match the schema of its method.",
        ),
    };
    error_frame(frame_id, method, fault, message)
}

fn synthetic_agent(at: u64) -> Agent {
    let started = format_timestamp(at);
    Agent {
        agent_id: id(AGENT_ID),
        revision: PositiveCount::of(1),
        registered_name: label("Synthetic agent (spike)"),
        destination: Destination::Local {
            endpoint: label("inside the broker spike"),
        },
        paused: false,
        session: Some(Session {
            session_id: id(SESSION_ID),
            started_at: started.clone(),
            last_active_at: started,
        }),
        last_tool_call_at: None,
    }
}

fn synthetic_account() -> Account {
    Account {
        account_id: id(ACCOUNT_ID),
        revision: PositiveCount::of(1),
        state: AccountState::Connected,
        email: Some(email("synthetic@example.invalid")),
        calendars: List::new(vec![Calendar {
            calendar_id: id(CALENDAR_ID),
            name: label("Synthetic calendar (spike)"),
            access: CalendarAccess::Owner,
            selected: true,
            agent_drafts: false,
        }])
        .expect("any list"),
    }
}

/// The action of request `index`, the agent's words in it, and the scope it needs.
fn synthetic_action(index: usize) -> (Action, Task, Scope) {
    let calendars = List::new(vec![CalendarRef {
        account_id: id(ACCOUNT_ID),
        calendar_id: id(CALENDAR_ID),
    }])
    .expect("one calendar");
    if index == 0 {
        let words = task(
            "Please hold an hour to plan next week. (Synthetic: the CD-W02 spike broker wrote this, not an agent.)",
        );
        let action = Action::ProposeCreate(ProposeCreateArguments {
            account_id: id(ACCOUNT_ID),
            calendar_id: id(CALENDAR_ID),
            event: NewEvent {
                title: Text::new("Synthetic planning block (spike)").expect("a valid title"),
                start: event_time("2026-11-23T10:00:00+01:00"),
                end: event_time("2026-11-23T11:00:00+01:00"),
                description: None,
                location: None,
            },
            task: words.clone(),
        });
        let scope = Scope {
            calendars,
            window: None,
            fields: None,
            recipients: None,
            send_updates: None,
        };
        (action, words, scope)
    } else {
        let words = task(
            "Please invite two people to the planning block. (Synthetic: the CD-W02 spike broker wrote this, not an agent.)",
        );
        let addresses = || vec![email("ally@example.invalid"), email("sam@example.invalid")];
        let action = Action::ProposeInvitation(ProposeInvitationArguments {
            account_id: id(ACCOUNT_ID),
            calendar_id: id(CALENDAR_ID),
            event: EventRef {
                event_id: ProviderRef::new("spike-event-1").expect("a valid event id"),
                revision: ProviderRef::new("1").expect("a valid revision"),
            },
            attendees: List::new(addresses()).expect("two distinct addresses"),
            send_updates: SendUpdates::All,
            task: words.clone(),
        });
        let scope = Scope {
            calendars,
            window: None,
            fields: None,
            recipients: Some(RecipientRule::Exact(
                Addresses::new(addresses()).expect("two distinct addresses"),
            )),
            send_updates: Some(SendUpdates::All),
        };
        (action, words, scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::{MAX_FRAME_BYTES, canonical, first_invalid};
    use serde_json::json;

    const T0: u64 = 1_795_190_400_000; // 2026-11-20T16:00:00.000Z

    /// Checks that every frame decodes again as what it is, and returns them.
    fn checked(frames: Vec<Frame>) -> Vec<Frame> {
        for frame in &frames {
            let line = frame.to_line();
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
            let bytes = &line.as_bytes()[..line.len() - 1];
            let entry = match frame {
                Frame::OwnerResponse(_) => Entry::OwnerResponse,
                Frame::OwnerNotice(_) => Entry::OwnerNotice,
                other => panic!("the broker wrote {other:?}"),
            };
            let back = decode(entry, bytes).unwrap_or_else(|r| panic!("{r}: {line}"));
            assert_eq!(&back, frame);
        }
        frames
    }

    fn send(spike: &mut Spike, value: &Value, now: u64) -> Vec<Frame> {
        checked(spike.handle_line(canonical(value).as_bytes(), now))
    }

    fn request(frame_id: &str, method: &str, params: Value) -> Value {
        json!({ "protocol": "fabric-desktop/1", "id": frame_id, "method": method, "params": params })
    }

    fn only_response(frames: Vec<Frame>) -> OwnerResponse {
        assert_eq!(frames.len(), 1, "{frames:?}");
        match frames.into_iter().next() {
            Some(Frame::OwnerResponse(r)) => r,
            other => panic!("{other:?}"),
        }
    }

    fn result_of(response: OwnerResponse) -> OwnerResult {
        match response {
            OwnerResponse::Result { result, .. } => result,
            OwnerResponse::Error { error, .. } => panic!("{error:?}"),
        }
    }

    fn error_of(response: &OwnerResponse) -> (Option<&str>, Option<&str>, ErrorCode, Value) {
        use fabric_desktop_contracts::error::Fault;
        match response {
            OwnerResponse::Error { id, method, error } => (
                id.as_ref().map(Id::as_str),
                method.as_ref().map(|m| m.as_str()),
                error.code(),
                error.fault.detail(),
            ),
            other => panic!("{other:?}"),
        }
    }

    fn notices(frames: &[Frame]) -> Vec<OwnerNotice> {
        frames
            .iter()
            .filter_map(|f| match f {
                Frame::OwnerNotice(n) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }

    fn snapshot(spike: &mut Spike, now: u64) -> Snapshot {
        let frames = send(spike, &request("snap", "get_snapshot", json!({})), now);
        match result_of(only_response(frames)) {
            OwnerResult::GetSnapshot(s) => s,
            other => panic!("{other:?}"),
        }
    }

    /// A broker subscribed at `T0`, with request 1 pending approval at `T0 + 3000`.
    fn pending() -> (Spike, Vec<Frame>) {
        let mut spike = Spike::new();
        let mut frames = send(
            &mut spike,
            &request("sub", "subscribe", json!({"cursor": 0})),
            T0,
        );
        frames.extend(checked(spike.tick(T0 + 3000)));
        (spike, frames)
    }

    fn decide_params(spike: &Spike, index: usize, decision: &str, duration: Value) -> Value {
        let r = &spike.requests[index].request;
        json!({
            "request_id": r.request_id,
            "request_digest": r.request_digest,
            "expected_revision": r.revision.get(),
            "decision": decision,
            "selected": null,
            "duration": duration,
        })
    }

    #[test]
    fn hello_reports_the_spike_and_the_limits() {
        let mut spike = Spike::new();
        let hello = json!({"client": {"name": "test", "build": "1"}, "schema_version": 1});
        let frames = send(&mut spike, &request("h", "hello", hello), T0);
        let OwnerResult::Hello(result) = result_of(only_response(frames)) else {
            panic!()
        };
        assert_eq!(result.broker.name.as_str(), "fabric-broker-spike");
        assert!(result.broker.build.as_str().ends_with(" CD-W02 spike"));
        assert_eq!(result.schema_version.get(), 1);
        assert!(result.capabilities.is_empty());
        assert_eq!(result.limits, Limits::CD_G0);
    }

    #[test]
    fn hello_with_another_schema_version_is_refused() {
        let mut spike = Spike::new();
        let hello = json!({"client": {"name": "test", "build": "1"}, "schema_version": 2});
        let response = only_response(send(&mut spike, &request("h", "hello", hello), T0));
        let (frame_id, method, code, detail) = error_of(&response);
        assert_eq!((frame_id, method), (Some("h"), Some("hello")));
        assert_eq!(code, ErrorCode::UnsupportedSchemaVersion);
        assert_eq!(detail, json!({"received": 2, "supported": 1}));
    }

    #[test]
    fn first_run_snapshot() {
        let mut spike = Spike::new();
        let s = snapshot(&mut spike, T0);
        assert_eq!(s.cursor.get(), 0);
        assert_eq!(s.health.prover, ProverHealth::Unavailable);
        assert_eq!(s.health.store, StoreHealth::Locked);
        assert_eq!(s.profiles.revision.get(), 0);
        assert_eq!(s.profiles.active_profile_id.as_str(), "careful");
        assert_eq!(s.profiles.items.len(), 2);
        assert!(s.accounts.is_empty() && s.agents.is_empty() && s.recipient_sets.is_empty());
        assert!(s.requests.is_empty() && s.grants.is_empty());
        assert_eq!(s.preferences.revision.get(), 0);
        assert_eq!(
            canonical(&serde_json::to_value(&s.preferences.settings).unwrap()),
            r#"{"buffer":null,"protected_times":[],"uninterrupted":null,"warning_calendars":[],"workday":null,"zone":null}"#
        );
    }

    #[test]
    fn nothing_is_scheduled_before_subscribe() {
        let mut spike = Spike::new();
        assert_eq!(spike.next_due_ms(), None);
        assert!(spike.tick(T0 + 1_000_000).is_empty());
        assert_eq!(snapshot(&mut spike, T0 + 1_000_000).cursor.get(), 0);
    }

    #[test]
    fn subscribe_starts_the_script() {
        let mut spike = Spike::new();
        let frames = send(
            &mut spike,
            &request("sub", "subscribe", json!({"cursor": 0})),
            T0,
        );
        assert_eq!(
            result_of(only_response(frames)),
            OwnerResult::Subscribe(SubscribeResult {
                cursor: Count::of(0)
            })
        );
        assert_eq!(spike.next_due_ms(), Some(T0 + 200));
        let mut all = Vec::new();
        let mut now = T0;
        while let Some(due) = spike.next_due_ms() {
            if due > T0 + 3000 {
                break;
            }
            assert!(checked(spike.tick(due - 1)).is_empty());
            now = due;
            all.extend(checked(spike.tick(now)));
        }
        assert_eq!(now, T0 + 3000);
        let notices = notices(&all);
        let sequences: Vec<u64> = notices.iter().map(|n| n.sequence.get()).collect();
        assert_eq!(sequences, [1, 2, 3, 4, 5, 6]);
        let kinds: Vec<&str> = notices.iter().map(|n| n.change.kind()).collect();
        assert_eq!(
            kinds,
            [
                "agent", "account", "request", "request", "request", "request"
            ]
        );
        let states: Vec<RequestState> = notices[2..]
            .iter()
            .map(|n| match &n.change {
                Change::Request(r) => r.status.state(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(first_invalid(&states), None);
        assert_eq!(states.last(), Some(&RequestState::PendingApproval));
        let Change::Request(last) = &notices[5].change else {
            panic!()
        };
        assert_eq!(last.revision.get(), 4);
        assert_eq!(last.received_at.as_str(), "2026-11-20T16:00:01.000Z");
        assert_eq!(last.updated_at.as_str(), "2026-11-20T16:00:03.000Z");
        assert_eq!(
            serde_json::to_value(&last.status).unwrap(),
            json!({"state": "pending_approval", "approval_expires_at": "2026-11-21T16:00:01.000Z"})
        );
        let agent = spike.agent.as_ref().unwrap();
        assert_eq!(agent.registered_name.as_str(), "Synthetic agent (spike)");
        assert_eq!(
            serde_json::to_value(&agent.destination).unwrap(),
            json!({"kind": "local", "endpoint": "inside the broker spike"})
        );
        let s = snapshot(&mut spike, T0 + 3000);
        assert_eq!(s.cursor.get(), 6);
        assert_eq!(
            (s.accounts.len(), s.agents.len(), s.requests.len()),
            (1, 1, 1)
        );
    }

    #[test]
    fn subscribe_from_a_cursor_resends_the_later_notices_and_replaces_the_subscription() {
        let (mut spike, _) = pending();
        let frames = send(
            &mut spike,
            &request("sub-2", "subscribe", json!({"cursor": 4})),
            T0 + 3000,
        );
        assert_eq!(frames.len(), 3);
        let sequences: Vec<u64> = notices(&frames).iter().map(|n| n.sequence.get()).collect();
        assert_eq!(sequences, [5, 6]);
        // A second subscribe does not start the script again.
        assert_eq!(spike.next_due_ms(), None);
    }

    #[test]
    fn subscribe_with_a_cursor_ahead_needs_a_snapshot() {
        let mut spike = Spike::new();
        let response = only_response(send(
            &mut spike,
            &request("sub", "subscribe", json!({"cursor": 1})),
            T0,
        ));
        assert_eq!(error_of(&response).2, ErrorCode::SnapshotRequired);
        assert_eq!(spike.next_due_ms(), None);
    }

    #[test]
    fn request_details_full_and_redacted() {
        let (mut spike, _) = pending();
        let ask = |redaction| {
            request(
                "d",
                "get_request_details",
                json!({"request_id": "spike-request-1", "redaction": redaction}),
            )
        };
        let OwnerResult::GetRequestDetails(full) =
            result_of(only_response(send(&mut spike, &ask("full"), T0 + 3000)))
        else {
            panic!()
        };
        let private = full.private.unwrap();
        assert_eq!(
            private.action.tool().as_str(),
            "fabric_calendar_propose_create"
        );
        assert!(private.before.is_none() && private.warnings.is_empty());
        let card = full.card.unwrap();
        assert_eq!(card.proposed_duration.kind, DurationKind::OneRequest);
        assert_eq!(card.insufficiency.rule.as_str(), "spike_no_rule_theory");
        assert_eq!(card.insufficiency.proof_id.as_str(), "spike-no-proof");
        assert_eq!(card.expires_at.as_str(), "2026-11-21T16:00:01.000Z");
        assert_eq!(
            full.request.request_digest,
            request_digest(&id(AGENT_ID), &private.action, &card.scope)
        );
        assert!(full.approval.is_none() && full.authorization.is_none());
        assert!(full.proof_ids.is_empty() && full.attempts.is_empty());
        let OwnerResult::GetRequestDetails(redacted) =
            result_of(only_response(send(&mut spike, &ask("redacted"), T0 + 3000)))
        else {
            panic!()
        };
        assert!(redacted.private.is_none());
        assert_eq!(redacted.redaction, Redaction::Redacted);
    }

    #[test]
    fn request_details_of_an_unknown_request() {
        let mut spike = Spike::new();
        let ask = request(
            "d",
            "get_request_details",
            json!({"request_id": "spike-request-9", "redaction": "full"}),
        );
        let response = only_response(send(&mut spike, &ask, T0));
        assert_eq!(
            error_of(&response),
            (
                Some("d"),
                Some("get_request_details"),
                ErrorCode::NotFound,
                Value::Null
            )
        );
    }

    #[test]
    fn approve_for_each_duration_creates_the_bound_grant() {
        for (kind, term) in [
            ("one_request", "one_request"),
            ("session", "session"),
            ("until_revoked", "until_revoked"),
        ] {
            let (mut spike, _) = pending();
            let params = decide_params(&spike, 0, "approve", json!({"kind": kind}));
            let frames = send(
                &mut spike,
                &request("cmd-1", "decide_request", params),
                T0 + 4000,
            );
            assert_eq!(frames.len(), 3);
            let notices = notices(&frames);
            assert_eq!(notices[0].sequence.get(), 7);
            let Change::Grant(grant) = &notices[0].change else {
                panic!()
            };
            let term_value = serde_json::to_value(&grant.term).unwrap();
            assert_eq!(term_value["kind"], term);
            match kind {
                "one_request" => assert_eq!(
                    term_value["action_digest"],
                    json!(action_digest(&spike.requests[0].action))
                ),
                "session" => assert_eq!(term_value["session_id"], "spike-session"),
                _ => assert_eq!(term_value["expires_at"], Value::Null),
            }
            assert_eq!(grant.grant_id.as_str(), "spike-grant-1");
            assert_eq!(grant.created_at.as_str(), "2026-11-20T16:00:04.000Z");
            let Change::Request(approved) = &notices[1].change else {
                panic!()
            };
            assert_eq!(approved.status.state(), RequestState::Approved);
            let Frame::OwnerResponse(response) = frames[2].clone() else {
                panic!()
            };
            let OwnerResult::DecideRequest(result) = result_of(response) else {
                panic!()
            };
            assert_eq!(result.commit_sequence.get(), 8);
            assert_eq!(&result.request, approved);
            let Approval::Approve(a) = &result.approval else {
                panic!()
            };
            assert_eq!(a.command_id.as_str(), "cmd-1");
            assert_eq!(a.grant_id, grant.grant_id);
            assert_eq!(a.effective_request_digest, approved.request_digest);
            assert_eq!(grant.origin.approval_digest, a.approval_digest);
            assert_eq!(
                a.approval_digest,
                approval_digest(
                    &approved.request_id,
                    &approved.request_digest,
                    &Decision::Approve {
                        scope: a.scope.clone(),
                        duration: a.duration
                    }
                )
            );
        }
    }

    /// Runs the whole script: approve request 1, deny request 2.
    #[test]
    fn the_whole_script() {
        let (mut spike, mut all) = pending();
        let params = decide_params(&spike, 0, "approve", json!({"kind": "one_request"}));
        let decided = T0 + 5000;
        all.extend(send(
            &mut spike,
            &request("cmd-1", "decide_request", params),
            decided,
        ));
        assert_eq!(spike.next_due_ms(), Some(decided + 300));
        all.extend(checked(spike.tick(decided + 1500)));
        assert_eq!(spike.requests[0].state(), RequestState::Undecided);
        // Request 2 comes 1000 ms after request 1 ended.
        assert!(checked(spike.tick(decided + 2499)).is_empty());
        all.extend(checked(spike.tick(decided + 2500)));
        assert_eq!(spike.requests.len(), 2);
        assert_eq!(
            spike.requests[1].request.received_at,
            format_timestamp(decided + 2500)
        );
        all.extend(checked(spike.tick(decided + 2500 + 2000)));
        assert_eq!(spike.requests[1].state(), RequestState::PendingApproval);
        let params = decide_params(&spike, 1, "deny", Value::Null);
        let frames = send(
            &mut spike,
            &request("cmd-2", "decide_request", params),
            decided + 6000,
        );
        let Some(Frame::OwnerResponse(response)) = frames.last().cloned() else {
            panic!()
        };
        let OwnerResult::DecideRequest(result) = result_of(response) else {
            panic!()
        };
        let Approval::Deny(denied) = &result.approval else {
            panic!()
        };
        assert_eq!(
            denied.approval_digest,
            approval_digest(
                &result.request.request_id,
                &result.request.request_digest,
                &Decision::Deny
            )
        );
        all.extend(frames);
        // After request 2 there is nothing more.
        assert_eq!(spike.next_due_ms(), None);
        assert!(checked(spike.tick(decided + 1_000_000)).is_empty());

        let notices = notices(&all);
        let sequences: Vec<u64> = notices.iter().map(|n| n.sequence.get()).collect();
        assert_eq!(sequences, (1..=sequences.len() as u64).collect::<Vec<_>>());
        for (index, expected) in [
            (
                0,
                &[
                    RequestState::Received,
                    RequestState::Validating,
                    RequestState::ProvingClassification,
                    RequestState::PendingApproval,
                    RequestState::Approved,
                    RequestState::ProvingFreshPermit,
                    RequestState::Undecided,
                ][..],
            ),
            (
                1,
                &[
                    RequestState::Received,
                    RequestState::Validating,
                    RequestState::ProvingClassification,
                    RequestState::PendingApproval,
                    RequestState::Denied,
                ][..],
            ),
        ] {
            let request_id = format!("spike-request-{}", index + 1);
            let seen: Vec<(RequestState, u64)> = notices
                .iter()
                .filter_map(|n| match &n.change {
                    Change::Request(r) if r.request_id.as_str() == request_id => {
                        Some((r.status.state(), r.revision.get()))
                    }
                    _ => None,
                })
                .collect();
            let states: Vec<RequestState> = seen.iter().map(|s| s.0).collect();
            assert_eq!(states, expected);
            assert_eq!(first_invalid(&states), None);
            let revisions: Vec<u64> = seen.iter().map(|s| s.1).collect();
            assert_eq!(revisions, (1..=states.len() as u64).collect::<Vec<_>>());
            assert_eq!(spike.requests[index].states, expected);
        }
        let s = snapshot(&mut spike, decided + 1_000_000);
        assert!(s.requests.is_empty());
        assert_eq!(s.grants.len(), 1);
        assert_eq!(s.cursor.get(), notices.len() as u64);
    }

    #[test]
    fn a_duplicate_command_gets_the_same_bytes_and_changes_nothing() {
        let (mut spike, _) = pending();
        let params = decide_params(&spike, 0, "approve", json!({"kind": "session"}));
        let command = request("cmd-1", "decide_request", params);
        let first = send(&mut spike, &command, T0 + 4000);
        let before = snapshot(&mut spike, T0 + 4000);
        let notices_before = spike.notices.len();
        let again = send(&mut spike, &command, T0 + 4000);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].to_line(), first.last().unwrap().to_line());
        assert_eq!(snapshot(&mut spike, T0 + 4000), before);
        assert_eq!(spike.notices.len(), notices_before);
        assert_eq!(spike.grants.len(), 1);
        assert_eq!(spike.commands.len(), 1);
    }

    #[test]
    fn a_duplicate_refusal_is_answered_again() {
        let (mut spike, _) = pending();
        let mut params = decide_params(&spike, 0, "deny", Value::Null);
        params["expected_revision"] = json!(1);
        let command = request("cmd-1", "decide_request", params);
        let first = send(&mut spike, &command, T0 + 4000);
        assert_eq!(
            error_of(&only_response(first.clone())).2,
            ErrorCode::StaleRequest
        );
        let again = send(&mut spike, &command, T0 + 4000);
        assert_eq!(again[0].to_line(), first[0].to_line());
    }

    #[test]
    fn a_reused_command_id_is_refused() {
        let (mut spike, _) = pending();
        let params = decide_params(&spike, 0, "approve", json!({"kind": "session"}));
        send(
            &mut spike,
            &request("cmd-1", "decide_request", params),
            T0 + 4000,
        );
        let other = decide_params(&spike, 0, "deny", Value::Null);
        let response = only_response(send(
            &mut spike,
            &request("cmd-1", "decide_request", other),
            T0 + 4000,
        ));
        assert_eq!(
            error_of(&response),
            (
                Some("cmd-1"),
                Some("decide_request"),
                ErrorCode::CommandReused,
                Value::Null
            )
        );
        assert_eq!(spike.requests[0].state(), RequestState::Approved);
    }

    #[test]
    fn stale_unknown_and_narrowed_decisions_are_refused() {
        let (mut spike, _) = pending();
        let base = decide_params(&spike, 0, "deny", Value::Null);
        let mut stale_revision = base.clone();
        stale_revision["expected_revision"] = json!(3);
        let mut stale_digest = base.clone();
        stale_digest["request_digest"] = json!(format!("sha256:{}", "0".repeat(64)));
        let mut unknown = base.clone();
        unknown["request_id"] = json!("spike-request-7");
        let mut narrowed = decide_params(&spike, 0, "approve", json!({"kind": "one_request"}));
        narrowed["selected"] = serde_json::to_value(&spike.requests[0].scope).unwrap();
        let cases = [
            (stale_revision, ErrorCode::StaleRequest, Value::Null),
            (stale_digest, ErrorCode::StaleRequest, Value::Null),
            (unknown, ErrorCode::NotFound, Value::Null),
            (
                narrowed,
                ErrorCode::InvalidParams,
                json!({"pointer": "/params/selected"}),
            ),
        ];
        for (n, (params, code, detail)) in cases.into_iter().enumerate() {
            let command_id = format!("cmd-{n}");
            let response = only_response(send(
                &mut spike,
                &request(&command_id, "decide_request", params),
                T0 + 4000,
            ));
            assert_eq!(
                error_of(&response),
                (
                    Some(command_id.as_str()),
                    Some("decide_request"),
                    code,
                    detail
                )
            );
        }
        assert_eq!(spike.requests[0].state(), RequestState::PendingApproval);
        assert!(spike.grants.is_empty());
        // A request that is no longer pending is stale too.
        send(
            &mut spike,
            &request("deny", "decide_request", base.clone()),
            T0 + 4000,
        );
        let response = only_response(send(
            &mut spike,
            &request("deny-again", "decide_request", base),
            T0 + 4000,
        ));
        assert_eq!(error_of(&response).2, ErrorCode::StaleRequest);
    }

    #[test]
    fn a_full_command_table_takes_no_new_command() {
        let (mut spike, _) = pending();
        let mut params = decide_params(&spike, 0, "deny", Value::Null);
        params["request_id"] = json!("spike-request-9");
        for n in 0..COMMAND_TABLE_CAPACITY {
            let response = only_response(send(
                &mut spike,
                &request(&format!("c{n}"), "decide_request", params.clone()),
                T0 + 4000,
            ));
            assert_eq!(error_of(&response).2, ErrorCode::NotFound);
        }
        let deny = decide_params(&spike, 0, "deny", Value::Null);
        let response = only_response(send(
            &mut spike,
            &request("one-more", "decide_request", deny),
            T0 + 4000,
        ));
        assert_eq!(error_of(&response).2, ErrorCode::Internal);
        assert_eq!(spike.requests[0].state(), RequestState::PendingApproval);
        assert_eq!(spike.commands.len(), COMMAND_TABLE_CAPACITY);
        // A stored command is still answered.
        let response = only_response(send(
            &mut spike,
            &request("c0", "decide_request", params),
            T0 + 4000,
        ));
        assert_eq!(error_of(&response).2, ErrorCode::NotFound);
    }

    #[test]
    fn the_other_eight_methods_are_not_implemented() {
        let methods: Vec<OwnerMethod> = OwnerMethod::ALL
            .into_iter()
            .filter(|m| {
                !matches!(
                    m,
                    OwnerMethod::Hello
                        | OwnerMethod::GetSnapshot
                        | OwnerMethod::Subscribe
                        | OwnerMethod::GetRequestDetails
                        | OwnerMethod::DecideRequest
                )
            })
            .collect();
        assert_eq!(methods.len(), 8);
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/desktop/v1/fixtures/messages");
        for method in methods {
            let name = method.as_str();
            // Any accepted request fixture of the method gives valid params.
            let value = std::fs::read_dir(&fixtures)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("accept-owner-request-")
                })
                .map(|p| {
                    let text = std::fs::read_to_string(p).unwrap();
                    serde_json::from_str::<Value>(&text).unwrap()["value"].clone()
                })
                .find(|v| v["method"] == name)
                .unwrap_or_else(|| panic!("no fixture for {name}"));
            let mut spike = Spike::new();
            let response = only_response(send(&mut spike, &value, T0));
            let (frame_id, method_seen, code, _) = error_of(&response);
            assert_eq!(frame_id, value["id"].as_str());
            assert_eq!(method_seen, Some(name));
            assert_eq!(code, ErrorCode::UnknownMethod);
            let OwnerResponse::Error { error, .. } = &response else {
                panic!()
            };
            assert_eq!(
                error.message.as_str(),
                format!("The spike broker of CD-W02 does not implement {name}.")
            );
        }
    }

    #[test]
    fn refusals_by_layer() {
        let mut spike = Spike::new();
        let mut answer = |line: &[u8]| only_response(checked(spike.handle_line(line, T0)));
        let malformed = (None, None, ErrorCode::MalformedFrame, Value::Null);
        assert_eq!(error_of(&answer(b"{not json")), malformed);
        assert_eq!(error_of(&answer(b"[]")), malformed);
        assert_eq!(error_of(&answer(b"")), malformed);
        assert_eq!(
            error_of(&answer(&vec![b' '; MAX_FRAME_BYTES + 1])),
            malformed
        );
        assert_eq!(
            error_of(&answer(
                br#"{"protocol":"fabric-desktop/2","id":"x","method":"hello","params":{}}"#
            )),
            (
                Some("x"),
                Some("hello"),
                ErrorCode::UnsupportedProtocol,
                json!({"received": "fabric-desktop/2", "supported": ["fabric-desktop/1"]})
            )
        );
        let long = format!(
            r#"{{"protocol":"{}","id":"-bad","method":7}}"#,
            "p".repeat(201)
        );
        assert_eq!(
            error_of(&answer(long.as_bytes())),
            (
                None,
                None,
                ErrorCode::UnsupportedProtocol,
                json!({"received": null, "supported": ["fabric-desktop/1"]})
            )
        );
        assert_eq!(
            error_of(&answer(
                br#"{"protocol":"fabric-desktop/1","id":"x","method":"teleport","params":{}}"#
            )),
            (
                Some("x"),
                Some("teleport"),
                ErrorCode::UnknownMethod,
                Value::Null
            )
        );
        assert_eq!(
            error_of(&answer(
                br#"{"protocol":"fabric-desktop/1","id":"x","method":"hello","params":{}}"#
            )),
            (
                Some("x"),
                Some("hello"),
                ErrorCode::InvalidParams,
                json!({"pointer": "/params"})
            )
        );
    }

    #[test]
    fn an_oversized_line_is_malformed() {
        let mut spike = Spike::new();
        let response = only_response(checked(spike.handle_oversized()));
        assert_eq!(
            error_of(&response),
            (None, None, ErrorCode::MalformedFrame, Value::Null)
        );
    }

    #[test]
    fn due_steps_run_before_a_line_is_answered() {
        let mut spike = Spike::new();
        send(
            &mut spike,
            &request("sub", "subscribe", json!({"cursor": 0})),
            T0,
        );
        let frames = send(
            &mut spike,
            &request("snap", "get_snapshot", json!({})),
            T0 + 450,
        );
        let kinds: Vec<&str> = notices(&frames).iter().map(|n| n.change.kind()).collect();
        assert_eq!(kinds, ["agent", "account"]);
        assert!(matches!(frames.last(), Some(Frame::OwnerResponse(_))));
    }
}
