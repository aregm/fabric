//! The pure mapping between the core's proof jobs and the rule gate: a job's
//! facts into the gate's context and action, and a gate's result into the core's proof result.
//!
//! Every failure is one of the fixed refusals below, never a panic. A refusal is a fixed text and
//! its cause: the text is never stored or shown; the cause is, on the request's move to
//! `undecided` (PW X12, PW 2.4).

use fabric_broker_core::format_timestamp;
use fabric_broker_core::ports::{
    FactActor, GrantFact, ObservedEvent, ProofAction, ProofDone, ProofJob, ProverCause,
};
use fabric_broker_policy::{
    Action, ActiveSession, Actor, Calendar, CalendarKey, Check, Context, Duration, Event,
    EventField, Fault, Grant, PendingEffect, PolicyError, Profile, Recipient, RecipientRule,
    RecipientSet, Tool, Verdict, Window,
};
use fabric_desktop_contracts::common::{CalendarRef, ToolName};
use fabric_desktop_contracts::owner::{self, GrantTerm, ProofPurpose};
use fabric_desktop_contracts::scalar::{Email, Id, ScalarError};

/// Why a result has no report: a fixed text, never stored or shown, and the cause the core
/// stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The fixed text, the result's `undecided_reason`.
    pub text: &'static str,
    /// What the prover states, so the core can store why the request is `undecided`.
    pub cause: ProverCause,
}

/// Why a job cannot be checked: one of the fixed refusals of this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobRefused(pub Refusal);

/// The gate is still starting.
pub const STARTING: Refusal = Refusal {
    text: "the rule prover is starting",
    cause: ProverCause::Starting,
};
/// The gate is down, a stop was sent, or the job could not be handed to the prover.
pub const NOT_RUNNING: Refusal = Refusal {
    text: "the rule prover is not running",
    cause: ProverCause::NotRunning,
};
/// The job's deadline passed while it waited.
pub const PAST_DEADLINE: Refusal = Refusal {
    text: "the proof's deadline passed before its check started",
    cause: ProverCause::DeadlinePassed,
};
/// The observed event is on another calendar than the action's.
pub const OTHER_CALENDAR: Refusal = Refusal {
    text: "the observation names another calendar",
    cause: ProverCause::JobRefused,
};
/// An observed attendee is not an email address.
pub const BAD_ATTENDEE: Refusal = Refusal {
    text: "an observed attendee is not an address",
    cause: ProverCause::JobRefused,
};
/// The action digest is not `sha256:` and 64 lowercase hex characters.
pub const BAD_DIGEST: Refusal = Refusal {
    text: "the action digest is not sha256",
    cause: ProverCause::JobRefused,
};
/// A grant's window is not a time.
pub const BAD_WINDOW: Refusal = Refusal {
    text: "a grant window is not a time",
    cause: ProverCause::JobRefused,
};
/// The tool call has no broker action.
pub const NO_ACTION: Refusal = Refusal {
    text: "no broker action for this tool call",
    cause: ProverCause::JobRefused,
};
/// The job's action is of a kind this adapter does not know.
pub const UNKNOWN_ACTION: Refusal = Refusal {
    text: "this adapter does not know the action",
    cause: ProverCause::JobRefused,
};
/// The check ran on another rule theory than the one this broker was built with.
pub const OTHER_THEORY: Refusal = Refusal {
    text: "the check ran on another rule theory",
    cause: ProverCause::ReportUnusable,
};
/// The check names another action or context than the job's.
pub const UNBOUND: Refusal = Refusal {
    text: "the check is bound to another action or context",
    cause: ProverCause::ReportUnusable,
};
/// The check's result does not fit the contract.
pub const NOT_FIT: Refusal = Refusal {
    text: "the proof did not fit the contract",
    cause: ProverCause::ReportUnusable,
};
/// A fact could not be rendered for Isabelle.
pub const RENDER: Refusal = Refusal {
    text: "a fact could not be written for Isabelle",
    cause: ProverCause::JobRefused,
};
/// The gate failed.
pub const FAILED: Refusal = Refusal {
    text: "the rule prover failed",
    cause: ProverCause::CheckNotDecided,
};
/// The check failed and returned at or after its job's deadline (PW 2.3).
pub const CHECK_LATE: Refusal = Refusal {
    text: "the check returned after the proof's deadline",
    cause: ProverCause::DeadlinePassed,
};
/// The check failed and took at least the limit it was given (PW 2.3).
pub const CHECK_LIMIT: Refusal = Refusal {
    text: "the check reached its time limit",
    cause: ProverCause::CheckLimitReached,
};

/// When a check returned, by the adapter's own clock (PW 2.3). It changes the cause of an
/// error or an undecided verdict only; a report's integrity is judged the same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lateness {
    /// Before the job's deadline and under the limit the check was given.
    InTime,
    /// Before the job's deadline, but at or over the limit the check was given.
    AtLimit,
    /// At or after the job's deadline.
    PastDeadline,
}

impl Lateness {
    /// The refusal of a failed check: `in_time` unless the check was late.
    const fn refusal(self, in_time: Refusal) -> Refusal {
        match self {
            Lateness::InTime => in_time,
            Lateness::AtLimit => CHECK_LIMIT,
            Lateness::PastDeadline => CHECK_LATE,
        }
    }

    /// The cause of an undecided verdict.
    const fn undecided_cause(self) -> ProverCause {
        match self {
            Lateness::InTime => ProverCause::CheckNotDecided,
            Lateness::AtLimit => ProverCause::CheckLimitReached,
            Lateness::PastDeadline => ProverCause::DeadlinePassed,
        }
    }
}

/// The context and the action of a job, or the fixed reason it cannot be checked.
///
/// # Errors
///
/// [`JobRefused`] with one of the fixed texts.
pub fn context_of(job: &ProofJob) -> Result<(Context, Action), JobRefused> {
    let action = match &job.action {
        ProofAction::Agent(call) => {
            Action::from_tool_call(call).map_err(|_| JobRefused(NO_ACTION))?
        }
        _ => return Err(JobRefused(UNKNOWN_ACTION)),
    };
    let facts = &job.facts;
    let event = match &facts.event {
        None => None,
        Some(observed) => Some(event_of(observed, &action)?),
    };
    let context = Context {
        actor: match &facts.actor {
            FactActor::Agent(id) => Actor::Agent(id.as_str().to_owned()),
            FactActor::System => Actor::System,
        },
        paused: facts.paused,
        now_ms: facts.now_ms,
        session: facts.session.as_ref().map(|session| ActiveSession {
            id: session.session_id.as_str().to_owned(),
            last_active_ms: session.last_active_ms,
        }),
        profile: Profile::from(&facts.profile),
        connected: facts.connected.iter().map(text).collect(),
        calendars: facts
            .calendars
            .iter()
            .map(|fact| Calendar {
                account: fact.calendar.account_id.as_str().to_owned(),
                id: fact.calendar.calendar_id.as_str().to_owned(),
                writable: fact.writable,
                drafts: fact.agent_drafts,
            })
            .collect(),
        warning_calendars: facts.warning_calendars.iter().map(key).collect(),
        grants: facts
            .grants
            .iter()
            .map(grant_of)
            .collect::<Result<_, _>>()?,
        recipient_sets: facts.recipient_sets.iter().map(recipient_set_of).collect(),
        event,
        digest: digest_hex(job.action_digest.as_str())?,
        relied: facts.relied.iter().map(text).collect(),
        result_held: facts.result_held,
        pending_effects: facts
            .pending_effects
            .iter()
            .map(|effect| PendingEffect {
                account: effect.calendar.account_id.as_str().to_owned(),
                calendar: effect.calendar.calendar_id.as_str().to_owned(),
                event_id: effect.provider_event_id.clone(),
            })
            .collect(),
        pause_epoch: facts.pause_epoch,
        revocation_epoch: facts.revocation_epoch,
    };
    Ok((context, action))
}

/// What happens to the gate after a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateFate {
    /// The gate is used again.
    Keep,
    /// The gate is dropped and a new one started.
    Restart,
}

/// The result for the core and what happens to the gate.
///
/// A report comes only from a check on `expected_theory` that names the context's action and
/// context digests and fits the contract; every other result is [`no_report`]. Only a request's
/// fault (`Fault::Request`) and a check keep the gate; a gate's fault restarts it, and a check on
/// another theory means the gate must meet the pin again.
///
/// `lateness` decides the cause of an error and of an undecided verdict (PW 2.4): past the
/// deadline it is `DeadlinePassed`, at the limit `CheckLimitReached`. Another theory, an unbound
/// check and a report that does not fit keep their cause whatever the lateness.
#[must_use]
pub fn done_of(
    job: &ProofJob,
    context: &Context,
    expected_theory: &str,
    result: Result<Check, PolicyError>,
    lateness: Lateness,
) -> (ProofDone, GateFate) {
    let check = match result {
        Ok(check) => check,
        Err(error) => {
            return match Fault::of(&error) {
                Fault::Request => (no_report(job, lateness.refusal(RENDER)), GateFate::Keep),
                Fault::Gate => (no_report(job, lateness.refusal(FAILED)), GateFate::Restart),
            };
        }
    };
    if check.theory_sha256 != expected_theory {
        return (no_report(job, OTHER_THEORY), GateFate::Restart);
    }
    if check.action_digest != context.digest
        || check.context_digest != fabric_broker_policy::context_digest(context)
    {
        return (no_report(job, UNBOUND), GateFate::Keep);
    }
    match report_of(job, check, lateness) {
        Ok(done) => (done, GateFate::Keep),
        Err(_) => (no_report(job, NOT_FIT), GateFate::Keep),
    }
}

/// A result with no report, so the core ends the job `undecided`; `refusal` is one of the fixed
/// refusals, whose text is the result's reason and whose cause the core stores.
#[must_use]
pub fn no_report(job: &ProofJob, refusal: Refusal) -> ProofDone {
    answer(job.job_id, &job.request_id, job.purpose, refusal)
}

/// [`no_report`] for a job known only by its id, request and purpose: the shell answers so a
/// job it handed to a worker that was lost (PW 2.2).
#[must_use]
pub fn answer(job_id: u64, request_id: &Id, purpose: ProofPurpose, refusal: Refusal) -> ProofDone {
    ProofDone {
        job_id,
        request_id: request_id.clone(),
        purpose,
        report: None,
        ruling: None,
        authorizing_grants: Vec::new(),
        undecided_reason: Some(refusal.text.to_owned()),
        check_theory_text: None,
        cause: Some(refusal.cause),
    }
}

/// The result of a check that ran on the built theory for exactly this job. An undecided verdict
/// states its cause by `lateness`; a decided one states none.
fn report_of(job: &ProofJob, check: Check, lateness: Lateness) -> Result<ProofDone, ScalarError> {
    let report = fabric_broker_policy::proof_report(
        &check,
        &job.request_id,
        job.purpose,
        &format_timestamp(job.started_at_ms),
    )?;
    let ruling = fabric_broker_policy::ruling(&check)?;
    let authorizing_grants = check
        .authorizing_grants
        .iter()
        .map(|id| Id::new(id.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    let (undecided_reason, cause) = match check.verdict {
        Verdict::Undecided { reason } => (Some(reason), Some(lateness.undecided_cause())),
        Verdict::Decided { .. } => (None, None),
    };
    Ok(ProofDone {
        job_id: job.job_id,
        request_id: job.request_id.clone(),
        purpose: job.purpose,
        report: Some(report),
        ruling,
        authorizing_grants,
        undecided_reason,
        check_theory_text: Some(check.theory_text),
        cause,
    })
}

/// The 64 hex characters of a contract digest (`sha256:` and 64 lowercase hex).
fn digest_hex(digest: &str) -> Result<String, JobRefused> {
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or(JobRefused(BAD_DIGEST))?;
    let lower_hex = hex
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if hex.len() == 64 && lower_hex {
        Ok(hex.to_owned())
    } else {
        Err(JobRefused(BAD_DIGEST))
    }
}

fn text(id: &Id) -> String {
    id.as_str().to_owned()
}

fn key(calendar: &CalendarRef) -> CalendarKey {
    CalendarKey::new(calendar.account_id.as_str(), calendar.calendar_id.as_str())
}

/// The account and calendar an action changes, if it names exactly one.
fn calendar_of(action: &Action) -> Option<(&str, &str)> {
    match action {
        Action::CreateEvent { account, calendar }
        | Action::UpdateEvent {
            account, calendar, ..
        }
        | Action::ChangeInvitation {
            account, calendar, ..
        }
        | Action::Respond {
            account, calendar, ..
        }
        | Action::DeleteEvent {
            account, calendar, ..
        } => Some((account.as_str(), calendar.as_str())),
        _ => None,
    }
}

/// The observed event, which must be on the calendar the action names.
fn event_of(observed: &ObservedEvent, action: &Action) -> Result<Event, JobRefused> {
    let same_calendar = calendar_of(action).is_some_and(|(account, calendar)| {
        observed.calendar.account_id.as_str() == account
            && observed.calendar.calendar_id.as_str() == calendar
    });
    if !same_calendar {
        return Err(JobRefused(OTHER_CALENDAR));
    }
    let attendees = observed
        .attendees
        .iter()
        .map(|address| {
            Email::new(address.as_str())
                .map(|email| Recipient::from_email(&email))
                .map_err(|_| JobRefused(BAD_ATTENDEE))
        })
        .collect::<Result<_, _>>()?;
    Ok(Event {
        id: observed.event_id.clone(),
        revision: observed.revision.clone(),
        fabric_created: observed.fabric_created,
        recurring: observed.recurring,
        default_type: observed.default_type,
        timed: observed.timed,
        invitation: observed.invitation,
        attendees,
        occurrence: observed.occurrence_start_ms,
    })
}

/// The gate's tool of a contract tool; the status and result tools have none, so a grant that
/// names them covers less, never more.
fn tool_of(tool: ToolName) -> Option<Tool> {
    match tool {
        ToolName::CalendarList => Some(Tool::List),
        ToolName::CalendarAvailability => Some(Tool::Availability),
        ToolName::CalendarRead => Some(Tool::Read),
        ToolName::ProposeCreate => Some(Tool::Create),
        ToolName::ProposeUpdate => Some(Tool::Update),
        ToolName::ProposeInvitation => Some(Tool::Invite),
        ToolName::ProposeResponse => Some(Tool::Respond),
        ToolName::ProposeDelete => Some(Tool::Delete),
        ToolName::RequestStatus | ToolName::RequestResult => None,
    }
}

fn grant_of(fact: &GrantFact) -> Result<Grant, JobRefused> {
    let grant = &fact.grant;
    let scope = &grant.scope;
    let duration = match &grant.term {
        GrantTerm::OneRequest { action_digest } => Duration::OneRequest {
            digest: digest_hex(action_digest.as_str())?,
            used: fact.used,
        },
        GrantTerm::Session { session_id } => Duration::Session {
            session_id: text(session_id),
        },
        GrantTerm::UntilRevoked => Duration::UntilRevoked,
    };
    let window = match &scope.window {
        None => None,
        Some(window) => Some(Window {
            start_ms: window.start.unix_millis().ok_or(JobRefused(BAD_WINDOW))?,
            end_ms: window.end.unix_millis().ok_or(JobRefused(BAD_WINDOW))?,
        }),
    };
    Ok(Grant {
        id: text(&grant.grant_id),
        tools: grant.tools.iter().copied().filter_map(tool_of).collect(),
        duration,
        ended: fact.ended,
        calendars: scope.calendars.iter().map(key).collect(),
        window,
        fields: scope
            .fields
            .as_ref()
            .map(|fields| fields.iter().copied().map(EventField::from).collect())
            .unwrap_or_default(),
        recipients: scope.recipients.as_ref().map(|rule| match rule {
            owner::RecipientRule::Exact(addresses) => RecipientRule::Exact(
                addresses
                    .iter()
                    .map(|address| address.as_str().to_owned())
                    .collect(),
            ),
            owner::RecipientRule::RecipientSets(ids) => {
                RecipientRule::Sets(ids.iter().map(text).collect())
            }
        }),
        send_updates: scope.send_updates.map(Into::into),
    })
}

/// A recipient set as written: members and domains are not lowercased, so they can only match
/// less.
fn recipient_set_of(set: &owner::RecipientSet) -> RecipientSet {
    RecipientSet {
        id: text(&set.recipient_set_id),
        members: set
            .definition
            .members
            .iter()
            .map(|member| member.as_str().to_owned())
            .collect(),
        domains: set
            .definition
            .domains
            .iter()
            .map(|domain| domain.as_str().to_owned())
            .collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! The map tests build `Check` values by hand and hand them only to the pure `done_of`;
    //! no such value reaches a worker, a shell or a broker.

    use fabric_broker_core::ports::{
        CalendarFact, Fence, PendingEffect as FactEffect, SessionFact, TrustedFacts,
    };
    use fabric_broker_policy::{
        Outcome, Rule, RuleOutcome, RuleResult, SendUpdates, context_digest, proof_id,
    };
    use fabric_desktop_contracts::agent;
    use fabric_desktop_contracts::owner::{ProfileRows, ProofOutcome, ProofPurpose};
    use fabric_desktop_contracts::scalar::Digest;
    use serde_json::json;

    use super::*;
    use crate::prover::built_theory_sha256;

    /// Every fixed refusal of this module.
    pub(crate) const REFUSALS: [Refusal; 16] = [
        STARTING,
        NOT_RUNNING,
        PAST_DEADLINE,
        OTHER_CALENDAR,
        BAD_ATTENDEE,
        BAD_DIGEST,
        BAD_WINDOW,
        NO_ACTION,
        UNKNOWN_ACTION,
        OTHER_THEORY,
        UNBOUND,
        NOT_FIT,
        RENDER,
        FAILED,
        CHECK_LATE,
        CHECK_LIMIT,
    ];

    const ACCOUNT: &str = "practice-account";
    const CALENDAR: &str = "practice-calendar";
    const STARTED_AT_MS: u64 = 1_795_000_000_000;

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    fn calendar_ref(account: &str, calendar: &str) -> CalendarRef {
        CalendarRef {
            account_id: id(account),
            calendar_id: id(calendar),
        }
    }

    /// A create on the practice calendar.
    pub(crate) fn create() -> agent::Action {
        serde_json::from_value(json!({
            "tool": "fabric_calendar_propose_create",
            "arguments": {
                "account_id": ACCOUNT,
                "calendar_id": CALENDAR,
                "event": {
                    "title": "Planning",
                    "start": {"date_time": "2026-11-20T10:00:00+00:00", "zone": "UTC"},
                    "end": {"date_time": "2026-11-20T11:00:00+00:00", "zone": "UTC"},
                    "description": null,
                    "location": null
                },
                "task": "Hold an hour."
            }
        }))
        .unwrap()
    }

    fn delete() -> agent::Action {
        serde_json::from_value(json!({
            "tool": "fabric_calendar_propose_delete",
            "arguments": {
                "account_id": ACCOUNT,
                "calendar_id": CALENDAR,
                "event": {"event_id": "event-1", "revision": "2"},
                "send_updates": "none",
                "task": "Drop it."
            }
        }))
        .unwrap()
    }

    fn grant(digest: &Digest, window_start: &str) -> GrantFact {
        GrantFact {
            grant: serde_json::from_value(json!({
                "grant_id": "grant-1",
                "revision": 1,
                "agent_id": "synthetic-agent",
                "purpose": "Plan the week.",
                "tools": [
                    "fabric_calendar_propose_create",
                    "fabric_request_status",
                    "fabric_calendar_propose_delete",
                    "fabric_request_result"
                ],
                "scope": {
                    "calendars": [{"account_id": ACCOUNT, "calendar_id": CALENDAR}],
                    "window": {"start": window_start, "end": "2026-11-21T00:00:00.000Z"},
                    "fields": ["time", "title"],
                    "recipients": {"kind": "recipient_sets", "recipient_set_ids": ["set-1"]},
                    "send_updates": "none"
                },
                "term": {"kind": "one_request", "action_digest": digest},
                "origin": {"request_id": "request-1", "approval_digest": digest},
                "policy": {"profile_id": "careful", "version": 1},
                "revocation_epoch": 0,
                "created_at": "2026-11-16T00:00:00.000Z",
                "ended": null
            }))
            .unwrap(),
            used: true,
            ended: false,
        }
    }

    /// A classification job for `action` with one grant, one recipient set and one pending
    /// effect.
    pub(crate) fn job_of(action: agent::Action) -> ProofJob {
        let digest = action.digest();
        let facts = TrustedFacts {
            actor: FactActor::Agent(id("synthetic-agent")),
            paused: false,
            now_ms: 1_795_000_000_000,
            session: Some(SessionFact {
                session_id: id("session-1"),
                last_active_ms: 1_794_999_990_000,
            }),
            profile: ProfileRows::CAREFUL,
            connected: vec![id(ACCOUNT)],
            calendars: vec![CalendarFact {
                calendar: calendar_ref(ACCOUNT, CALENDAR),
                writable: true,
                agent_drafts: false,
            }],
            warning_calendars: vec![calendar_ref(ACCOUNT, "warnings")],
            grants: vec![grant(&digest, "2026-11-20T00:00:00.000Z")],
            recipient_sets: vec![
                serde_json::from_value(json!({
                    "recipient_set_id": "set-1",
                    "revision": 1,
                    "source": {"kind": "local"},
                    "definition": {
                        "name": "Team",
                        "members": ["Ana.Lee@example.com"],
                        "domains": ["example.org"]
                    }
                }))
                .unwrap(),
            ],
            event: None,
            relied: vec![id("grant-0")],
            result_held: false,
            pending_effects: vec![FactEffect {
                calendar: calendar_ref(ACCOUNT, CALENDAR),
                provider_event_id: "event-9".to_owned(),
            }],
            pause_epoch: 3,
            revocation_epoch: 4,
        };
        ProofJob {
            job_id: 7,
            request_id: id("request-1"),
            purpose: ProofPurpose::Classification,
            action: ProofAction::Agent(action),
            action_digest: digest,
            facts,
            fence: Fence {
                pause_epoch: 3,
                revocation_epoch: 4,
                event_revision: None,
            },
            started_at_ms: STARTED_AT_MS,
            deadline_ms: STARTED_AT_MS + 35_000,
        }
    }

    fn observed(account: &str, calendar: &str, attendees: &[&str]) -> ObservedEvent {
        ObservedEvent {
            calendar: calendar_ref(account, calendar),
            event_id: "event-1".to_owned(),
            revision: "2".to_owned(),
            fabric_created: true,
            recurring: false,
            default_type: true,
            timed: true,
            invitation: false,
            attendees: attendees.iter().map(|&a| a.to_owned()).collect(),
            occurrence_start_ms: None,
        }
    }

    /// A check as the gate would return it for `context`, with every rule holding except
    /// `failing`.
    fn check_of(
        context: &Context,
        verdict: Verdict,
        failing: Option<Rule>,
        authorizing: &[&str],
    ) -> Check {
        let text = "theory Check_000001 imports Fabric_Broker.Fabric_Broker begin end".to_owned();
        Check {
            theory: "Check_000001".to_owned(),
            proof_id: proof_id(built_theory_sha256(), "Isabelle2025-2", &text),
            theory_text: text,
            theory_sha256: built_theory_sha256().to_owned(),
            isabelle: "Isabelle2025-2".to_owned(),
            context_digest: context_digest(context),
            action_digest: context.digest.clone(),
            verdict,
            failing_rule: failing,
            rules: Rule::ALL
                .iter()
                .map(|&rule| RuleResult {
                    rule,
                    outcome: if Some(rule) == failing {
                        RuleOutcome::Fails
                    } else {
                        RuleOutcome::Holds
                    },
                })
                .collect(),
            grants: Vec::new(),
            authorizing_grants: authorizing.iter().map(|&g| g.to_owned()).collect(),
            seconds: 0.25,
        }
    }

    fn decided(outcome: Outcome) -> Verdict {
        Verdict::Decided { outcome }
    }

    fn checked(job: &ProofJob) -> Context {
        context_of(job).unwrap().0
    }

    #[test]
    fn facts_map_to_the_context_field_by_field() {
        let job = job_of(create());
        let (context, action) = context_of(&job).unwrap();
        assert_eq!(
            action,
            Action::CreateEvent {
                account: ACCOUNT.to_owned(),
                calendar: CALENDAR.to_owned(),
            }
        );
        let hex = job.action_digest.as_str()["sha256:".len()..].to_owned();
        let expected = Context {
            actor: Actor::Agent("synthetic-agent".to_owned()),
            paused: false,
            now_ms: 1_795_000_000_000,
            session: Some(ActiveSession {
                id: "session-1".to_owned(),
                last_active_ms: 1_794_999_990_000,
            }),
            profile: Profile::CAREFUL,
            connected: vec![ACCOUNT.to_owned()],
            calendars: vec![Calendar {
                account: ACCOUNT.to_owned(),
                id: CALENDAR.to_owned(),
                writable: true,
                drafts: false,
            }],
            warning_calendars: vec![CalendarKey::new(ACCOUNT, "warnings")],
            grants: vec![Grant {
                id: "grant-1".to_owned(),
                tools: vec![Tool::Create, Tool::Delete],
                duration: Duration::OneRequest {
                    digest: hex.clone(),
                    used: true,
                },
                ended: false,
                calendars: vec![CalendarKey::new(ACCOUNT, CALENDAR)],
                window: Some(Window {
                    start_ms: 1_795_132_800_000,
                    end_ms: 1_795_219_200_000,
                }),
                fields: vec![EventField::Time, EventField::Title],
                recipients: Some(RecipientRule::Sets(vec!["set-1".to_owned()])),
                send_updates: Some(SendUpdates::NotifyNone),
            }],
            recipient_sets: vec![RecipientSet {
                id: "set-1".to_owned(),
                members: vec!["Ana.Lee@example.com".to_owned()],
                domains: vec!["example.org".to_owned()],
            }],
            event: None,
            digest: hex,
            relied: vec!["grant-0".to_owned()],
            result_held: false,
            pending_effects: vec![PendingEffect {
                account: ACCOUNT.to_owned(),
                calendar: CALENDAR.to_owned(),
                event_id: "event-9".to_owned(),
            }],
            pause_epoch: 3,
            revocation_epoch: 4,
        };
        assert_eq!(context, expected);

        let mut system = job_of(create());
        system.facts.actor = FactActor::System;
        system.facts.session = None;
        let context = checked(&system);
        assert_eq!(context.actor, Actor::System);
        assert_eq!(context.session, None);
    }

    #[test]
    fn the_digest_loses_its_prefix_and_comes_back_in_the_report() {
        let job = job_of(create());
        let context = checked(&job);
        assert_eq!(
            format!("sha256:{}", context.digest),
            job.action_digest.as_str()
        );
        let check = check_of(
            &context,
            decided(Outcome::NeedApproval),
            Some(Rule::Authority),
            &[],
        );
        let (done, _) = done_of(
            &job,
            &context,
            built_theory_sha256(),
            Ok(check),
            Lateness::InTime,
        );
        let report = done.report.unwrap();
        assert_eq!(report.action_digest, job.action_digest);
        assert_eq!(report.request_id, job.request_id);
        assert_eq!(report.purpose, job.purpose);
    }

    #[test]
    fn a_digest_that_is_not_sha256_fails_the_job() {
        let hex = "ab".repeat(32);
        assert_eq!(digest_hex(&format!("sha256:{hex}")), Ok(hex.clone()));
        for bad in [
            hex.clone(),
            format!("md5:{hex}"),
            format!("sha256:{}", hex.to_uppercase()),
            format!("sha256:{}", &hex[..62]),
            format!("sha256:{hex}00"),
            format!("sha256:{}zz", &hex[..62]),
        ] {
            assert_eq!(digest_hex(&bad), Err(JobRefused(BAD_DIGEST)), "{bad}");
        }
    }

    #[test]
    fn status_and_result_tools_are_dropped_from_grants() {
        let context = checked(&job_of(create()));
        assert_eq!(context.grants[0].tools, vec![Tool::Create, Tool::Delete]);
        let all = ToolName::ALL
            .iter()
            .copied()
            .filter_map(tool_of)
            .collect::<Vec<_>>();
        assert_eq!(
            all,
            vec![
                Tool::List,
                Tool::Availability,
                Tool::Read,
                Tool::Create,
                Tool::Update,
                Tool::Invite,
                Tool::Respond,
                Tool::Delete,
            ]
        );
    }

    #[test]
    fn a_window_that_is_not_a_time_fails_the_job() {
        let mut job = job_of(create());
        job.facts.grants = vec![grant(&job.action_digest, "2026-02-30T00:00:00.000Z")];
        assert_eq!(context_of(&job).unwrap_err(), JobRefused(BAD_WINDOW));
    }

    #[test]
    fn set_domains_are_passed_as_written() {
        let context = checked(&job_of(create()));
        assert_eq!(context.recipient_sets[0].members, ["Ana.Lee@example.com"]);
        assert_eq!(context.recipient_sets[0].domains, ["example.org"]);
    }

    #[test]
    fn an_observation_of_another_calendar_or_a_bad_attendee_fails_the_job() {
        let mut job = job_of(delete());
        job.facts.event = Some(observed(ACCOUNT, "other-calendar", &[]));
        assert_eq!(context_of(&job).unwrap_err(), JobRefused(OTHER_CALENDAR));
        job.facts.event = Some(observed("other-account", CALENDAR, &[]));
        assert_eq!(context_of(&job).unwrap_err(), JobRefused(OTHER_CALENDAR));
        job.facts.event = Some(observed(ACCOUNT, CALENDAR, &["not an address"]));
        assert_eq!(context_of(&job).unwrap_err(), JobRefused(BAD_ATTENDEE));

        // An observation with an action that names no calendar is refused too.
        let mut create = job_of(create());
        create.facts.event = Some(observed(ACCOUNT, CALENDAR, &[]));
        assert!(context_of(&create).is_ok());
        let mut list = job_of(
            serde_json::from_value(json!({"tool": "fabric_calendar_list", "arguments": {}}))
                .unwrap(),
        );
        list.facts.event = Some(observed(ACCOUNT, CALENDAR, &[]));
        assert_eq!(context_of(&list).unwrap_err(), JobRefused(OTHER_CALENDAR));

        job.facts.event = Some(observed(ACCOUNT, CALENDAR, &["Bo.Ray@example.org"]));
        let event = checked(&job).event.unwrap();
        assert_eq!(
            event,
            Event {
                id: "event-1".to_owned(),
                revision: "2".to_owned(),
                fabric_created: true,
                recurring: false,
                default_type: true,
                timed: true,
                invitation: false,
                attendees: vec![Recipient {
                    address: "Bo.Ray@example.org".to_owned(),
                    domain: "example.org".to_owned(),
                }],
                occurrence: None,
            }
        );
    }

    #[test]
    fn request_result_fails_the_job() {
        let job = job_of(
            serde_json::from_value(json!({
                "tool": "fabric_request_result",
                "arguments": {"request_id": "request-0"}
            }))
            .unwrap(),
        );
        assert_eq!(context_of(&job).unwrap_err(), JobRefused(NO_ACTION));
    }

    #[test]
    fn a_block_check_maps_to_report_and_ruling_with_proof_id() {
        let job = job_of(delete());
        let context = checked(&job);
        let check = check_of(
            &context,
            decided(Outcome::Block),
            Some(Rule::FabricCreated),
            &[],
        );
        let proof = check.proof_id.clone();
        let (done, fate) = done_of(
            &job,
            &context,
            built_theory_sha256(),
            Ok(check),
            Lateness::InTime,
        );
        assert_eq!(fate, GateFate::Keep);
        assert_eq!((done.job_id, &done.request_id), (7, &job.request_id));
        let report = done.report.unwrap();
        assert_eq!(report.outcome, ProofOutcome::Block);
        assert_eq!(report.proof_id.as_str(), proof);
        assert_eq!(report.failing_rule.unwrap().as_str(), "fabric_created");
        assert_eq!(report.rule_verdicts.len(), 18);
        assert_eq!(
            report.theory_hash.as_str(),
            format!("sha256:{}", built_theory_sha256())
        );
        assert_eq!(report.started_at, format_timestamp(STARTED_AT_MS));
        assert_eq!(report.elapsed_ms.get(), 250);
        let ruling = done.ruling.unwrap();
        assert_eq!(ruling.rule.as_str(), "fabric_created");
        assert_eq!(ruling.proof_id.as_str(), proof);
        assert_eq!(
            ruling.explanation.as_str(),
            Rule::FabricCreated.explanation()
        );
        assert!(done.authorizing_grants.is_empty());
        assert_eq!(done.undecided_reason, None);
        assert_eq!(done.cause, None);
        assert!(
            done.check_theory_text
                .unwrap()
                .starts_with("theory Check_000001")
        );
    }

    #[test]
    fn a_permit_check_names_its_authorizing_grants_in_order() {
        let mut job = job_of(create());
        job.purpose = ProofPurpose::FreshPermit;
        let context = checked(&job);
        let check = check_of(
            &context,
            decided(Outcome::Permit),
            None,
            &["grant-1", "grant-0"],
        );
        let (done, fate) = done_of(
            &job,
            &context,
            built_theory_sha256(),
            Ok(check),
            Lateness::InTime,
        );
        assert_eq!(fate, GateFate::Keep);
        assert_eq!(done.authorizing_grants, [id("grant-1"), id("grant-0")]);
        assert_eq!(done.ruling, None);
        let report = done.report.unwrap();
        assert_eq!(report.outcome, ProofOutcome::Permit);
        assert_eq!(report.purpose, ProofPurpose::FreshPermit);
        assert_eq!(report.failing_rule, None);
    }

    #[test]
    fn undecided_keeps_its_reason_and_names_no_grant() {
        let job = job_of(create());
        let context = checked(&job);
        let reason = "Isabelle did not finish the check".to_owned();
        let check = check_of(
            &context,
            Verdict::Undecided {
                reason: reason.clone(),
            },
            None,
            &[],
        );
        let (done, fate) = done_of(
            &job,
            &context,
            built_theory_sha256(),
            Ok(check),
            Lateness::InTime,
        );
        assert_eq!(fate, GateFate::Keep);
        assert_eq!(done.report.unwrap().outcome, ProofOutcome::Undecided);
        assert_eq!(done.ruling, None);
        assert!(done.authorizing_grants.is_empty());
        assert_eq!(done.undecided_reason, Some(reason));
        assert_eq!(done.cause, Some(ProverCause::CheckNotDecided));
    }

    #[test]
    fn a_report_that_does_not_fit_the_contract_is_no_report() {
        let job = job_of(create());
        let context = checked(&job);
        let fits = || check_of(&context, decided(Outcome::Permit), None, &["grant-1"]);
        let mut bad_proof_id = fits();
        bad_proof_id.proof_id = "not an id!".to_owned();
        let mut bad_version = fits();
        bad_version.isabelle = "x".repeat(10_000);
        let mut bad_grant = fits();
        bad_grant.authorizing_grants = vec!["grant 1".to_owned()];
        for check in [bad_proof_id, bad_version, bad_grant] {
            let (done, fate) = done_of(
                &job,
                &context,
                built_theory_sha256(),
                Ok(check),
                Lateness::InTime,
            );
            assert_eq!(fate, GateFate::Keep);
            assert_eq!(done, no_report(&job, NOT_FIT));
        }
    }

    fn errors(text: &str) -> Vec<PolicyError> {
        vec![
            PolicyError::Render(text.to_owned()),
            PolicyError::Io(std::io::Error::other(text.to_owned())),
            PolicyError::Protocol(text.to_owned()),
            PolicyError::Isabelle(text.to_owned()),
            PolicyError::Json(serde_json::from_str::<u8>(text).unwrap_err()),
        ]
    }

    #[test]
    fn only_a_render_error_keeps_the_gate() {
        let job = job_of(create());
        let context = checked(&job);
        let faults = errors("x").iter().map(Fault::of).collect::<Vec<_>>();
        let fates = errors("x")
            .into_iter()
            .map(|error| {
                done_of(
                    &job,
                    &context,
                    built_theory_sha256(),
                    Err(error),
                    Lateness::InTime,
                )
            })
            .collect::<Vec<_>>();
        for (fault, (_, fate)) in faults.iter().zip(&fates) {
            assert_eq!(*fate == GateFate::Restart, *fault == Fault::Gate);
        }
        assert_eq!(
            fates,
            vec![
                (no_report(&job, RENDER), GateFate::Keep),
                (no_report(&job, FAILED), GateFate::Restart),
                (no_report(&job, FAILED), GateFate::Restart),
                (no_report(&job, FAILED), GateFate::Restart),
                (no_report(&job, FAILED), GateFate::Restart),
            ]
        );
    }

    #[test]
    fn no_policy_error_text_reaches_the_reason() {
        let job = job_of(create());
        let context = checked(&job);
        for error in errors("marker 5f1c Planning") {
            let (done, _) = done_of(
                &job,
                &context,
                built_theory_sha256(),
                Err(error),
                Lateness::InTime,
            );
            let reason = done.undecided_reason.unwrap();
            assert!(
                [RENDER.text, FAILED.text].contains(&reason.as_str()),
                "{reason}"
            );
            assert!(done.report.is_none() && done.check_theory_text.is_none());
        }
    }

    #[test]
    fn a_check_on_another_theory_is_no_report_and_restarts() {
        let job = job_of(create());
        let context = checked(&job);
        let mut check = check_of(&context, decided(Outcome::Permit), None, &["grant-1"]);
        check.theory_sha256 = "0".repeat(64);
        let (done, fate) = done_of(
            &job,
            &context,
            built_theory_sha256(),
            Ok(check),
            Lateness::InTime,
        );
        assert_eq!(
            (done, fate),
            (no_report(&job, OTHER_THEORY), GateFate::Restart)
        );
    }

    #[test]
    fn a_check_bound_to_another_action_or_context_is_no_report() {
        let job = job_of(create());
        let context = checked(&job);
        let mut other_action = check_of(&context, decided(Outcome::Permit), None, &["grant-1"]);
        other_action.action_digest = "cd".repeat(32);
        let mut other_context = context.clone();
        other_context.pause_epoch += 1;
        let other_context = check_of(&other_context, decided(Outcome::Permit), None, &["grant-1"]);
        for check in [other_action, other_context] {
            let (done, fate) = done_of(
                &job,
                &context,
                built_theory_sha256(),
                Ok(check),
                Lateness::InTime,
            );
            assert_eq!((done, fate), (no_report(&job, UNBOUND), GateFate::Keep));
        }
    }

    /// PW u17: `no_report`, and `answer` for a job known by its id, copy the job, the text and the
    /// cause.
    #[test]
    fn no_report_copies_the_job_and_names_the_reason() {
        let job = job_of(create());
        let done = no_report(&job, STARTING);
        assert_eq!(
            done,
            ProofDone {
                job_id: 7,
                request_id: id("request-1"),
                purpose: ProofPurpose::Classification,
                report: None,
                ruling: None,
                authorizing_grants: Vec::new(),
                undecided_reason: Some(STARTING.text.to_owned()),
                check_theory_text: None,
                cause: Some(ProverCause::Starting),
            }
        );
        assert_eq!(
            answer(job.job_id, &job.request_id, job.purpose, STARTING),
            done
        );
    }

    /// PW u18: every fixed refusal names its cause, and no two share a text.
    #[test]
    fn every_refusal_names_its_cause() {
        let table = [
            (STARTING, ProverCause::Starting),
            (NOT_RUNNING, ProverCause::NotRunning),
            (PAST_DEADLINE, ProverCause::DeadlinePassed),
            (OTHER_CALENDAR, ProverCause::JobRefused),
            (BAD_ATTENDEE, ProverCause::JobRefused),
            (BAD_DIGEST, ProverCause::JobRefused),
            (BAD_WINDOW, ProverCause::JobRefused),
            (NO_ACTION, ProverCause::JobRefused),
            (UNKNOWN_ACTION, ProverCause::JobRefused),
            (OTHER_THEORY, ProverCause::ReportUnusable),
            (UNBOUND, ProverCause::ReportUnusable),
            (NOT_FIT, ProverCause::ReportUnusable),
            (RENDER, ProverCause::JobRefused),
            (FAILED, ProverCause::CheckNotDecided),
            (CHECK_LATE, ProverCause::DeadlinePassed),
            (CHECK_LIMIT, ProverCause::CheckLimitReached),
        ];
        let job = job_of(create());
        for (refusal, cause) in table {
            assert_eq!(refusal.cause, cause, "{}", refusal.text);
            let done = no_report(&job, refusal);
            assert_eq!(done.undecided_reason.as_deref(), Some(refusal.text));
            assert_eq!(done.cause, Some(cause), "{}", refusal.text);
        }
        assert_eq!(table.map(|(refusal, _)| refusal), REFUSALS);
        let texts = table
            .iter()
            .map(|(refusal, _)| refusal.text)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(texts.len(), table.len(), "every text is its own");
    }

    /// PW u19 (PW 2.4): lateness decides the cause of an error and of an undecided verdict, never
    /// the fate of the gate, and never the cause of a report that cannot be used.
    #[test]
    fn done_of_states_the_cause_and_lateness() {
        let job = job_of(create());
        let context = checked(&job);
        let all = [Lateness::InTime, Lateness::AtLimit, Lateness::PastDeadline];
        let theory = built_theory_sha256();
        for lateness in all {
            let late = |in_time: Refusal| match lateness {
                Lateness::InTime => in_time,
                Lateness::AtLimit => CHECK_LIMIT,
                Lateness::PastDeadline => CHECK_LATE,
            };
            let render = done_of(&job, &context, theory, Err(errors("x").remove(0)), lateness);
            assert_eq!(
                render,
                (no_report(&job, late(RENDER)), GateFate::Keep),
                "{lateness:?}"
            );
            let gate = done_of(&job, &context, theory, Err(errors("x").remove(3)), lateness);
            assert_eq!(
                gate,
                (no_report(&job, late(FAILED)), GateFate::Restart),
                "{lateness:?}"
            );

            let mut other_theory = check_of(&context, decided(Outcome::Permit), None, &[]);
            other_theory.theory_sha256 = "0".repeat(64);
            let mut unbound = check_of(&context, decided(Outcome::Permit), None, &[]);
            unbound.action_digest = "cd".repeat(32);
            let mut not_fit = check_of(&context, decided(Outcome::Permit), None, &[]);
            not_fit.proof_id = "not an id!".to_owned();
            let unusable = [
                (other_theory, OTHER_THEORY, GateFate::Restart),
                (unbound, UNBOUND, GateFate::Keep),
                (not_fit, NOT_FIT, GateFate::Keep),
            ];
            for (check, refusal, fate) in unusable {
                assert_eq!(
                    done_of(&job, &context, theory, Ok(check), lateness),
                    (no_report(&job, refusal), fate),
                    "{lateness:?} {}",
                    refusal.text
                );
            }

            let undecided = check_of(
                &context,
                Verdict::Undecided {
                    reason: "Isabelle did not finish the check".to_owned(),
                },
                None,
                &[],
            );
            let (done, fate) = done_of(&job, &context, theory, Ok(undecided), lateness);
            assert_eq!(fate, GateFate::Keep);
            assert!(done.report.is_some());
            assert_eq!(
                done.undecided_reason.as_deref(),
                Some("Isabelle did not finish the check")
            );
            let expected = match lateness {
                Lateness::InTime => ProverCause::CheckNotDecided,
                Lateness::AtLimit => ProverCause::CheckLimitReached,
                Lateness::PastDeadline => ProverCause::DeadlinePassed,
            };
            assert_eq!(done.cause, Some(expected), "{lateness:?}");

            let permit = check_of(&context, decided(Outcome::Permit), None, &[]);
            let (done, _) = done_of(&job, &context, theory, Ok(permit), lateness);
            assert!(done.report.is_some());
            assert_eq!(done.cause, None, "{lateness:?}");
        }
    }
}
