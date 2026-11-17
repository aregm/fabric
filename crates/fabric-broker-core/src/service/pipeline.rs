//! The pipeline (5.7): proofs, authorizations, dispatch intents and attempt results.
//!
//! The pump takes one due request at a time and moves it as far as it can go without an
//! outside answer, in one commit; a proof result and an attempt result are each one commit.
//! Nothing leaves for a worker before the commit that records it returned (7.1).

use fabric_desktop_contracts::common::FailureCode;
use fabric_desktop_contracts::owner::{
    ApprovalCard, Attempt, AttemptOutcome, Authorization, Duration, DurationKind, Failure,
    LifecycleStatus, LifetimeSeconds, ProofOutcome, ProofPurpose, ProofReport, Ruling,
};
use fabric_desktop_contracts::scalar::{Explanation, Fixed, Id, PositiveCount};
use fabric_desktop_contracts::{AUTHORIZATION_LIFETIME_SECONDS, RequestState};
use fabric_storage::{
    Actor, AttemptRow, AuthorizationRecord, AuthorizationState, EffectRow, Mutation, ProofRecord,
    ProverKind, ReadTx, Reproofs, RequestRecord, StoreFault, UndecidedCause, VoidReason, WriteTx,
};

use crate::broker::{Job, MAX_REPROOFS, PROVIDER_CALL_MS, RetryPolicy};
use crate::ports::{AttemptDone, AttemptReport, DispatchOrder, ProofDone, ProverCause};
use crate::service::core::{Core, InFlight};
use crate::service::grants::grant_is_valid;
use crate::service::reducer::{Event, next, states_a_cause};
use crate::service::relied::set_relied;
use crate::service::requests::{calendars_of, is_change, move_request_with, record_of};
use crate::service::{Activity, Cx, epochs, facts, invariant, plain, timestamp};

/// The `undecided_reason` of a result naming a grant outside its job's facts (D17).
const OUTSIDE_THE_FACTS: &str = "authorizing grant outside the facts";

/// The state a proof of `purpose` runs in.
#[must_use]
pub(crate) fn proving_state(purpose: ProofPurpose) -> RequestState {
    match purpose {
        ProofPurpose::Classification => RequestState::ProvingClassification,
        ProofPurpose::FreshPermit => RequestState::ProvingFreshPermit,
    }
}

/// The status `event` moves a request in `from` to; `status` is used for the states that carry
/// more than their name.
fn status_after(
    from: RequestState,
    event: Event,
    status: Option<LifecycleStatus>,
) -> Result<LifecycleStatus, StoreFault> {
    let to = next(from, event).ok_or(invariant("the reducer refuses the move"))?;
    match status {
        Some(status) if status.state() == to => Ok(status),
        Some(_) => Err(invariant("a status names the reducer's state")),
        None => plain(to),
    }
}

/// Moves the request by `event` (5.7): the reducer names the state. The move to `undecided` is
/// refused here: it states its cause, so only [`undecide`] makes it.
pub(in crate::service) fn advance(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    event: Event,
    status: Option<LifecycleStatus>,
    due_at_ms: Option<i64>,
) -> Result<(), StoreFault> {
    if states_a_cause(event) {
        return Err(invariant("undecided names its cause"));
    }
    let from = record_of(tx, request_id)?.row.request.status.state();
    let to = status_after(from, event, status)?;
    move_request_with(tx, cx, request_id, to, due_at_ms, None).map(|_| ())
}

/// Moves the request to `undecided` with `cause`, stored on the move in the same commit: the
/// only way into `undecided`. The commit is correlated to the request (`Core::meta`), where the
/// owner's details read the cause back.
pub(in crate::service) fn undecide(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    cause: UndecidedCause,
) -> Result<(), StoreFault> {
    let from = record_of(tx, request_id)?.row.request.status.state();
    let to = status_after(from, Event::Undecide, None)?;
    move_request_with(tx, cx, request_id, to, None, Some(cause)).map(|_| ())
}

/// The cause the broker stores for what a prover stated. Exhaustive: a new prover cause must be
/// given its stored cause here.
#[must_use]
pub(crate) const fn stored_cause(cause: ProverCause) -> UndecidedCause {
    match cause {
        ProverCause::Starting => UndecidedCause::ProverStarting,
        ProverCause::NotRunning => UndecidedCause::ProverNotRunning,
        ProverCause::DeadlinePassed => UndecidedCause::DeadlinePassed,
        ProverCause::CheckLimitReached => UndecidedCause::CheckLimitReached,
        ProverCause::CheckNotDecided => UndecidedCause::CheckNotDecided,
        ProverCause::ReportUnusable => UndecidedCause::ReportUnusable,
        ProverCause::JobRefused => UndecidedCause::JobRefused,
    }
}

/// True when every id the result names is among the grants its job's facts offered (5.7,
/// D17). It runs before the fence or the verdict is looked at, for every purpose and verdict.
#[must_use]
pub(crate) fn grant_check(authorizing: &[Id], offered: &[Id]) -> bool {
    authorizing.iter().all(|id| offered.contains(id))
}

/// The re-proof counts after one more re-proof of `purpose`, or `None` when its counter is at
/// `MAX_REPROOFS` (D16). Counted over the request's life, never reset.
#[must_use]
pub(crate) fn reproof(reproofs: Reproofs, purpose: ProofPurpose) -> Option<Reproofs> {
    let mut next = reproofs;
    let count = match purpose {
        ProofPurpose::Classification => &mut next.classification,
        ProofPurpose::FreshPermit => &mut next.fresh_permit,
    };
    if *count >= MAX_REPROOFS {
        return None;
    }
    *count += 1;
    Some(next)
}

/// When a transient failure may be retried (5.7, D16): the due time of the retry, or
/// `None` when the retries or the budget are spent. `attempts_started` counts the attempts
/// handed off so far; the retries already made are one fewer.
#[must_use]
pub(crate) fn retry_due(
    policy: &RetryPolicy,
    attempts_started: u32,
    first_attempt_at_ms: i64,
    retry_after_ms: Option<u64>,
    now_ms: i64,
) -> Option<i64> {
    let retries = attempts_started.saturating_sub(1);
    if retries >= policy.max_retries {
        return None;
    }
    let base = policy
        .backoff_ms
        .get(retries as usize)
        .or(policy.backoff_ms.last())
        .copied()
        .unwrap_or(0);
    let backoff = i64::try_from(base.max(retry_after_ms.unwrap_or(0))).ok()?;
    let due = now_ms.checked_add(backoff)?;
    let budget = i64::try_from(policy.budget_ms).ok()?;
    (due <= first_attempt_at_ms.saturating_add(budget)).then_some(due)
}

/// What an accepted result does (5.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Accepted {
    /// Classification Permit from Isabelle: `allowed`, resting on the authorizing grants.
    Allow,
    /// Fresh Permit that can authorize: an authorization, then `queued`.
    Authorize,
    /// NeedApproval with a ruling: a card.
    Ask(Ruling),
    /// Block with a ruling.
    Block(Ruling),
    /// Anything else: `undecided`, with its cause.
    Undecided(UndecidedCause),
}

/// What the broker knows about an accepted result besides the result itself.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context {
    pub(crate) kind: ProverKind,
    pub(crate) provider_present: bool,
    pub(crate) agent_paused: bool,
    /// The request's tool changes a calendar.
    pub(crate) change: bool,
    /// The request has a scope a card can show.
    pub(crate) scoped: bool,
}

/// What an accepted result of `purpose` leads to (5.7, D13, D17, L7(d)). `stated` is the cause
/// the prover stated for an undecided verdict.
///
/// Every `undecided` names its cause. For a kind that is not Isabelle it is `no_prover`, whatever
/// the result states (D13). For an Isabelle fresh Permit the checks run in a fixed order: no
/// authorizing grant, then a tool that changes no calendar, then a paused agent, then no
/// provider: the report's own defect first, then the request, the agent and the build.
#[must_use]
pub(crate) fn accepted(
    purpose: ProofPurpose,
    outcome: ProofOutcome,
    ruling: Option<Ruling>,
    authorizing: &[Id],
    stated: Option<UndecidedCause>,
    context: Context,
) -> Accepted {
    let isabelle = context.kind == ProverKind::Isabelle;
    let undecided = |cause| {
        Accepted::Undecided(if isabelle {
            cause
        } else {
            UndecidedCause::NoProver
        })
    };
    match outcome {
        ProofOutcome::Permit => match purpose {
            ProofPurpose::Classification if isabelle => Accepted::Allow,
            ProofPurpose::Classification => undecided(UndecidedCause::NoProver),
            ProofPurpose::FreshPermit => {
                if authorizing.is_empty() {
                    undecided(UndecidedCause::NoGrant)
                } else if !context.change {
                    undecided(UndecidedCause::ReadTool)
                } else if context.agent_paused {
                    undecided(UndecidedCause::AgentPaused)
                } else if !context.provider_present {
                    undecided(UndecidedCause::NoProvider)
                } else if isabelle {
                    Accepted::Authorize
                } else {
                    undecided(UndecidedCause::NoProver)
                }
            }
        },
        ProofOutcome::NeedApproval => match ruling {
            Some(ruling) if context.scoped => Accepted::Ask(ruling),
            Some(_) => undecided(UndecidedCause::NoScope),
            None => undecided(UndecidedCause::NoRuling),
        },
        ProofOutcome::Block => match ruling {
            Some(ruling) => Accepted::Block(ruling),
            None => undecided(UndecidedCause::NoRuling),
        },
        ProofOutcome::Undecided => undecided(stated.unwrap_or(UndecidedCause::CheckNotDecided)),
    }
}

/// How a proof result is handled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Judgement {
    /// An authorizing grant outside the job's facts: recorded, `undecided`, relied list kept.
    OutsideFacts,
    /// No report: no check ran; `undecided` with the cause the prover stated, nothing recorded.
    NoReport,
    /// A report bound to another job (its request, purpose or action digest differs from the
    /// job's): it proves nothing about this request; `undecided`, nothing recorded, relied list
    /// kept. Never filed under the report's request, so it cannot fault the store (5.7, c18).
    Unbound,
    /// The fence changed, or a fresh Permit rests on a grant no longer valid: recorded, proved
    /// again while the purpose's counter allows.
    Discard,
    /// Accepted.
    Accept(Accepted),
}

/// True when `report` is the proof of `job`: the same request, purpose and action digest
/// (PLAN 8 step 5, proof binding).
fn bound_to(report: &ProofReport, job: &InFlight) -> bool {
    report.request_id == job.request_id
        && report.purpose == job.purpose
        && report.action_digest == job.action_digest
}

/// Judges a result in one read (5.7): the report's binding to its job first, then the grant
/// check, the report, the fence, the validity of a fresh Permit's grants, and the verdict. `None` when the request is no longer
/// in the job's proving state, so the result is ignored.
fn judge(
    r: &ReadTx<'_>,
    activity: &Activity,
    now_ms: i64,
    job: &InFlight,
    done: &ProofDone,
    kind: ProverKind,
    provider_present: bool,
) -> Result<Option<Judgement>, StoreFault> {
    let Some(record) = r.request(&job.request_id)? else {
        return Ok(None);
    };
    if record.row.request.status.state() != proving_state(job.purpose) {
        return Ok(None);
    }
    if done
        .report
        .as_ref()
        .is_some_and(|report| !bound_to(report, job))
    {
        return Ok(Some(Judgement::Unbound));
    }
    if !grant_check(&done.authorizing_grants, &job.grant_ids) {
        return Ok(Some(Judgement::OutsideFacts));
    }
    let Some(report) = &done.report else {
        return Ok(Some(Judgement::NoReport));
    };
    if !epochs::fence_holds(&job.fence, &epochs::fence(r.counters()?)) {
        return Ok(Some(Judgement::Discard));
    }
    if job.purpose == ProofPurpose::FreshPermit && report.outcome == ProofOutcome::Permit {
        for grant_id in &done.authorizing_grants {
            if !grant_valid_now(r, activity, now_ms, grant_id)? {
                return Ok(Some(Judgement::Discard));
            }
        }
    }
    let agent = r
        .agent(&record.row.request.agent_id)?
        .ok_or(invariant("the agent exists"))?;
    let context = Context {
        kind,
        provider_present,
        agent_paused: agent.paused,
        change: is_change(record.row.action.tool()),
        scoped: record.row.scope.is_some(),
    };
    Ok(Some(Judgement::Accept(accepted(
        job.purpose,
        report.outcome,
        done.ruling.clone(),
        &done.authorizing_grants,
        done.cause.map(stored_cause),
        context,
    ))))
}

/// The validity predicate (5.5) of one grant now, from the store and this run's activity.
fn grant_valid_now(
    r: &ReadTx<'_>,
    activity: &Activity,
    now_ms: i64,
    grant_id: &Id,
) -> Result<bool, StoreFault> {
    let Some(grant) = r.grant(grant_id)? else {
        return Ok(false);
    };
    let origin_open = r
        .request(&grant.origin.request_id)?
        .is_some_and(|o| !o.row.request.status.state().is_terminal());
    Ok(grant_is_valid(
        &grant,
        activity.sessions.get(&grant.agent_id),
        origin_open,
        now_ms,
    ))
}

/// Starts a proof of `purpose` for the request in this transaction: with a prover, the facts
/// are read here, the watchdog is scheduled (`deadline + 1`) with `reproofs`, and the job is
/// handed out after commit; with none the request goes `undecided` with no report, cause
/// `no_prover` (D14).
pub(crate) fn start_proof(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    purpose: ProofPurpose,
    prover: Option<ProverKind>,
    reproofs: Reproofs,
) -> Result<(), StoreFault> {
    if prover.is_none() {
        return undecide(tx, cx, request_id, UndecidedCause::NoProver);
    }
    let record = record_of(tx, request_id)?;
    if record.row.request.status.state() != proving_state(purpose) {
        return Err(invariant("a proof runs in its proving state"));
    }
    let record = RequestRecord { reproofs, ..record };
    let job = facts::proof_job(&tx.read(), cx.activity, cx.now_ms, &record, purpose)?;
    let deadline = i64::try_from(job.deadline_ms).map_err(|_| invariant("a deadline fits"))?;
    tx.apply(Mutation::RequestScheduled {
        request_id: request_id.clone(),
        due_at_ms: Some(deadline + 1),
        reproofs,
    })?;
    cx.after.jobs.push(Job::Prove(job));
    Ok(())
}

/// A proof of `purpose` was discarded, or a dispatch voided: prove again while the purpose's
/// counter allows; otherwise `undecided` with `reproofs_spent` (classification) or `stale`
/// (fresh permit) (D16).
fn prove_again(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    purpose: ProofPurpose,
    prover: Option<ProverKind>,
) -> Result<(), StoreFault> {
    let reproofs = record_of(tx, request_id)?.reproofs;
    match reproof(reproofs, purpose) {
        Some(reproofs) => start_proof(tx, cx, request_id, purpose, prover, reproofs),
        None => match purpose {
            ProofPurpose::Classification => {
                undecide(tx, cx, request_id, UndecidedCause::ReproofsSpent)
            }
            ProofPurpose::FreshPermit => advance(tx, cx, request_id, Event::Exhausted, None, None),
        },
    }
}

/// Why the arguments of a request name nothing the broker can act on, or `None` when the world
/// checks pass (5.7): the account is connected, every calendar exists and is selected, a window
/// starts before it ends. Zones are CD-W09's.
fn invalid_arguments(
    r: &ReadTx<'_>,
    action: &fabric_desktop_contracts::agent::Action,
) -> Result<Option<&'static str>, StoreFault> {
    use fabric_desktop_contracts::agent::Action;
    if matches!(action, Action::RequestResult(_)) {
        return Ok(Some(
            "No result is held for this request; this build keeps no read results.",
        ));
    }
    if let Some((account_id, calendar_ids)) = calendars_of(action) {
        let Some(account) = r.account(&account_id)? else {
            return Ok(Some("The account is not known."));
        };
        if account.state != fabric_desktop_contracts::owner::AccountState::Connected {
            return Ok(Some("The account is not connected."));
        }
        for calendar_id in &calendar_ids {
            match account
                .calendars
                .iter()
                .find(|c| &c.calendar_id == calendar_id)
            {
                None => return Ok(Some("A calendar is not known in the account.")),
                Some(calendar) if !calendar.selected => {
                    return Ok(Some("A calendar is not selected for agents."));
                }
                Some(_) => {}
            }
        }
    }
    let window = match action {
        Action::CalendarAvailability(a) => Some(&a.window),
        Action::CalendarRead(a) => Some(&a.window),
        _ => None,
    };
    if let Some(window) = window {
        match (window.start.unix_millis(), window.end.unix_millis()) {
            (Some(start), Some(end)) if start < end => {}
            _ => return Ok(Some("The window must start before it ends.")),
        }
    }
    Ok(None)
}

/// A received (or, after a restart, validating) request: validated, then classified (5.7).
fn classify(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    prover: Option<ProverKind>,
) -> Result<(), StoreFault> {
    let record = record_of(tx, request_id)?;
    if record.row.request.status.state() == RequestState::Received {
        advance(tx, cx, request_id, Event::Advance, None, None)?;
    }
    match invalid_arguments(&tx.read(), &record.row.action)? {
        Some(reason) => {
            let failure = LifecycleStatus::TerminalFailure {
                failure: Failure {
                    code: FailureCode::InvalidArguments,
                    explanation: Explanation::new(reason)
                        .map_err(|_| invariant("a reason is an explanation"))?,
                },
            };
            advance(
                tx,
                cx,
                request_id,
                Event::Checked { valid: false },
                Some(failure),
                None,
            )
        }
        None => {
            advance(
                tx,
                cx,
                request_id,
                Event::Checked { valid: true },
                None,
                None,
            )?;
            start_proof(
                tx,
                cx,
                request_id,
                ProofPurpose::Classification,
                prover,
                record.reproofs,
            )
        }
    }
}

/// What the dispatch check found (5.6, PLAN 8 step 7).
pub(in crate::service) enum Handoff {
    /// Every check holds: consume and hand off.
    Ready(AuthorizationRecord),
    /// A check failed: void with the reason.
    Void(AuthorizationRecord, VoidReason),
    /// No issued authorization: prove again.
    Missing,
}

/// The dispatch check of a queued request: authorization issued, of this run, unexpired, the
/// fence unchanged, every grant it rests on valid, the agent not paused.
pub(in crate::service) fn handoff(
    r: &ReadTx<'_>,
    activity: &Activity,
    now_ms: i64,
    run_id: i64,
    request: &RequestRecord,
) -> Result<Handoff, StoreFault> {
    let Some(authorization) = r.latest_authorization_of(&request.row.request.request_id)? else {
        return Ok(Handoff::Missing);
    };
    if authorization.state != AuthorizationState::Issued {
        return Ok(Handoff::Missing);
    }
    let void = |reason| -> Result<Handoff, StoreFault> {
        Ok(Handoff::Void(authorization.clone(), reason))
    };
    if authorization.run_id != run_id {
        return void(VoidReason::BrokerRestart);
    }
    let expires = authorization.authorization.expires_at.unix_millis();
    if expires.is_none_or(|expires| now_ms >= expires) {
        return void(VoidReason::Expired);
    }
    if !epochs::fence_holds(&authorization.fence, &epochs::fence(r.counters()?)) {
        return void(VoidReason::FenceChanged);
    }
    for grant_id in &authorization.grant_ids {
        if !grant_valid_now(r, activity, now_ms, grant_id)? {
            return void(VoidReason::GrantInvalid);
        }
    }
    let paused = r
        .agent(&request.row.request.agent_id)?
        .is_none_or(|a| a.paused);
    if paused {
        return void(VoidReason::AgentPaused);
    }
    Ok(Handoff::Ready(authorization))
}

impl Core {
    /// One step of a due request (5.7), in one commit.
    pub(in crate::service) fn step(&mut self, record: RequestRecord) -> Result<(), StoreFault> {
        let request_id = record.row.request.request_id.clone();
        let state = record.row.request.status.state();
        let prover = self.prover_kind();
        let now = self.now;
        match state {
            RequestState::Received | RequestState::Validating => {
                let meta = self.meta("request.advance", Actor::broker(), Some(&request_id), None);
                self.commit(meta, |tx, cx| classify(tx, cx, &request_id, prover))
            }
            RequestState::ProvingClassification | RequestState::ProvingFreshPermit => {
                let purpose = match state {
                    RequestState::ProvingClassification => ProofPurpose::Classification,
                    _ => ProofPurpose::FreshPermit,
                };
                let running = self
                    .in_flight
                    .iter()
                    .find(|(_, job)| job.request_id == request_id)
                    .map(|(job_id, job)| (*job_id, job.deadline_ms));
                match running {
                    Some((job_id, deadline)) if now > deadline => {
                        // The watchdog (D16): a late result is then ignored by its job id.
                        self.in_flight.remove(&job_id);
                        let meta =
                            self.meta("request.proof", Actor::broker(), Some(&request_id), None);
                        self.commit(meta, |tx, cx| {
                            undecide(tx, cx, &request_id, UndecidedCause::DeadlinePassed)
                        })
                    }
                    Some((_, deadline)) => {
                        let meta =
                            self.meta("request.advance", Actor::broker(), Some(&request_id), None);
                        self.commit(meta, |tx, _| {
                            tx.apply(Mutation::RequestScheduled {
                                request_id: request_id.clone(),
                                due_at_ms: Some(deadline + 1),
                                reproofs: record.reproofs,
                            })
                        })
                    }
                    None => {
                        let meta =
                            self.meta("request.advance", Actor::broker(), Some(&request_id), None);
                        self.commit(meta, |tx, cx| {
                            start_proof(tx, cx, &request_id, purpose, prover, record.reproofs)
                        })
                    }
                }
            }
            RequestState::Allowed | RequestState::Approved | RequestState::RetryWait => {
                let meta = self.meta("request.advance", Actor::broker(), Some(&request_id), None);
                self.commit(meta, |tx, cx| {
                    advance(tx, cx, &request_id, Event::Prove, None, None)?;
                    start_proof(
                        tx,
                        cx,
                        &request_id,
                        ProofPurpose::FreshPermit,
                        prover,
                        record.reproofs,
                    )
                })
            }
            RequestState::PendingApproval => {
                let meta = self.meta("request.advance", Actor::broker(), Some(&request_id), None);
                let expires = record.card_expires_at_ms;
                self.commit(meta, |tx, cx| match expires {
                    Some(expires) if now < expires => tx.apply(Mutation::RequestScheduled {
                        request_id: request_id.clone(),
                        due_at_ms: Some(expires),
                        reproofs: record.reproofs,
                    }),
                    _ => advance(tx, cx, &request_id, Event::CardExpired, None, None),
                })
            }
            RequestState::Queued => self.dispatch(&record),
            _ => {
                // Nothing in W04 is due in any other state.
                let meta = self.meta("request.advance", Actor::broker(), Some(&request_id), None);
                self.commit(meta, |tx, _| {
                    tx.apply(Mutation::RequestScheduled {
                        request_id: request_id.clone(),
                        due_at_ms: None,
                        reproofs: record.reproofs,
                    })
                })
            }
        }
    }

    /// The handoff (PLAN 8 step 7) in one commit (`request.dispatch`): the authorization is
    /// consumed and the dispatch intent written, and only after commit does `Job::Dispatch`
    /// leave. A failed check voids the authorization and proves again (5.7).
    fn dispatch(&mut self, record: &RequestRecord) -> Result<(), StoreFault> {
        let request_id = record.row.request.request_id.clone();
        let now = self.now;
        let run_id = self.run_id;
        let prover = self.prover_kind();
        let check = self
            .store
            .read(|r| handoff(r, &self.activity, now, run_id, record))?;
        let meta = self.meta("request.dispatch", Actor::broker(), Some(&request_id), None);
        self.commit(meta, |tx, cx| match check {
            Handoff::Ready(authorization) => {
                let authorization_id = authorization.authorization.authorization_id.clone();
                tx.apply(Mutation::AuthorizationConsumed {
                    authorization_id: authorization_id.clone(),
                    at_ms: cx.now_ms,
                })?;
                let effect = tx
                    .read()
                    .effect_of(&request_id)?
                    .ok_or(invariant("a queued request has its effect"))?;
                let number = effect.attempts_started + 1;
                tx.apply(Mutation::DispatchIntent {
                    attempt: AttemptRow {
                        effect_id: effect.effect_id.clone(),
                        authorization_id: authorization_id.clone(),
                        attempt: Attempt {
                            number: PositiveCount::of(u64::from(number)),
                            proof_id: authorization.authorization.proof_id.clone(),
                            started_at: timestamp(cx.now_ms),
                            finished_at: None,
                            outcome: None,
                        },
                    },
                })?;
                advance(tx, cx, &request_id, Event::Dispatch, None, None)?;
                let consumed = tx
                    .read()
                    .authorization(&authorization_id)?
                    .ok_or(invariant("a consumed authorization stays"))?;
                cx.after.jobs.push(Job::Dispatch(DispatchOrder {
                    request_id: request_id.clone(),
                    effect_id: effect.effect_id,
                    attempt: number,
                    authorization: consumed.authorization,
                    action: record.row.action.clone(),
                    account_id: effect.account_id,
                    calendar_id: effect.calendar_id,
                    provider_event_id: effect.provider_event_id,
                    deadline_ms: u64::try_from(cx.now_ms).unwrap_or(0) + PROVIDER_CALL_MS,
                }));
                Ok(())
            }
            Handoff::Void(authorization, reason) => {
                tx.apply(Mutation::AuthorizationVoided {
                    authorization_id: authorization.authorization.authorization_id,
                    at_ms: cx.now_ms,
                    reason,
                })?;
                advance(tx, cx, &request_id, Event::Prove, None, None)?;
                prove_again(tx, cx, &request_id, ProofPurpose::FreshPermit, prover)
            }
            Handoff::Missing => {
                advance(tx, cx, &request_id, Event::Prove, None, None)?;
                prove_again(tx, cx, &request_id, ProofPurpose::FreshPermit, prover)
            }
        })
    }

    /// A prover's result (5.7). A result for an unknown or expired job, or for a request no
    /// longer in the job's proving state, is ignored and not recorded; every other is recorded
    /// (when it has a report) in the commit that handles it.
    pub(crate) fn proof_done(&mut self, done: ProofDone) -> Result<(), StoreFault> {
        let Some(job) = self.in_flight.get(&done.job_id).cloned() else {
            return Ok(());
        };
        if job.request_id != done.request_id || job.purpose != done.purpose {
            return Ok(());
        }
        let Some(kind) = self.prover_kind() else {
            return Ok(());
        };
        self.in_flight.remove(&done.job_id);
        let now = self.now;
        let provider_present = self.attachments.provider_present;
        let card_ms = i64::try_from(self.settings.limits.approval_expiry_seconds.get())
            .map_err(|_| invariant("the card lifetime fits"))?
            * 1_000;
        let judgement = self
            .store
            .read(|r| judge(r, &self.activity, now, &job, &done, kind, provider_present))?;
        let Some(judgement) = judgement else {
            return Ok(());
        };
        let operation_type = if judgement == Judgement::Accept(Accepted::Authorize) {
            "request.authorize"
        } else {
            "request.proof"
        };
        let causation = done.report.as_ref().map(|r| r.proof_id.as_str().to_owned());
        let meta = self.meta(
            operation_type,
            Actor::broker(),
            Some(&job.request_id),
            causation,
        );
        self.commit(meta, |tx, cx| {
            settle(tx, cx, &job, done, kind, judgement, card_ms)
        })
    }

    /// A provider's result (5.7): one commit (`request.attempt`) that finishes the attempt and
    /// moves the request. A result for another attempt than the open one is ignored.
    pub(crate) fn attempt_done(&mut self, done: AttemptDone) -> Result<(), StoreFault> {
        let policy = self.settings.retry;
        let open = self.store.read(|r| {
            let Some(record) = r.request(&done.request_id)? else {
                return Ok(None);
            };
            if record.row.request.status.state() != RequestState::Dispatching {
                return Ok(None);
            }
            let Some(effect) = r.effect_of(&done.request_id)? else {
                return Ok(None);
            };
            if effect.effect_id != done.effect_id || effect.attempts_started != done.attempt {
                return Ok(None);
            }
            let unfinished = r
                .attempts(&effect.effect_id)?
                .last()
                .is_some_and(|a| a.attempt.outcome.is_none());
            Ok(unfinished.then_some(effect))
        })?;
        let Some(effect) = open else {
            return Ok(());
        };
        let meta = self.meta(
            "request.attempt",
            Actor::broker(),
            Some(&done.request_id),
            None,
        );
        self.commit(meta, |tx, cx| {
            finish_attempt(tx, cx, &effect, done, &policy)
        })
    }
}

/// Applies a judgement in the commit that handles the result (5.7, D36).
fn settle(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    job: &InFlight,
    done: ProofDone,
    kind: ProverKind,
    judgement: Judgement,
    card_ms: i64,
) -> Result<(), StoreFault> {
    let request_id = &job.request_id;
    let record_proof =
        |tx: &mut WriteTx<'_>, now_ms: i64, reason: Option<String>| match &done.report {
            Some(report) => tx.apply(Mutation::ProofRecorded {
                proof: ProofRecord {
                    report: report.clone(),
                    prover: kind,
                    ruling: done.ruling.clone(),
                    authorizing_grants: done.authorizing_grants.clone(),
                    undecided_reason: reason,
                    fence: job.fence.clone(),
                    check_theory_text: done.check_theory_text.clone(),
                },
                at_ms: now_ms,
            }),
            None => Ok(()),
        };
    match judgement {
        Judgement::OutsideFacts => {
            record_proof(tx, cx.now_ms, Some(OUTSIDE_THE_FACTS.to_owned()))?;
            undecide(tx, cx, request_id, UndecidedCause::ReportUnusable)
        }
        Judgement::NoReport => {
            // The prover states why no check ran; only an Isabelle prover's word counts (D13).
            let cause = if kind == ProverKind::Isabelle {
                done.cause.map_or(UndecidedCause::NotStated, stored_cause)
            } else {
                UndecidedCause::NoProver
            };
            undecide(tx, cx, request_id, cause)
        }
        Judgement::Unbound => undecide(tx, cx, request_id, UndecidedCause::ReportUnusable),
        Judgement::Discard => {
            record_proof(tx, cx.now_ms, done.undecided_reason.clone())?;
            prove_again(tx, cx, request_id, job.purpose, Some(kind))
        }
        Judgement::Accept(accepted) => {
            record_proof(tx, cx.now_ms, done.undecided_reason.clone())?;
            if job.purpose == ProofPurpose::FreshPermit {
                set_relied(tx, request_id, done.authorizing_grants.clone())?;
            }
            match accepted {
                Accepted::Allow => {
                    set_relied(tx, request_id, done.authorizing_grants.clone())?;
                    let now = cx.now_ms;
                    advance(tx, cx, request_id, Event::Allow, None, Some(now))
                }
                Accepted::Authorize => authorize(tx, cx, job, &done),
                Accepted::Ask(ruling) => open_card(tx, cx, request_id, ruling, card_ms),
                Accepted::Block(ruling) => advance(
                    tx,
                    cx,
                    request_id,
                    Event::Block,
                    Some(LifecycleStatus::Blocked { denial: ruling }),
                    None,
                ),
                Accepted::Undecided(cause) => undecide(tx, cx, request_id, cause),
            }
        }
    }
}

/// `proving_fresh_permit -> queued`: the authorization (60 s; `grant_id` the first authorizing
/// grant, the store keeping the list, L7(c)) and, the first time, the effect with a create's
/// provider event id reserved before any call.
fn authorize(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    job: &InFlight,
    done: &ProofDone,
) -> Result<(), StoreFault> {
    let request_id = &job.request_id;
    let record = record_of(tx, request_id)?;
    let report = done
        .report
        .as_ref()
        .ok_or(invariant("an authorizing proof has its report"))?;
    let first = done
        .authorizing_grants
        .first()
        .ok_or(invariant("an authorization rests on a grant"))?;
    let action = &record.row.action;
    let attempt = tx
        .read()
        .latest_authorization_of(request_id)?
        .map_or(1, |latest| latest.attempt + 1);
    let lifetime_ms = i64::try_from(AUTHORIZATION_LIFETIME_SECONDS)
        .map_err(|_| invariant("the lifetime fits"))?
        * 1_000;
    let authorization = Authorization {
        profile: Fixed::new(),
        authorization_id: crate::ids::new_id("authorization"),
        request_id: request_id.clone(),
        agent_id: record.row.request.agent_id.clone(),
        action_digest: action.digest(),
        proof_id: report.proof_id.clone(),
        grant_id: first.clone(),
        issued_at: timestamp(cx.now_ms),
        expires_at: timestamp(cx.now_ms + lifetime_ms),
        lifetime_seconds: LifetimeSeconds,
        consumed_at: None,
    };
    tx.apply(Mutation::AuthorizationIssued {
        record: AuthorizationRecord {
            authorization,
            grant_ids: done.authorizing_grants.clone(),
            attempt,
            run_id: cx.run_id,
            fence: job.fence.clone(),
            state: AuthorizationState::Issued,
        },
    })?;
    if tx.read().effect_of(request_id)?.is_none() {
        let (account_id, calendar_ids) =
            calendars_of(action).ok_or(invariant("a change names its calendar"))?;
        let calendar_id = calendar_ids
            .into_iter()
            .next()
            .ok_or(invariant("a change names its calendar"))?;
        let tool = action.tool();
        tx.apply(Mutation::EffectReserved {
            effect: EffectRow {
                effect_id: crate::ids::new_id("effect"),
                request_id: request_id.clone(),
                tool,
                account_id,
                calendar_id,
                action_digest: action.digest(),
                provider_event_id: (tool
                    == fabric_desktop_contracts::common::ToolName::ProposeCreate)
                    .then(crate::ids::provider_event_id),
                created_at_ms: cx.now_ms,
                attempts_started: 0,
                first_attempt_at_ms: None,
            },
        })?;
    }
    let now = cx.now_ms;
    advance(tx, cx, request_id, Event::Authorize, None, Some(now))
}

/// `-> pending_approval`: a 24 h card with the request's scope and the ruling (5.7). The
/// open-card limit is not checked here (5.9, R18).
fn open_card(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    ruling: Ruling,
    card_ms: i64,
) -> Result<(), StoreFault> {
    let scope = record_of(tx, request_id)?
        .row
        .scope
        .ok_or(invariant("a card shows a scope"))?;
    let expires_ms = cx.now_ms + card_ms;
    let expires_at = timestamp(expires_ms);
    tx.apply(Mutation::CardOpened {
        request_id: request_id.clone(),
        card: ApprovalCard {
            scope,
            proposed_duration: Duration {
                kind: DurationKind::OneRequest,
            },
            insufficiency: ruling,
            expires_at: expires_at.clone(),
        },
        opened_at_ms: cx.now_ms,
    })?;
    advance(
        tx,
        cx,
        request_id,
        Event::Ask,
        Some(LifecycleStatus::PendingApproval {
            approval_expires_at: expires_at,
        }),
        Some(expires_ms),
    )
}

/// Finishes the open attempt and moves the request by the report (5.7).
fn finish_attempt(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    effect: &EffectRow,
    done: AttemptDone,
    policy: &RetryPolicy,
) -> Result<(), StoreFault> {
    let finish = |tx: &mut WriteTx<'_>, now_ms: i64, outcome| {
        tx.apply(Mutation::AttemptFinished {
            effect_id: effect.effect_id.clone(),
            number: effect.attempts_started,
            finished_at_ms: now_ms,
            outcome,
        })
    };
    let request_id = &done.request_id;
    let terminal = |code, explanation: &str| -> Result<LifecycleStatus, StoreFault> {
        Ok(LifecycleStatus::TerminalFailure {
            failure: Failure {
                code,
                explanation: Explanation::new(explanation)
                    .map_err(|_| invariant("a failure has an explanation"))?,
            },
        })
    };
    match done.report {
        AttemptReport::Applied { .. } => {
            finish(tx, cx.now_ms, AttemptOutcome::Applied)?;
            advance(tx, cx, request_id, Event::Applied, None, None)
        }
        AttemptReport::Transient { retry_after_ms } => {
            finish(tx, cx.now_ms, AttemptOutcome::TransientFailure)?;
            let first = effect.first_attempt_at_ms.unwrap_or(cx.now_ms);
            match retry_due(
                policy,
                effect.attempts_started,
                first,
                retry_after_ms,
                cx.now_ms,
            ) {
                Some(due) => advance(tx, cx, request_id, Event::Retry, None, Some(due)),
                None => advance(
                    tx,
                    cx,
                    request_id,
                    Event::Fail,
                    Some(terminal(
                        FailureCode::RetryBudgetExhausted,
                        "Google failed for a passing reason too often; the retry budget is spent.",
                    )?),
                    None,
                ),
            }
        }
        AttemptReport::Rejected { code, explanation } => {
            finish(tx, cx.now_ms, AttemptOutcome::TerminalFailure)?;
            advance(
                tx,
                cx,
                request_id,
                Event::Fail,
                Some(LifecycleStatus::TerminalFailure {
                    failure: Failure { code, explanation },
                }),
                None,
            )
        }
        AttemptReport::NoAnswer => {
            finish(tx, cx.now_ms, AttemptOutcome::OutcomeUnknown)?;
            advance(tx, cx, request_id, Event::Unknown, None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    fn ruling() -> Ruling {
        fabric_desktop_contracts::decode_record(
            br#"{"rule": "test_rule", "explanation": "Because.", "proof_id": "proof-1"}"#,
        )
        .unwrap()
    }

    const ISABELLE: Context = Context {
        kind: ProverKind::Isabelle,
        provider_present: true,
        agent_paused: false,
        change: true,
        scoped: true,
    };

    #[test]
    fn the_grant_check_refuses_any_id_outside_the_facts() {
        let offered = [id("grant-1"), id("grant-2")];
        assert!(grant_check(&[], &offered));
        assert!(grant_check(&[id("grant-2"), id("grant-1")], &offered));
        assert!(!grant_check(&[id("grant-3")], &offered));
        assert!(!grant_check(
            &[id("grant-1"), id("grant-other-agent")],
            &offered
        ));
        assert!(!grant_check(&[id("grant-1")], &[]));
    }

    #[test]
    fn only_an_isabelle_permit_allows_a_classification() {
        let permit = |context| {
            accepted(
                ProofPurpose::Classification,
                ProofOutcome::Permit,
                None,
                &[],
                None,
                context,
            )
        };
        assert_eq!(permit(ISABELLE), Accepted::Allow);
        let synthetic = Context {
            kind: ProverKind::Synthetic,
            ..ISABELLE
        };
        assert_eq!(
            permit(synthetic),
            Accepted::Undecided(UndecidedCause::NoProver)
        );
    }

    #[test]
    fn a_fresh_permit_authorizes_only_when_everything_it_needs_is_there() {
        let grants = [id("grant-1")];
        let permit = |grants: &[Id], context| {
            accepted(
                ProofPurpose::FreshPermit,
                ProofOutcome::Permit,
                None,
                grants,
                None,
                context,
            )
        };
        assert_eq!(permit(&grants, ISABELLE), Accepted::Authorize);
        // L7(d): no authorizing grant.
        assert_eq!(
            permit(&[], ISABELLE),
            Accepted::Undecided(UndecidedCause::NoGrant)
        );
        // D17: no provider; D13: not Isabelle.
        let without_provider = Context {
            provider_present: false,
            ..ISABELLE
        };
        assert_eq!(
            permit(&grants, without_provider),
            Accepted::Undecided(UndecidedCause::NoProvider)
        );
        let synthetic = Context {
            kind: ProverKind::Synthetic,
            ..ISABELLE
        };
        assert_eq!(
            permit(&grants, synthetic),
            Accepted::Undecided(UndecidedCause::NoProver)
        );
        let paused = Context {
            agent_paused: true,
            ..ISABELLE
        };
        assert_eq!(
            permit(&grants, paused),
            Accepted::Undecided(UndecidedCause::AgentPaused)
        );
        // A read is not dispatched in W04 (D33).
        let read = Context {
            change: false,
            ..ISABELLE
        };
        assert_eq!(
            permit(&grants, read),
            Accepted::Undecided(UndecidedCause::ReadTool)
        );
    }

    #[test]
    fn block_and_need_approval_need_a_ruling() {
        let no_ruling = Accepted::Undecided(UndecidedCause::NoRuling);
        for purpose in [ProofPurpose::Classification, ProofPurpose::FreshPermit] {
            let verdict = |outcome, ruling| accepted(purpose, outcome, ruling, &[], None, ISABELLE);
            assert_eq!(
                verdict(ProofOutcome::Block, Some(ruling())),
                Accepted::Block(ruling())
            );
            assert_eq!(verdict(ProofOutcome::Block, None), no_ruling);
            assert_eq!(
                verdict(ProofOutcome::NeedApproval, Some(ruling())),
                Accepted::Ask(ruling())
            );
            assert_eq!(verdict(ProofOutcome::NeedApproval, None), no_ruling);
            assert_eq!(
                verdict(ProofOutcome::Undecided, Some(ruling())),
                Accepted::Undecided(UndecidedCause::CheckNotDecided)
            );
        }
        // A request with no scope cannot be put on a card.
        let unscoped = Context {
            scoped: false,
            ..ISABELLE
        };
        assert_eq!(
            accepted(
                ProofPurpose::Classification,
                ProofOutcome::NeedApproval,
                Some(ruling()),
                &[],
                None,
                unscoped
            ),
            Accepted::Undecided(UndecidedCause::NoScope)
        );
    }

    /// c33. 5.7: every undecided verdict of `accepted` names its cause, one row per case, the overlap
    /// of no grant and no provider included (the report's own defect comes first).
    #[test]
    fn accepted_states_each_cause() {
        use UndecidedCause as C;
        let grants = [id("grant-1")];
        let synthetic = Context {
            kind: ProverKind::Synthetic,
            ..ISABELLE
        };
        let no_provider = Context {
            provider_present: false,
            ..ISABELLE
        };
        let read = Context {
            change: false,
            ..ISABELLE
        };
        let paused = Context {
            agent_paused: true,
            ..ISABELLE
        };
        let unscoped = Context {
            scoped: false,
            ..ISABELLE
        };
        let classification = ProofPurpose::Classification;
        let fresh = ProofPurpose::FreshPermit;
        let deadline = Some(C::DeadlinePassed);
        let permit = |purpose, authorizing: &[Id], context| {
            accepted(
                purpose,
                ProofOutcome::Permit,
                None,
                authorizing,
                None,
                context,
            )
        };
        let ask = |purpose, ruling, context| {
            accepted(
                purpose,
                ProofOutcome::NeedApproval,
                ruling,
                &[],
                None,
                context,
            )
        };
        let undecided = |purpose, stated, context| {
            accepted(purpose, ProofOutcome::Undecided, None, &[], stated, context)
        };
        let block =
            |purpose, context| accepted(purpose, ProofOutcome::Block, None, &[], None, context);
        let rows = [
            (
                "synthetic classification permit",
                permit(classification, &[], synthetic),
                C::NoProver,
            ),
            (
                "synthetic fresh permit",
                permit(fresh, &grants, synthetic),
                C::NoProver,
            ),
            (
                "synthetic undecided with a cause",
                undecided(classification, deadline, synthetic),
                C::NoProver,
            ),
            (
                "synthetic unscoped ask",
                ask(
                    classification,
                    Some(ruling()),
                    Context {
                        scoped: false,
                        ..synthetic
                    },
                ),
                C::NoProver,
            ),
            ("no grant", permit(fresh, &[], ISABELLE), C::NoGrant),
            ("read tool", permit(fresh, &grants, read), C::ReadTool),
            ("paused", permit(fresh, &grants, paused), C::AgentPaused),
            (
                "no provider",
                permit(fresh, &grants, no_provider),
                C::NoProvider,
            ),
            (
                "no grant and no provider",
                permit(fresh, &[], no_provider),
                C::NoGrant,
            ),
            (
                "ask without a ruling",
                ask(classification, None, ISABELLE),
                C::NoRuling,
            ),
            (
                "ask without a scope",
                ask(fresh, Some(ruling()), unscoped),
                C::NoScope,
            ),
            (
                "block without a ruling",
                block(fresh, ISABELLE),
                C::NoRuling,
            ),
            (
                "undecided stating the deadline",
                undecided(classification, deadline, ISABELLE),
                C::DeadlinePassed,
            ),
            (
                "undecided stating the limit",
                undecided(fresh, Some(C::CheckLimitReached), ISABELLE),
                C::CheckLimitReached,
            ),
            (
                "undecided stating nothing",
                undecided(fresh, None, ISABELLE),
                C::CheckNotDecided,
            ),
        ];
        for (case, verdict, cause) in rows {
            assert_eq!(verdict, Accepted::Undecided(cause), "{case}");
        }
    }

    /// Every cause a prover states has its own stored cause.
    #[test]
    fn each_prover_cause_has_its_stored_cause() {
        let table = [
            (ProverCause::Starting, UndecidedCause::ProverStarting),
            (ProverCause::NotRunning, UndecidedCause::ProverNotRunning),
            (ProverCause::DeadlinePassed, UndecidedCause::DeadlinePassed),
            (
                ProverCause::CheckLimitReached,
                UndecidedCause::CheckLimitReached,
            ),
            (
                ProverCause::CheckNotDecided,
                UndecidedCause::CheckNotDecided,
            ),
            (ProverCause::ReportUnusable, UndecidedCause::ReportUnusable),
            (ProverCause::JobRefused, UndecidedCause::JobRefused),
        ];
        for (stated, stored) in table {
            assert_eq!(stored_cause(stated), stored, "{stated:?}");
        }
    }

    #[test]
    fn the_reproof_counters_are_per_purpose_and_stop_at_the_maximum() {
        let mut reproofs = Reproofs::default();
        for n in 1..=MAX_REPROOFS {
            reproofs = reproof(reproofs, ProofPurpose::Classification).unwrap();
            assert_eq!(reproofs.classification, n);
            assert_eq!(reproofs.fresh_permit, 0);
        }
        assert_eq!(reproof(reproofs, ProofPurpose::Classification), None);
        let fresh = reproof(reproofs, ProofPurpose::FreshPermit).unwrap();
        assert_eq!(fresh.fresh_permit, 1);
        assert_eq!(fresh.classification, MAX_REPROOFS);
    }

    const FIRST: i64 = 1_795_190_400_000;

    #[test]
    fn transient_failures_wait_2_4_and_8_s_then_end() {
        let policy = RetryPolicy::PLAN;
        let mut now = FIRST;
        for (attempts, wait) in [(1, 2_000), (2, 4_000), (3, 8_000)] {
            assert_eq!(
                retry_due(&policy, attempts, FIRST, None, now),
                Some(now + wait),
                "attempt {attempts}"
            );
            now += wait;
        }
        // `retries` = `attempts_started - 1`: the fourth transient attempt ends the request.
        assert_eq!(retry_due(&policy, 4, FIRST, None, now), None);
    }

    #[test]
    fn a_provider_hint_longer_than_the_backoff_is_used_within_the_budget() {
        let policy = RetryPolicy::PLAN;
        assert_eq!(
            retry_due(&policy, 1, FIRST, Some(30_000), FIRST),
            Some(FIRST + 30_000)
        );
        assert_eq!(
            retry_due(&policy, 1, FIRST, Some(500), FIRST),
            Some(FIRST + 2_000)
        );
        assert_eq!(
            retry_due(&policy, 1, FIRST, Some(120_000), FIRST),
            Some(FIRST + 120_000)
        );
        // A hint past the budget ends the request at once.
        assert_eq!(retry_due(&policy, 1, FIRST, Some(121_000), FIRST), None);
        // So does a backoff that would wake after the budget.
        assert_eq!(retry_due(&policy, 2, FIRST, None, FIRST + 117_000), None);
    }
}
