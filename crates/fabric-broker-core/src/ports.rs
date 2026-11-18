//! What the prover, the provider and the agent adapter exchange with the core (design 10.3).
//!
//! The W03 wiring implements [`Prover`], CD-W05 implements [`Provider`], and the process shell
//! runs both on worker threads.

use fabric_desktop_contracts::Frame;
use fabric_desktop_contracts::agent::{Action, AgentRequest};
use fabric_desktop_contracts::common::{CalendarRef, EventRef, FailureCode};
use fabric_desktop_contracts::owner::{
    Authorization, Grant, ProfileRows, ProofPurpose, ProofReport, ProverHealth, ProverProblem,
    RecipientSet, Ruling,
};
use fabric_desktop_contracts::scalar::{Digest, Explanation, Id};
pub use fabric_storage::{Fence, ProverKind, UndecidedCause};

/// Fixed at start; the core takes the kind of every result from here (D13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProverIdentity {
    /// The kind of prover.
    pub kind: ProverKind,
    /// Its status at start, which the owner sees as health.
    pub health: ProverStatus,
}

/// The prover's status: its health and, while it is unavailable, the problem when known. A
/// problem together with `Starting` or `Ready` cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProverStatus {
    /// The prover is starting.
    Starting,
    /// The prover is ready.
    Ready,
    /// The prover is unavailable; `None` when the cause is not known.
    Unavailable(Option<ProverProblem>),
}

impl ProverStatus {
    /// The contract's `prover` health.
    #[must_use]
    pub const fn health(self) -> ProverHealth {
        match self {
            ProverStatus::Starting => ProverHealth::Starting,
            ProverStatus::Ready => ProverHealth::Ready,
            ProverStatus::Unavailable(_) => ProverHealth::Unavailable,
        }
    }

    /// The contract's `prover_problem`: `Some` only while unavailable with a known cause.
    #[must_use]
    pub const fn problem(self) -> Option<ProverProblem> {
        match self {
            ProverStatus::Unavailable(problem) => problem,
            ProverStatus::Starting | ProverStatus::Ready => None,
        }
    }
}

/// Why a prover's result decides nothing: what it states with no report, or with an undecided
/// verdict. The core stores its own cause for each (`UndecidedCause`), and ignores this for a
/// prover whose kind is not Isabelle: the kind decides then. A synthetic run stores the same cause
/// as an Isabelle run on the paths outside the verdict (spent re-proofs, a result outside the
/// facts, an unbound report, the watchdog).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProverCause {
    /// The prover was starting.
    Starting,
    /// The prover was down, stopping or lost, or the job could not be handed to it.
    NotRunning,
    /// The job's deadline passed before the check, or by the time it returned.
    DeadlinePassed,
    /// The check reached the time limit the prover gave it.
    CheckLimitReached,
    /// A check ran and did not decide, or the gate failed.
    CheckNotDecided,
    /// The prover's report could not be used: another theory, unbound or not fit.
    ReportUnusable,
    /// The prover refused the job before any check, for a reason that is not its state or time.
    JobRefused,
}

/// What is proved. CD-W05/W06 add `Disclose` and system variants.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProofAction {
    /// An agent's proposed action.
    Agent(Action),
}

/// Who acts in a proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactActor {
    /// The agent.
    Agent(Id),
    /// The broker itself.
    System,
}

/// The agent's live session of this run; `last_active_ms` from memory (D35).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFact {
    /// The session.
    pub session_id: Id,
    /// The agent's last activity in it.
    pub last_active_ms: i64,
}

/// A selected calendar of a connected account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarFact {
    /// The calendar.
    pub calendar: CalendarRef,
    /// True when the owner may write to it.
    pub writable: bool,
    /// True for the Agent Drafts calendar.
    pub agent_drafts: bool,
}

/// One grant of the agent. `ended`: ended, or failing the validity predicate (5.5) at `now_ms`. `used`: for a
/// one-request grant, the change it permits was applied (for a read: its result released, CD-W05/W06).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantFact {
    /// The grant.
    pub grant: Grant,
    /// True when a one-request grant was used.
    pub used: bool,
    /// True when ended or no longer valid.
    pub ended: bool,
}

/// The broker's observation of the event an action names (CD-W05 fills it; `None` in W04).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedEvent {
    /// The event's calendar.
    pub calendar: CalendarRef,
    /// The event.
    pub event_id: String,
    /// The observed revision.
    pub revision: String,
    /// True when Fabric created the event.
    pub fabric_created: bool,
    /// True for a recurring event.
    pub recurring: bool,
    /// True for an event of the default type.
    pub default_type: bool,
    /// True for a timed (not all-day) event.
    pub timed: bool,
    /// The owner is an attendee, not the organizer.
    pub invitation: bool,
    /// The other people's addresses as written.
    pub attendees: Vec<String>,
    /// The original start of an occurrence.
    pub occurrence_start_ms: Option<i64>,
}

/// From `ReadTx::open_effects`: `calendar` from the effect's account and calendar; `provider_event_id` is
/// `OpenEffect.event_id` (a create's reserved id, or the action's target event for every other tool).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingEffect {
    /// The calendar the effect changes.
    pub calendar: CalendarRef,
    /// The event it targets.
    pub provider_event_id: String,
}

/// Broker records for one proof, read in one transaction plus the core's activity (THEORY section 1; L6).
/// Nothing from an agent or a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedFacts {
    /// Who acts.
    pub actor: FactActor,
    /// True while the agent is paused.
    pub paused: bool,
    /// The core clock's `now`.
    pub now_ms: i64,
    /// The agent's live session of this run.
    pub session: Option<SessionFact>,
    /// The profile in force.
    pub profile: ProfileRows,
    /// Every connected account (L6 and 11.3 W2: not filtered to the accounts the action names).
    pub connected: Vec<Id>,
    /// Every selected calendar of those accounts.
    pub calendars: Vec<CalendarFact>,
    /// The owner's warning calendars.
    pub warning_calendars: Vec<CalendarRef>,
    /// The agent's live grants (one_request only for its origin request) + every relied grant, even ended.
    pub grants: Vec<GrantFact>,
    /// The sets those grants name.
    pub recipient_sets: Vec<RecipientSet>,
    /// The observation of the named event.
    pub event: Option<ObservedEvent>,
    /// Empty for classification; for a fresh permit the request's stored relied list (D36, L7).
    pub relied: Vec<Id>,
    /// False until CD-W05/W06.
    pub result_held: bool,
    /// `ReadTx::open_effects`.
    pub pending_effects: Vec<PendingEffect>,
    /// `pause_epoch`.
    pub pause_epoch: u64,
    /// `revocation_epoch`.
    pub revocation_epoch: u64,
}

/// One proof for a worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofJob {
    /// The job, unique in the run.
    pub job_id: u64,
    /// The request.
    pub request_id: Id,
    /// Why the proof runs.
    pub purpose: ProofPurpose,
    /// What is proved.
    pub action: ProofAction,
    /// Contract form "sha256:<64 hex>"; the W03 adapter strips the prefix. For a release (later), the digest of the
    /// action whose result is released.
    pub action_digest: Digest,
    /// The facts.
    pub facts: TrustedFacts,
    /// The versions the facts depend on.
    pub fence: Fence,
    /// When the job started.
    pub started_at_ms: u64,
    /// Started + 35_000.
    pub deadline_ms: u64,
}

/// A prover never errs: every failure of the prover itself is a ProofDone without a permit. No result can fault the
/// store: one whose `authorizing_grants` names an id outside `job.facts.grants` is refused and the request goes
/// `undecided` (5.7 grant check, D17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofDone {
    /// The job.
    pub job_id: u64,
    /// The request.
    pub request_id: Id,
    /// Why the proof ran.
    pub purpose: ProofPurpose,
    /// None when no check ran -> undecided.
    pub report: Option<ProofReport>,
    /// Required for block and need_approval.
    pub ruling: Option<Ruling>,
    /// The agent's grants the prover proved authorize the action (W03 `Check.authorizing_grants`), in its order.
    /// Each must be among `job.facts.grants`; otherwise the core refuses the result (5.7).
    pub authorizing_grants: Vec<Id>,
    /// Stored only in the encrypted store.
    pub undecided_reason: Option<String>,
    /// Stored only in the encrypted store.
    pub check_theory_text: Option<String>,
    /// Why nothing was decided: required with no report and with an undecided verdict, `None`
    /// otherwise. The core stores `not_stated` for a result with no report and no cause.
    pub cause: Option<ProverCause>,
}

/// Runs on a worker thread owned by the composition.
pub trait Prover: Send {
    /// Fixed at start.
    fn identity(&self) -> ProverIdentity;
    /// Proves one job.
    fn prove(&mut self, job: &ProofJob) -> ProofDone;
}

/// One attempt for the provider worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchOrder {
    /// The request.
    pub request_id: Id,
    /// The effect.
    pub effect_id: Id,
    /// The attempt's number.
    pub attempt: u32,
    /// Consumed in the committed dispatch-intent transaction.
    pub authorization: Authorization,
    /// The action to carry out.
    pub action: Action,
    /// The account it changes.
    pub account_id: Id,
    /// The calendar it changes.
    pub calendar_id: Id,
    /// Stable id for creates, persisted before the first call.
    pub provider_event_id: Option<String>,
    /// Started + 30_000 (PLAN 5).
    pub deadline_ms: u64,
}

/// What one attempt found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptReport {
    /// Google applied the change.
    Applied {
        /// The event changed, if known.
        event: Option<EventRef>,
    },
    /// A failure that may pass.
    Transient {
        /// Google's hint for the next attempt.
        retry_after_ms: Option<u64>,
    },
    /// Google refused the change.
    Rejected {
        /// The failure.
        code: FailureCode,
        /// Its explanation.
        explanation: Explanation,
    },
    /// No answer before the deadline: the outcome is unknown.
    NoAnswer,
}

/// The result of one attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptDone {
    /// The request.
    pub request_id: Id,
    /// The effect.
    pub effect_id: Id,
    /// The attempt's number.
    pub attempt: u32,
    /// What it found.
    pub report: AttemptReport,
    /// When it finished.
    pub finished_at_ms: u64,
}

/// CD-W05 implements and extends (reconcile, revoke token). Runs on a worker thread.
pub trait Provider: Send {
    /// Carries out one attempt.
    fn dispatch(&mut self, order: &DispatchOrder) -> AttemptDone;
}

/// One agent connection of the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// What the agent adapter hands to the core.
#[allow(
    clippy::large_enum_variant,
    reason = "one input at a time on the main thread"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentInput {
    /// The adapter proved this enrolled agent's identity (CD-W06 handshake); opens a session.
    Connect {
        /// The connection.
        connection: ConnectionId,
        /// The agent.
        agent_id: Id,
    },
    /// One decoded agent frame (a proposal or `fabric_request_status`).
    Call {
        /// The connection.
        connection: ConnectionId,
        /// The frame.
        request: AgentRequest,
    },
    /// The connection closed.
    Disconnect {
        /// The connection.
        connection: ConnectionId,
    },
}

/// A frame for one agent connection: `Frame::AgentResponse`.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentReply {
    /// The connection.
    pub connection: ConnectionId,
    /// The frame.
    pub frame: Frame,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// c32. 10.3: only an unavailable status names a problem, and each status maps to its health.
    #[test]
    fn prover_status_has_no_cause_unless_unavailable() {
        let table = [
            (ProverStatus::Starting, ProverHealth::Starting, None),
            (ProverStatus::Ready, ProverHealth::Ready, None),
            (
                ProverStatus::Unavailable(None),
                ProverHealth::Unavailable,
                None,
            ),
            (
                ProverStatus::Unavailable(Some(ProverProblem::WrongRules)),
                ProverHealth::Unavailable,
                Some(ProverProblem::WrongRules),
            ),
        ];
        for (status, health, problem) in table {
            assert_eq!(status.health(), health, "{status:?}");
            assert_eq!(status.problem(), problem, "{status:?}");
        }
    }
}
