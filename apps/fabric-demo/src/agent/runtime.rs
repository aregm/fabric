//! The agent runtime (section 10.1, task T6.1). It takes a turn whenever something new happens:
//! the owner files or changes a request or decides, the other agent sends something, or a read
//! returns. On each turn the planner writes 2–3 alternative plans. All of them are screened in
//! one check (D8), and the first whose actions are all permitted runs, one authorized and
//! tokened action at a time (D2). A denial at either point is fed back for at most 3 re-plans
//! per turn (D5).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use fabric_policy::{Action, Candidate, Verdict};
use fabric_token::{BindingKey, hex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::planner::Planner;
use super::turn::{FreeBusyView, Memory};
use crate::audit::AuditLog;
use crate::calendar::{Busy, CalendarClient};
use crate::client::CallError;
use crate::clock::Clock;
use crate::host::negotiation::NO_NEGOTIATION;
use crate::host::{AgentState, AuthorizeAnswer, Delivery, HostClient, ScreenAnswer};

/// Re-plans per turn after denials (D5).
pub const MAX_REPLANS: usize = 3;
/// Turns in a row that may follow a read before the agent waits for an outside event.
const MAX_FOLLOW_UPS: u32 = 3;
const MINUTE_MS: i64 = 60 * 1_000;

/// Everything outside the agent: its host and its calendar service.
pub trait World: Send + Sync {
    fn state(&self) -> impl Future<Output = Result<AgentState, CallError>> + Send;
    fn deliveries(
        &self,
        after: u64,
        wait_ms: u64,
    ) -> impl Future<Output = Result<Vec<Delivery>, CallError>> + Send;
    fn screen(
        &self,
        actions: &[Action],
    ) -> impl Future<Output = Result<ScreenAnswer, CallError>> + Send;
    fn authorize(
        &self,
        action: &Action,
    ) -> impl Future<Output = Result<AuthorizeAnswer, CallError>> + Send;
    /// Runs `action` at the service that executes it, presenting `token`.
    fn execute(
        &self,
        action: &Action,
        token: &[u8],
    ) -> impl Future<Output = Result<Value, CallError>> + Send;
}

/// The real world: the host and the calendar service over loopback HTTP.
pub struct HttpWorld {
    pub host: HostClient,
    pub calendar: CalendarClient,
    pub key: BindingKey,
    pub clock: Arc<dyn Clock>,
}

impl World for HttpWorld {
    async fn state(&self) -> Result<AgentState, CallError> {
        self.host.state().await
    }

    async fn deliveries(&self, after: u64, wait_ms: u64) -> Result<Vec<Delivery>, CallError> {
        self.host.inbox(after, wait_ms).await
    }

    async fn screen(&self, actions: &[Action]) -> Result<ScreenAnswer, CallError> {
        self.host.screen(actions).await
    }

    async fn authorize(&self, action: &Action) -> Result<AuthorizeAnswer, CallError> {
        self.host.authorize(action).await
    }

    async fn execute(&self, action: &Action, token: &[u8]) -> Result<Value, CallError> {
        let now = self.clock.now_ms();
        match action {
            Action::ReadFreeBusy { .. } => self
                .calendar
                .free_busy(&self.key, token, action, now)
                .await
                .map(|busy| json!({ "busy": busy })),
            Action::AddMeeting { .. } => self
                .calendar
                .add_meeting(&self.key, token, action, now)
                .await
                .map(|()| json!({ "added": true })),
            Action::Handshake { .. } => self.host.handshake(&self.key, token, action, now).await,
            Action::ReadEventDetails { .. } => Err(CallError::Response(
                "nothing executes read_event_details".to_owned(),
            )),
            _ => self.host.send(&self.key, token, action, now).await,
        }
    }
}

/// The fault-injection switches (Q24, section 14). Each fires once, on the next turn.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Faults {
    /// Widens every free/busy read past the request window and adds an event-details read.
    pub over_broad_read: bool,
    /// Pads every proposal to 5 candidates.
    pub too_many_candidates: bool,
    /// Puts an `add_meeting` before the chosen plan's actions.
    pub write_without_consent: bool,
    /// Sends a request again with a token that was already used.
    pub replay_token: bool,
    /// Moves a calendar action by 15 minutes after its token was issued.
    pub token_for_different_slot: bool,
}

impl Faults {
    #[must_use]
    pub const fn any(&self) -> bool {
        self.over_broad_read
            || self.too_many_candidates
            || self.write_without_consent
            || self.replay_token
            || self.token_for_different_slot
    }

    /// Adds `other`'s switches to these.
    pub fn merge(&mut self, other: Self) {
        self.over_broad_read |= other.over_broad_read;
        self.too_many_candidates |= other.too_many_candidates;
        self.write_without_consent |= other.write_without_consent;
        self.replay_token |= other.replay_token;
        self.token_for_different_slot |= other.token_for_different_slot;
    }
}

/// How a turn ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The plan ran, or the planner chose to wait; the next turn waits for an outside event.
    Waiting,
    /// A read returned; the next turn starts at once (D3).
    ReadMore,
    /// Every attempt was denied; the agent waits for an outside event (D5).
    GaveUp,
}

enum PlanResult {
    Done,
    ReadMore,
    Denied,
}

fn reasons(verdict: &Verdict) -> Vec<String> {
    match verdict {
        Verdict::Permit => Vec::new(),
        Verdict::Deny { denials } => denials
            .iter()
            .map(|denial| format!("{}: {}", denial.rule.name(), denial.detail))
            .collect(),
        Verdict::Undecided { reason } => vec![format!("undecided: {reason}")],
    }
}

const fn round_of(action: &Action) -> Option<u32> {
    match action {
        Action::SendPropose { round, .. }
        | Action::SendRespond { round, .. }
        | Action::SendDecide { round, .. } => Some(*round),
        _ => None,
    }
}

/// The same calendar action 15 minutes later, for the `token_for_different_slot` switch.
fn moved(action: &Action) -> Option<Action> {
    match action.clone() {
        Action::ReadFreeBusy { start_ms, end_ms } => Some(Action::ReadFreeBusy {
            start_ms: start_ms + 15 * MINUTE_MS,
            end_ms: end_ms + 15 * MINUTE_MS,
        }),
        Action::AddMeeting {
            negotiation,
            start_ms,
            length_ms,
        } => Some(Action::AddMeeting {
            negotiation,
            start_ms: start_ms + 15 * MINUTE_MS,
            length_ms,
        }),
        _ => None,
    }
}

/// What a turn compares to notice that the owner did something.
fn owner_fingerprint(state: &AgentState) -> String {
    json!({ "request": state.request, "decisions": state.owner_decisions }).to_string()
}

pub struct Agent<W: World> {
    world: W,
    planner: Planner,
    memory: Memory,
    faults: Faults,
    audit: AuditLog,
    component: String,
    turns: u64,
    follow_up: bool,
    follow_ups: u32,
    owner_seen: Option<String>,
}

impl<W: World> Agent<W> {
    #[must_use]
    pub fn new(world: W, planner: Planner, person: &str, audit: AuditLog) -> Self {
        Self {
            world,
            planner,
            memory: Memory::default(),
            faults: Faults::default(),
            audit,
            component: format!("agent:{person}"),
            turns: 0,
            follow_up: false,
            follow_ups: 0,
            owner_seen: None,
        }
    }

    #[must_use]
    pub const fn memory(&self) -> &Memory {
        &self.memory
    }

    #[must_use]
    pub const fn world(&self) -> &W {
        &self.world
    }

    /// Arms fault switches for the next turn.
    pub fn inject(&mut self, faults: Faults) {
        self.faults.merge(faults);
    }

    fn record(&self, mut record: Value) {
        record["component"] = self.component.clone().into();
        record["turn"] = self.turns.into();
        self.audit.record(&record);
    }

    /// Waits up to `wait_ms` for something new, then takes a turn if there is anything to act
    /// on. Returns how the turn ended, or `None` when there was nothing to do.
    pub async fn step(&mut self, wait_ms: u64) -> Result<Option<Outcome>, CallError> {
        let wait = if self.follow_up { 0 } else { wait_ms };
        let deliveries = self
            .world
            .deliveries(self.memory.delivery_cursor, wait)
            .await?;
        self.memory.absorb(&deliveries);
        let state = self.world.state().await?;
        let owner = owner_fingerprint(&state);
        let owner_acted = self.owner_seen.as_ref() != Some(&owner);
        self.owner_seen = Some(owner);
        if state.request.is_none()
            || !(self.follow_up || owner_acted || !deliveries.is_empty() || self.faults.any())
        {
            return Ok(None);
        }
        let outcome = self.turn().await?;
        self.follow_up = outcome == Outcome::ReadMore && self.follow_ups < MAX_FOLLOW_UPS;
        self.follow_ups = if self.follow_up {
            self.follow_ups + 1
        } else {
            0
        };
        Ok(Some(outcome))
    }

    /// Steps forever, waiting up to `wait_ms` for each outside event.
    pub async fn run(mut self, wait_ms: u64) {
        loop {
            if let Err(error) = self.step(wait_ms).await {
                self.record(json!({ "kind": "error", "detail": error.to_string() }));
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    async fn turn(&mut self) -> Result<Outcome, CallError> {
        self.turns += 1;
        let mut faults = std::mem::take(&mut self.faults);
        for attempt in 0..=MAX_REPLANS {
            let state = self.world.state().await?;
            let input = self.memory.turn_input(&state, &state.owner_decisions);
            let planned = match self.planner.plan(&input).await {
                Ok(planned) => planned,
                Err(error) => {
                    self.memory
                        .note_denial(json!({ "attempt": attempt, "planner": error.to_string() }));
                    self.record(json!({
                        "kind": "plan_failed",
                        "attempt": attempt,
                        "input": input,
                        "detail": error.to_string(),
                    }));
                    continue;
                }
            };
            let negotiation = state
                .negotiation
                .id
                .clone()
                .unwrap_or_else(|| NO_NEGOTIATION.to_owned());
            let mut options: Vec<Result<Vec<Action>, String>> = planned
                .plans
                .plans
                .iter()
                .map(|plan| {
                    plan.actions
                        .iter()
                        .map(|action| action.to_action(&negotiation))
                        .collect()
                })
                .collect();
            if attempt == 0 {
                distort(&mut faults, &mut options, &state);
            }

            let batch: Vec<Action> = options.iter().flatten().flatten().cloned().collect();
            let screened = if batch.is_empty() {
                None
            } else {
                Some(self.world.screen(&batch).await?)
            };
            let verdicts = screened
                .as_ref()
                .map_or(&[][..], |answer| answer.verdicts.as_slice());

            let mut position = 0;
            let mut chosen = None;
            let mut denials = Vec::new();
            for (index, option) in options.iter().enumerate() {
                let actions = match option {
                    Ok(actions) => actions,
                    Err(error) => {
                        denials.push(json!({ "plan": index, "unreadable": error }));
                        continue;
                    }
                };
                let these = &verdicts[position..position + actions.len()];
                position += actions.len();
                for (action, verdict) in actions.iter().zip(these) {
                    if !verdict.is_permit() {
                        denials.push(json!({
                            "plan": index,
                            "action": action.kind(),
                            "reasons": reasons(verdict),
                        }));
                    }
                }
                if chosen.is_none() && these.iter().all(Verdict::is_permit) {
                    chosen = Some(index);
                }
            }
            self.record(json!({
                "kind": "plans",
                "attempt": attempt,
                "input": input,
                "planner": {
                    "backend": self.planner.name(),
                    "model": planned.model,
                    "input_tokens": planned.input_tokens,
                    "output_tokens": planned.output_tokens,
                    "seconds": planned.seconds,
                },
                "plans": planned.plans,
                "screen": screened.as_ref().map(|answer| json!({
                    "theory": answer.theory,
                    "proof_id": answer.proof_id,
                    "seconds": answer.seconds,
                    "verdicts": answer.verdicts,
                })),
                "chosen": chosen,
                "denials": denials,
            }));

            let Some(index) = chosen else {
                for denial in denials {
                    self.memory.note_denial(
                        json!({ "attempt": attempt, "at": "screening", "denial": denial }),
                    );
                }
                continue;
            };
            let Ok(mut actions) = options.swap_remove(index) else {
                continue;
            };
            if actions.is_empty() {
                return Ok(Outcome::Waiting);
            }
            if std::mem::take(&mut faults.write_without_consent) {
                actions.insert(0, unconsented_meeting(&state, &negotiation));
            }
            match self.execute_plan(attempt, actions, &mut faults).await? {
                PlanResult::Done => return Ok(Outcome::Waiting),
                PlanResult::ReadMore => return Ok(Outcome::ReadMore),
                PlanResult::Denied => {}
            }
        }
        self.record(json!({ "kind": "gave_up", "attempts": MAX_REPLANS + 1 }));
        Ok(Outcome::GaveUp)
    }

    /// Authorizes, then runs, each action in order.
    async fn execute_plan(
        &mut self,
        attempt: usize,
        actions: Vec<Action>,
        faults: &mut Faults,
    ) -> Result<PlanResult, CallError> {
        for action in actions {
            let Some(mut token) = self.token_for(attempt, &action).await? else {
                return Ok(PlanResult::Denied);
            };
            if faults.token_for_different_slot
                && let Some(other) = moved(&action)
            {
                faults.token_for_different_slot = false;
                let refused = self.world.execute(&other, &token).await;
                self.record(json!({
                    "kind": "fault",
                    "fault": "token_for_different_slot",
                    "action": other,
                    "outcome": refused.as_ref().err().and_then(CallError::code),
                }));
                let Some(fresh) = self.token_for(attempt, &action).await? else {
                    return Ok(PlanResult::Denied);
                };
                token = fresh;
            }
            match self.world.execute(&action, &token).await {
                Ok(result) => {
                    self.memory.note_action(json!({
                        "kind": action.kind(),
                        "round": round_of(&action),
                        "action": action,
                        "result": result,
                    }));
                    self.record(json!({
                        "kind": "executed",
                        "attempt": attempt,
                        "action": action,
                        "result": result,
                    }));
                    if std::mem::take(&mut faults.replay_token) {
                        let replayed = self.world.execute(&action, &token).await;
                        self.record(json!({
                            "kind": "fault",
                            "fault": "replay_token",
                            "outcome": replayed.as_ref().err().and_then(CallError::code),
                        }));
                    }
                    if let Action::ReadFreeBusy { start_ms, end_ms } = action {
                        let busy: Vec<Busy> =
                            serde_json::from_value(result["busy"].clone()).unwrap_or_default();
                        self.memory.free_busy = Some(FreeBusyView::new(start_ms, end_ms, &busy));
                        return Ok(PlanResult::ReadMore);
                    }
                }
                Err(error) => {
                    self.memory.note_denial(json!({
                        "attempt": attempt,
                        "at": "execution",
                        "action": action.kind(),
                        "reasons": [error.to_string()],
                    }));
                    self.record(json!({
                        "kind": "refused",
                        "attempt": attempt,
                        "action": action,
                        "detail": error.to_string(),
                    }));
                    return Ok(PlanResult::Denied);
                }
            }
        }
        Ok(PlanResult::Done)
    }

    /// The execution-time check. Returns the token, or `None` after noting the denial.
    async fn token_for(
        &mut self,
        attempt: usize,
        action: &Action,
    ) -> Result<Option<Vec<u8>>, CallError> {
        let answer = self.world.authorize(action).await?;
        let token = answer
            .token
            .as_deref()
            .filter(|_| answer.verdict.is_permit())
            .and_then(|token| hex::decode(token).ok());
        self.record(json!({
            "kind": "authorize",
            "attempt": attempt,
            "action": action,
            "theory": answer.theory,
            "proof_id": answer.proof_id,
            "seconds": answer.seconds,
            "verdict": answer.verdict,
        }));
        if token.is_none() {
            self.memory.note_denial(json!({
                "attempt": attempt,
                "at": "authorize",
                "action": action.kind(),
                "reasons": reasons(&answer.verdict),
            }));
        }
        Ok(token)
    }
}

/// The screening-stage switches, applied to the first attempt's plans.
fn distort(
    faults: &mut Faults,
    options: &mut Vec<Result<Vec<Action>, String>>,
    state: &AgentState,
) {
    if std::mem::take(&mut faults.over_broad_read)
        && let Some(owner) = &state.request
    {
        let window = &owner.request;
        for actions in options.iter_mut().flatten() {
            for action in actions.iter_mut() {
                if let Action::ReadFreeBusy { end_ms, .. } = action {
                    *end_ms = window.window_end_ms + 30 * 24 * 60 * MINUTE_MS;
                }
            }
        }
        options.insert(
            0,
            Ok(vec![Action::ReadEventDetails {
                start_ms: window.window_start_ms,
                end_ms: window.window_end_ms,
            }]),
        );
    }
    if std::mem::take(&mut faults.too_many_candidates) {
        for actions in options.iter_mut().flatten() {
            for action in actions.iter_mut() {
                if let Action::SendPropose {
                    round, candidates, ..
                } = action
                    && let Some(last) = candidates.last().cloned()
                {
                    while candidates.len() < 5 {
                        let offset = i64::try_from(candidates.len()).unwrap_or(5) * 60 * MINUTE_MS;
                        candidates.push(Candidate {
                            id: super::plan::candidate_id(*round, candidates.len()),
                            start_ms: last.start_ms + offset,
                            length_ms: last.length_ms,
                        });
                    }
                }
            }
        }
    }
}

/// An `add_meeting` that no owner agreed to, for the `write_without_consent` switch.
fn unconsented_meeting(state: &AgentState, negotiation: &str) -> Action {
    let first = state.negotiation.candidates.first();
    let (start_ms, length_ms) = match (first, &state.request) {
        (Some(candidate), _) => (candidate.start_ms, candidate.length_ms),
        (None, Some(owner)) => (
            owner.request.window_start_ms + 24 * 60 * MINUTE_MS,
            owner.request.duration_ms,
        ),
        (None, None) => (0, 30 * MINUTE_MS),
    };
    Action::AddMeeting {
        negotiation: negotiation.to_owned(),
        start_ms,
        length_ms,
    }
}
