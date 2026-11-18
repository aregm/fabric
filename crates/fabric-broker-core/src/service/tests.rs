//! Unit tests of the service over a real store (design 9.1, B): the core on a test-key store in a
//! temporary directory, driven without the owner channel. The owner's decision is written here
//! the way `decide_request` writes it, with the service's own helpers.

use fabric_desktop_contracts::agent::AgentRequest;
use fabric_desktop_contracts::owner::{
    Account, Approval, Approved, AttemptOutcome, Duration, DurationKind, GrantEndReason,
    LifecycleStatus, ProofPurpose, ProofReport, Ruling,
};
use fabric_desktop_contracts::scalar::{Id, Label};
use fabric_desktop_contracts::{Decision, Entry, Frame, RequestState, approval_digest};
use fabric_storage::{
    Actor, AuthorizationState, Mutation, ProverKind, StoreFault, TxError, UndecidedCause,
    VoidReason,
};
use serde_json::{Value, json};

use super::core::testing::{self, T0, TempDir};
use super::core::{Core, Outbox};
use super::grants::{create_grant, end_grant};
use super::pipeline::{Handoff, advance, handoff, undecide};
use super::reducer::Event;
use super::relied::set_relied;
use super::requests::{decided, move_request, move_request_with, record_of};
use super::{OwnerRefusal, timestamp};
use crate::broker::{Job, MAX_REPROOFS, PROOF_WATCHDOG_MS, Seed, SeedAgent};
use crate::ports::{
    AgentInput, AttemptDone, AttemptReport, ConnectionId, DispatchOrder, ProofDone, ProofJob,
};

const AGENT: &str = "agent-a";
const ACCOUNT: &str = "acct-1";
const CALENDAR: &str = "cal-1";

fn id(text: &str) -> Id {
    Id::new(text).unwrap()
}

fn record<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    fabric_desktop_contracts::decode_record(serde_json::to_string(value).unwrap().as_bytes())
        .unwrap()
}

fn account(account_id: &str, state: &str, calendars: &[(&str, bool)]) -> Account {
    let calendars: Vec<Value> = calendars
        .iter()
        .map(|(calendar_id, selected)| {
            json!({"calendar_id": calendar_id, "name": "Calendar", "access": "owner",
                   "selected": selected, "agent_drafts": false})
        })
        .collect();
    record(
        &json!({"account_id": account_id, "revision": 1, "state": state,
                   "email": "owner@example.com", "calendars": calendars}),
    )
}

/// A broker run in a test: the core, its directory and the raw clock.
struct Run {
    dir: TempDir,
    core: Option<Core>,
    now: u64,
    calls: u64,
}

impl Run {
    fn new() -> Run {
        let dir = TempDir::new();
        let core = testing::open(&dir, testing::isabelle(), T0);
        Run {
            dir,
            core: Some(core),
            now: T0,
            calls: 0,
        }
    }

    fn core(&mut self) -> &mut Core {
        self.core.as_mut().expect("the core is open")
    }

    /// Drops the core without quit, then opens it again (a kill and a restart).
    fn restart(&mut self) -> Outbox {
        self.core = None;
        let core = testing::open(&self.dir, testing::isabelle(), self.now);
        self.core = Some(core);
        Outbox::default()
    }

    /// One step as the broker runs it: the clock, due work, `f`, due work again.
    fn step(&mut self, f: impl FnOnce(&mut Core)) -> Outbox {
        let now = self.now;
        let core = self.core();
        let _ = core.now_ms(now);
        core.pump().unwrap();
        f(core);
        core.pump().unwrap();
        core.take_outbox()
    }

    /// Stops the core and checks the ledger chain and the replay of everything it wrote.
    fn verify(&mut self) {
        self.core = None;
        let store = (testing::opener(&self.dir))(i64::try_from(self.now).unwrap()).unwrap();
        store.verify_chain().unwrap();
        store.verify_replay().unwrap();
        store.close().unwrap();
    }

    fn tick(&mut self) -> Outbox {
        self.step(|_| {})
    }

    fn seed(&mut self, agents: &[&str], accounts: Vec<Account>) {
        let seed = Seed {
            agents: agents
                .iter()
                .map(|a| SeedAgent {
                    agent_id: id(a),
                    registered_name: Label::new(*a).unwrap(),
                    destination: fabric_desktop_contracts::owner::Destination::Local {
                        endpoint: Label::new("in-process").unwrap(),
                    },
                })
                .collect(),
            accounts,
        };
        let _ = self.step(|core| core.seed(&seed).unwrap());
    }

    fn setup(&mut self) {
        self.seed(
            &[AGENT],
            vec![account(ACCOUNT, "connected", &[(CALENDAR, true)])],
        );
        let _ = self.connect(1, AGENT);
    }

    fn connect(&mut self, connection: u64, agent: &str) -> Outbox {
        let input = AgentInput::Connect {
            connection: ConnectionId(connection),
            agent_id: id(agent),
        };
        self.step(|core| core.agent(input).unwrap())
    }

    fn call(&mut self, connection: u64, call_id: &str, method: &str, params: Value) -> Outbox {
        let line = serde_json::to_vec(&json!({"protocol": "fabric-desktop/1", "id": call_id,
                                               "method": method, "params": params}))
        .unwrap();
        let Ok(Frame::AgentRequest(request)) =
            fabric_desktop_contracts::decode(Entry::AgentRequest, &line)
        else {
            panic!("not an agent request");
        };
        let request: AgentRequest = request;
        let input = AgentInput::Call {
            connection: ConnectionId(connection),
            request,
        };
        self.step(|core| core.agent(input).unwrap())
    }

    /// A `propose_create` on `account`/`calendar`; returns the request id and the output.
    fn propose_on(&mut self, account: &str, calendar: &str) -> (Id, Outbox) {
        self.calls += 1;
        let call_id = format!("call-{}", self.calls);
        let out = self.call(
            1,
            &call_id,
            "fabric_calendar_propose_create",
            json!({"account_id": account, "calendar_id": calendar,
                   "event": {"title": "Plan",
                             "start": {"date_time": "2026-11-27T10:00:00+01:00", "zone": "Europe/Berlin"},
                             "end": {"date_time": "2026-11-27T11:00:00+01:00", "zone": "Europe/Berlin"},
                             "description": null, "location": null},
                   "task": "Plan the week."}),
        );
        let reply = out.agent.last().expect("a reply").frame.to_value();
        let request_id = reply["result"]["request"]["request_id"]
            .as_str()
            .unwrap_or_else(|| panic!("no request: {reply}"));
        (id(request_id), out)
    }

    fn propose(&mut self) -> (Id, Outbox) {
        self.propose_on(ACCOUNT, CALENDAR)
    }

    fn prove(&mut self, done: ProofDone) -> Outbox {
        self.step(|core| core.proof_done(done).unwrap())
    }

    fn attempt(&mut self, order: &DispatchOrder, report: AttemptReport) -> Outbox {
        let done = AttemptDone {
            request_id: order.request_id.clone(),
            effect_id: order.effect_id.clone(),
            attempt: order.attempt,
            report,
            finished_at_ms: self.now,
        };
        self.step(|core| core.attempt_done(done).unwrap())
    }

    fn state(&mut self, request_id: &Id) -> RequestState {
        self.core()
            .read(|r| r.request(request_id))
            .unwrap()
            .expect("the request")
            .row
            .request
            .status
            .state()
    }

    fn ledger_head(&mut self) -> u64 {
        self.core().read(|r| r.meta()).unwrap().ledger_head_seq
    }

    /// A proposal asked to the owner: `pending_approval`.
    fn pending(&mut self) -> Id {
        let (request_id, out) = self.propose();
        let job = job(&out, &request_id, ProofPurpose::Classification);
        let _ = self.prove(done(&job, "need_approval", &[]));
        assert_eq!(self.state(&request_id), RequestState::PendingApproval);
        request_id
    }

    /// The owner approves for `kind`, as `decide_request` writes it; returns the grant and the
    /// fresh-permit job the pump then starts.
    fn approve(&mut self, request_id: &Id, kind: DurationKind) -> (Id, ProofJob) {
        let now = self.now;
        let request_id = request_id.clone();
        let mut granted = None;
        let out = self.step(|core| {
            let meta = core.meta(
                "owner.decide_request",
                Actor::owner(),
                Some(&request_id),
                None,
            );
            let written = core.write::<Id, OwnerRefusal>(meta, |tx, cx| {
                let record = record_of(tx, &request_id)?;
                let scope = record.card.as_ref().expect("a card").scope.clone();
                let duration = Duration { kind };
                let digest = approval_digest(
                    &request_id,
                    &record.row.request.request_digest,
                    &Decision::Approve {
                        scope: scope.clone(),
                        duration,
                    },
                );
                let (grant, _) = create_grant(tx, cx, &record, &scope, duration, &digest)?;
                tx.apply(Mutation::ApprovalRecorded {
                    request_id: request_id.clone(),
                    approval: Approval::Approve(Approved {
                        command_id: id("command-1"),
                        decided_at: timestamp(i64::try_from(now).unwrap()),
                        approval_digest: digest,
                        scope,
                        duration,
                        effective_request_digest: record.row.request.request_digest.clone(),
                        grant_id: grant.grant_id.clone(),
                    }),
                })?;
                set_relied(tx, &request_id, vec![grant.grant_id.clone()])?;
                let status = decided(record.row.request.status.state(), true).expect("pending");
                let due = cx.now_ms;
                move_request(tx, cx, &request_id, status, Some(due))?;
                Ok(grant.grant_id)
            });
            match written {
                Ok(committed) => granted = Some(committed.value),
                Err(TxError::Fault(f)) => panic!("the approval failed: {f}"),
                Err(TxError::Refused(r)) => panic!("the approval was refused: {}", r.message),
            }
        });
        let grant = granted.expect("a grant");
        (grant, job(&out, &request_id, ProofPurpose::FreshPermit))
    }

    /// Ends a grant as `revoke_grant` does.
    fn end(&mut self, grant_id: &Id) {
        let grant_id = grant_id.clone();
        let _ = self.step(|core| {
            let meta = core.meta("owner.revoke_grant", Actor::owner(), None, None);
            core.commit(meta, |tx, cx| {
                end_grant(tx, cx, &grant_id, GrantEndReason::RevokedByOwner).map(|_| ())
            })
            .unwrap();
        });
    }
}

fn jobs(out: &Outbox) -> Vec<ProofJob> {
    out.jobs
        .iter()
        .filter_map(|j| match j {
            Job::Prove(p) => Some(p.clone()),
            Job::Dispatch(_) => None,
        })
        .collect()
}

fn orders(out: &Outbox) -> Vec<DispatchOrder> {
    out.jobs
        .iter()
        .filter_map(|j| match j {
            Job::Dispatch(d) => Some(d.clone()),
            Job::Prove(_) => None,
        })
        .collect()
}

fn job(out: &Outbox, request_id: &Id, purpose: ProofPurpose) -> ProofJob {
    let found: Vec<ProofJob> = jobs(out)
        .into_iter()
        .filter(|j| &j.request_id == request_id && j.purpose == purpose)
        .collect();
    assert_eq!(found.len(), 1, "expected one {purpose:?} job");
    found.into_iter().next().unwrap()
}

static PROOFS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A result for `job` with `outcome` and `grants` as its authorizing grants.
fn done(job: &ProofJob, outcome: &str, grants: &[&Id]) -> ProofDone {
    let n = PROOFS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let proof_id = format!("proof-{n}");
    let rule = matches!(outcome, "block" | "need_approval").then_some("test_rule");
    let report: ProofReport = record(&json!({
        "proof_id": proof_id, "request_id": job.request_id, "purpose": job.purpose,
        "outcome": outcome, "failing_rule": rule, "rule_verdicts": [],
        "isabelle_version": "Isabelle2025",
        "theory_hash": format!("sha256:{}", "a".repeat(64)),
        "context_digest": format!("sha256:{}", "b".repeat(64)),
        "action_digest": job.action_digest,
        "started_at": timestamp(i64::try_from(job.started_at_ms).unwrap()),
        "elapsed_ms": 10
    }));
    let ruling = rule.map(|rule| {
        record::<Ruling>(&json!({"rule": rule, "explanation": "Because.", "proof_id": proof_id}))
    });
    ProofDone {
        job_id: job.job_id,
        request_id: job.request_id.clone(),
        purpose: job.purpose,
        report: Some(report),
        ruling,
        authorizing_grants: grants.iter().map(|g| (*g).clone()).collect(),
        undecided_reason: None,
        check_theory_text: None,
        cause: None,
    }
}

fn ruling() -> Ruling {
    record(&json!({"rule": "test_rule", "explanation": "Because.", "proof_id": "proof-1"}))
}

fn offered(job: &ProofJob) -> Vec<(String, bool)> {
    job.facts
        .grants
        .iter()
        .map(|f| (f.grant.grant_id.as_str().to_owned(), f.ended))
        .collect()
}

#[test]
fn facts_hold_every_connected_account_and_its_selected_calendars() {
    // 11.3 W2: no filter by the accounts the action names.
    let mut run = Run::new();
    run.seed(
        &[AGENT],
        vec![
            account("acct-a", "connected", &[("cal-1", true), ("cal-2", false)]),
            account("acct-b", "connected", &[("cal-1", true)]),
            account("acct-c", "disconnected", &[("cal-1", true)]),
        ],
    );
    let _ = run.connect(1, AGENT);
    let (request_id, out) = run.propose_on("acct-b", "cal-1");
    let job = job(&out, &request_id, ProofPurpose::Classification);
    let connected: Vec<&str> = job.facts.connected.iter().map(Id::as_str).collect();
    assert_eq!(connected, ["acct-a", "acct-b"]);
    let calendars: Vec<(&str, &str)> = job
        .facts
        .calendars
        .iter()
        .map(|c| {
            (
                c.calendar.account_id.as_str(),
                c.calendar.calendar_id.as_str(),
            )
        })
        .collect();
    assert_eq!(calendars, [("acct-a", "cal-1"), ("acct-b", "cal-1")]);
    assert!(job.facts.calendars.iter().all(|c| c.writable));
    assert_eq!(job.facts.relied, Vec::<Id>::new());
    assert!(!job.facts.paused);
    assert!(job.facts.session.is_some());
    assert_eq!(job.deadline_ms, job.started_at_ms + PROOF_WATCHDOG_MS);
}

#[test]
fn a_classification_has_no_relied_list_and_a_fresh_proof_the_stored_one() {
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, fresh) = run.approve(&request_id, DurationKind::UntilRevoked);
    assert_eq!(fresh.facts.relied, vec![grant.clone()]);
    assert!(offered(&fresh).contains(&(grant.as_str().to_owned(), false)));

    // A later classification is offered the live grant but no relied list.
    let (other, out) = run.propose();
    let classification = job(&out, &other, ProofPurpose::Classification);
    assert!(classification.facts.relied.is_empty());
    assert!(offered(&classification).contains(&(grant.as_str().to_owned(), false)));
}

#[test]
fn an_ended_relied_grant_is_offered_to_the_fresh_proof_marked_ended() {
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, fresh) = run.approve(&request_id, DurationKind::UntilRevoked);
    run.end(&grant);
    // The permit was computed before the end: the fence changed, so it is proved again.
    let out = run.prove(done(&fresh, "permit", &[&grant]));
    assert!(orders(&out).is_empty());
    let again = job(&out, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(again.facts.relied, vec![grant.clone()]);
    assert_eq!(offered(&again), vec![(grant.as_str().to_owned(), true)]);
    let reproofs = run
        .core()
        .read(|r| r.request(&request_id))
        .unwrap()
        .unwrap()
        .reproofs;
    assert_eq!(reproofs.fresh_permit, 1);
    assert_eq!(reproofs.classification, 0);
}

#[test]
fn a_one_request_grant_is_offered_only_to_its_origin_request() {
    let mut run = Run::new();
    run.setup();
    let origin = run.pending();
    let (grant, fresh) = run.approve(&origin, DurationKind::OneRequest);
    assert!(offered(&fresh).contains(&(grant.as_str().to_owned(), false)));
    let (other, out) = run.propose();
    let classification = job(&out, &other, ProofPurpose::Classification);
    assert!(
        offered(&classification)
            .iter()
            .all(|(g, _)| g != grant.as_str())
    );
}

#[test]
fn a_result_naming_a_grant_outside_its_facts_is_undecided_for_every_purpose_and_verdict() {
    let mut run = Run::new();
    run.setup();
    let unknown = id("grant-unknown");
    for outcome in ["permit", "need_approval", "block", "undecided"] {
        // Classification.
        let (request_id, out) = run.propose();
        let job = job(&out, &request_id, ProofPurpose::Classification);
        let result = done(&job, outcome, &[&unknown]);
        let proof_id = result.report.as_ref().unwrap().proof_id.clone();
        let out = run.prove(result);
        assert!(jobs(&out).is_empty(), "{outcome}");
        assert_eq!(run.state(&request_id), RequestState::Undecided, "{outcome}");
        let recorded = run.core().read(|r| r.proof(&proof_id)).unwrap().unwrap();
        assert_eq!(
            recorded.undecided_reason.as_deref(),
            Some("authorizing grant outside the facts")
        );
        assert_eq!(recorded.authorizing_grants, vec![unknown.clone()]);
        assert_eq!(recorded.prover, ProverKind::Isabelle);

        // Fresh permit: the relied list stays the approval's grant.
        let pending = run.pending();
        let (grant, fresh) = run.approve(&pending, DurationKind::UntilRevoked);
        let out = run.prove(done(&fresh, outcome, &[&grant, &unknown]));
        assert!(out.jobs.is_empty(), "{outcome}");
        assert_eq!(run.state(&pending), RequestState::Undecided, "{outcome}");
        let relied = run.core().read(|r| r.request(&pending)).unwrap().unwrap();
        assert_eq!(relied.relied_grant_ids, vec![grant], "{outcome}");
        run.now += 61_000;
    }
    run.verify();
}

#[test]
fn the_watchdog_ends_a_proof_past_35_s_and_a_late_result_is_ignored() {
    let mut run = Run::new();
    run.setup();
    let (request_id, out) = run.propose();
    let job = job(&out, &request_id, ProofPurpose::Classification);
    run.now = job.deadline_ms;
    let _ = run.tick();
    assert_eq!(run.state(&request_id), RequestState::ProvingClassification);
    run.now = job.deadline_ms + 1;
    let _ = run.tick();
    assert_eq!(run.state(&request_id), RequestState::Undecided);
    let head = run.ledger_head();
    let out = run.prove(done(&job, "permit", &[]));
    assert!(out.jobs.is_empty());
    assert_eq!(run.ledger_head(), head, "a late result was recorded");
}

#[test]
fn a_fresh_permit_is_authorized_then_handed_off_in_its_own_commit() {
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, fresh) = run.approve(&request_id, DurationKind::UntilRevoked);
    let head = run.ledger_head();
    let out = run.prove(done(&fresh, "permit", &[&grant]));
    // One commit authorizes (`queued`), the next hands off (`dispatching`).
    assert_eq!(run.ledger_head(), head + 2);
    let orders = orders(&out);
    assert_eq!(orders.len(), 1);
    let order = &orders[0];
    assert_eq!(order.attempt, 1);
    assert_eq!(order.authorization.grant_id, grant);
    assert!(order.authorization.consumed_at.is_some());
    assert_eq!(order.provider_event_id.as_ref().map(String::len), Some(26));
    assert_eq!(run.state(&request_id), RequestState::Dispatching);
    let authorization = run
        .core()
        .read(|r| r.latest_authorization_of(&request_id))
        .unwrap()
        .unwrap();
    assert_eq!(authorization.state, AuthorizationState::Consumed);
    assert_eq!(authorization.grant_ids, vec![grant]);
}

#[test]
fn a_transient_attempt_waits_and_the_retry_proves_again_on_the_relied_list() {
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, fresh) = run.approve(&request_id, DurationKind::UntilRevoked);
    let out = run.prove(done(&fresh, "permit", &[&grant]));
    let order = orders(&out).remove(0);
    let out = run.attempt(
        &order,
        AttemptReport::Transient {
            retry_after_ms: None,
        },
    );
    assert!(out.jobs.is_empty());
    assert_eq!(run.state(&request_id), RequestState::RetryWait);
    assert_eq!(run.core().next_due_raw_ms(), Some(run.now + 2_000));
    run.now += 1_999;
    assert!(run.tick().jobs.is_empty());
    run.now += 1;
    let out = run.tick();
    let retry = job(&out, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(retry.facts.relied, vec![grant.clone()]);
    let out = run.prove(done(&retry, "permit", &[&grant]));
    assert_eq!(orders(&out)[0].attempt, 2);
    // A different authorization for the second attempt.
    assert_ne!(
        orders(&out)[0].authorization.authorization_id,
        order.authorization.authorization_id
    );
    run.verify();
}

#[test]
fn recovery_ends_the_dead_runs_sessions_and_leaves_its_dispatch_unknown() {
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, fresh) = run.approve(&request_id, DurationKind::Session);
    let out = run.prove(done(&fresh, "permit", &[&grant]));
    let order = orders(&out).remove(0);
    run.now += 1_000;
    let _ = run.restart();
    assert_eq!(run.state(&request_id), RequestState::OutcomeUnknown);
    let (ended, sessions, attempts) = run
        .core()
        .read(|r| {
            Ok((
                r.grant(&grant)?.unwrap().ended,
                r.live_sessions()?,
                r.attempts(&order.effect_id)?,
            ))
        })
        .unwrap();
    assert_eq!(ended.map(|e| e.reason), Some(GrantEndReason::BrokerRestart));
    assert!(sessions.is_empty());
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].attempt.outcome,
        Some(AttemptOutcome::OutcomeUnknown)
    );
    // Nothing is handed off again.
    assert!(orders(&run.tick()).is_empty());
    run.verify();
}

#[test]
fn a_restart_with_nothing_open_sends_no_notice() {
    let mut run = Run::new();
    run.seed(
        &[AGENT],
        vec![account(ACCOUNT, "connected", &[(CALENDAR, true)])],
    );
    let before = run.core().read(|r| r.meta()).unwrap().notice_sequence;
    run.now += 1_000;
    let _ = run.restart();
    let after = run.core().read(|r| r.meta()).unwrap().notice_sequence;
    assert_eq!(after, before);
}

#[test]
fn a_repeated_call_returns_its_request_and_writes_nothing() {
    let mut run = Run::new();
    run.setup();
    let params = |title: &str| {
        json!({"account_id": ACCOUNT, "calendar_id": CALENDAR,
               "event": {"title": title,
                         "start": {"date_time": "2026-11-27T10:00:00+01:00", "zone": "Europe/Berlin"},
                         "end": {"date_time": "2026-11-27T11:00:00+01:00", "zone": "Europe/Berlin"},
                         "description": null, "location": null},
               "task": "Plan."})
    };
    let out = run.call(1, "call-1", "fabric_calendar_propose_create", params("A"));
    let first = out.agent[0].frame.to_value()["result"]["request"]["request_id"].clone();
    let head = run.ledger_head();
    let out = run.call(1, "call-1", "fabric_calendar_propose_create", params("A"));
    assert_eq!(
        out.agent[0].frame.to_value()["result"]["request"]["request_id"],
        first
    );
    let out = run.call(
        1,
        "call-2",
        "fabric_request_status",
        json!({"request_id": first}),
    );
    assert_eq!(
        out.agent[0].frame.to_value()["result"]["request"]["request_id"],
        first
    );
    let out = run.call(1, "call-1", "fabric_calendar_propose_create", params("B"));
    assert_eq!(
        out.agent[0].frame.to_value()["error"]["code"],
        "invalid_params"
    );
    assert_eq!(run.ledger_head(), head);
}

#[test]
fn a_call_past_a_limit_is_busy_and_counted_in_the_agents_window() {
    let mut run = Run::new();
    run.setup();
    for _ in 0..15 {
        let _ = run.propose();
    }
    run.now += 1_000;
    let head = run.ledger_head();
    let out = run.call(
        1,
        "call-busy",
        "fabric_calendar_propose_create",
        json!({"account_id": ACCOUNT, "calendar_id": CALENDAR,
               "event": {"title": "x",
                         "start": {"date_time": "2026-11-27T10:00:00+01:00", "zone": "Europe/Berlin"},
                         "end": {"date_time": "2026-11-27T11:00:00+01:00", "zone": "Europe/Berlin"},
                         "description": null, "location": null},
               "task": "x"}),
    );
    let error = out.agent[0].frame.to_value()["error"].clone();
    assert_eq!(error["code"], "busy");
    assert_eq!(error["detail"]["limit"], "requests_per_minute_per_agent");
    assert_eq!(error["detail"]["retry_after_ms"], 59_000);
    assert_eq!(run.ledger_head(), head);
    assert_eq!(
        run.core().busy.next_end_ms(),
        Some(T0 as i64 + 1_000 + 60_000)
    );
}

#[test]
fn a_connect_that_replaces_a_session_raises_the_revision_once_and_closes_the_old_connection() {
    let mut run = Run::new();
    run.setup();
    let before = run
        .core()
        .read(|r| r.agent(&id(AGENT)))
        .unwrap()
        .unwrap()
        .revision;
    let head = run.ledger_head();
    let out = run.connect(2, AGENT);
    assert_eq!(run.ledger_head(), head + 1);
    let agent_notices = out
        .notices
        .iter()
        .filter(|n| matches!(n.change, fabric_desktop_contracts::owner::Change::Agent(_)))
        .count();
    assert_eq!(agent_notices, 1);
    assert_eq!(out.close, vec![ConnectionId(1)]);
    let after = run
        .core()
        .read(|r| r.agent(&id(AGENT)))
        .unwrap()
        .unwrap()
        .revision;
    assert_eq!(after, before + 1);
    let sessions = run.core().read(|r| r.live_sessions()).unwrap();
    assert_eq!(sessions.len(), 1);
    run.verify();
}

#[test]
fn a_call_on_a_connection_without_a_session_is_not_permitted() {
    let mut run = Run::new();
    run.setup();
    let out = run.call(
        9,
        "call-1",
        "fabric_request_status",
        json!({"request_id": "request-1"}),
    );
    assert_eq!(
        out.agent[0].frame.to_value()["error"]["code"],
        "not_permitted"
    );
    let out = run.connect(7, "agent-unknown");
    assert_eq!(out.close, vec![ConnectionId(7)]);
}

#[test]
fn quit_ends_live_sessions_and_their_grants_with_app_quit() {
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, _) = run.approve(&request_id, DurationKind::Session);
    let core = run.core();
    core.quit().unwrap();
    let ended = core.read(|r| r.grant(&grant)).unwrap().unwrap().ended;
    assert_eq!(ended.map(|e| e.reason), Some(GrantEndReason::AppQuit));
    assert!(core.read(|r| r.live_sessions()).unwrap().is_empty());
    assert!(core.take_outbox().close.contains(&ConnectionId(1)));
}

#[test]
fn an_idle_session_ends_with_its_grants_at_1800000_ms() {
    // c39: the session is live while now < last activity + the limit (5.5), so it ends
    // at the limit itself, with its grant.
    let mut run = Run::new();
    run.setup();
    let request_id = run.pending();
    let (grant, _) = run.approve(&request_id, DurationKind::Session);
    let last_call = run.now;
    run.now = last_call + 1_799_999;
    let _ = run.tick();
    assert_eq!(run.core().read(|r| r.live_sessions()).unwrap().len(), 1);
    run.now = last_call + 1_800_000;
    let _ = run.tick();
    let (ended, sessions) = run
        .core()
        .read(|r| Ok((r.grant(&grant)?.unwrap().ended, r.live_sessions()?)))
        .unwrap();
    assert!(sessions.is_empty());
    assert_eq!(
        ended.map(|e| e.reason),
        Some(GrantEndReason::SessionIdleTimeout)
    );
    let session = run.core().read(|r| r.live_session(&id(AGENT))).unwrap();
    assert!(session.is_none());
    run.verify();
}

#[test]
fn a_discarded_classification_is_proved_again_until_its_counter_is_spent() {
    let mut run = Run::new();
    run.setup();
    let (request_id, out) = run.propose();
    let mut current = job(&out, &request_id, ProofPurpose::Classification);
    for n in 1..=MAX_REPROOFS {
        // Any ended grant raises the revocation epoch; a fence change stands for all.
        let _ = run.step(|core| {
            let meta = core.meta("owner.save_group", Actor::owner(), None, None);
            core.commit(meta, |tx, _| {
                super::epochs::raise(tx, fabric_storage::Counter::RevocationEpoch).map(|_| ())
            })
            .unwrap();
        });
        let out = run.prove(done(&current, "need_approval", &[]));
        current = job(&out, &request_id, ProofPurpose::Classification);
        let reproofs = run
            .core()
            .read(|r| r.request(&request_id))
            .unwrap()
            .unwrap()
            .reproofs;
        assert_eq!(reproofs.classification, n);
    }
    let _ = run.step(|core| {
        let meta = core.meta("owner.save_group", Actor::owner(), None, None);
        core.commit(meta, |tx, _| {
            super::epochs::raise(tx, fabric_storage::Counter::RevocationEpoch).map(|_| ())
        })
        .unwrap();
    });
    let last = done(&current, "need_approval", &[]);
    let last_proof = last.report.as_ref().unwrap().proof_id.clone();
    let out = run.prove(last);
    assert!(out.jobs.is_empty());
    assert_eq!(run.state(&request_id), RequestState::Undecided);
    // c33: the discarded proof is recorded, and the move states that the re-proofs are spent.
    let (cause, recorded) = run
        .core()
        .read(|r| Ok((r.undecided_cause_of(&request_id)?, r.proof(&last_proof)?)))
        .unwrap();
    assert_eq!(cause, Some(UndecidedCause::ReproofsSpent));
    assert_eq!(
        cause.and_then(crate::owner::views::reason_of),
        Some(fabric_desktop_contracts::common::UndecidedReason::Failed)
    );
    assert!(recorded.is_some(), "the discarded proof was not recorded");
    run.verify();
}

/// c34. 5.7: only `undecide` moves a request to `undecided`, with its cause. `advance`
/// refuses the event, and `move_request_with` refuses a move to `undecided` without a cause and
/// a cause on any other move.
#[test]
fn advance_refuses_undecide() {
    let mut run = Run::new();
    run.setup();
    let (request_id, _) = run.propose();
    assert_eq!(run.state(&request_id), RequestState::ProvingClassification);
    let refused = |run: &mut Run,
                   f: &dyn Fn(
        &mut fabric_storage::WriteTx<'_>,
        &mut super::Cx<'_>,
    ) -> Result<(), StoreFault>| {
        let core = run.core();
        let meta = core.meta("request.advance", Actor::broker(), Some(&request_id), None);
        match core.commit(meta, f) {
            Err(StoreFault::Invariant { what }) => what,
            other => panic!("expected an invariant, got {other:?}"),
        }
    };
    assert_eq!(
        refused(&mut run, &|tx, cx| advance(
            tx,
            cx,
            &request_id,
            Event::Undecide,
            None,
            None
        )),
        "undecided names its cause"
    );
    assert_eq!(
        refused(&mut run, &|tx, cx| {
            let to = super::plain(RequestState::Undecided)?;
            move_request(tx, cx, &request_id, to, None).map(|_| ())
        }),
        "a move to undecided names its cause"
    );
    assert_eq!(
        refused(&mut run, &|tx, cx| {
            let to = LifecycleStatus::Blocked { denial: ruling() };
            move_request_with(
                tx,
                cx,
                &request_id,
                to,
                None,
                Some(UndecidedCause::NoRuling),
            )
            .map(|_| ())
        }),
        "only a move to undecided names its cause"
    );
    assert_eq!(run.state(&request_id), RequestState::ProvingClassification);
    let core = run.core();
    let meta = core.meta("request.advance", Actor::broker(), Some(&request_id), None);
    core.commit(meta, |tx, cx| {
        undecide(tx, cx, &request_id, UndecidedCause::NoRuling)
    })
    .unwrap();
    assert_eq!(run.state(&request_id), RequestState::Undecided);
    let cause = run
        .core()
        .read(|r| r.undecided_cause_of(&request_id))
        .unwrap();
    assert_eq!(cause, Some(UndecidedCause::NoRuling));
    run.verify();
}

#[test]
fn a_backward_clock_step_is_reported_once_and_never_lowers_now() {
    let mut run = Run::new();
    let core = run.core();
    let first = core.now_ms(T0 + 5_000);
    let back = core.now_ms(T0 + 1_000);
    assert_eq!(back, first);
    let lines = core.take_outbox().stderr;
    assert_eq!(
        lines,
        vec!["the clock went back 4000 ms; time continues from the last commit".to_owned()]
    );
    let _ = core.now_ms(T0 + 1_500);
    assert!(core.take_outbox().stderr.is_empty());
}

/// Puts a request in `queued` with an issued authorization of this run, without the pump that
/// hands it off in the same step (5.1). No public input stops there, so this is the only way to
/// reach the dispatch check's void branches other than `broker_restart` (design 9.3 c08, c29).
fn queued(run: &mut Run) -> (Id, Id) {
    run.setup();
    let request_id = run.pending();
    let (grant, fresh) = run.approve(&request_id, DurationKind::UntilRevoked);
    let now = run.now;
    let core = run.core();
    let _ = core.now_ms(now);
    core.pump().unwrap();
    core.proof_done(done(&fresh, "permit", &[&grant])).unwrap();
    let _ = core.take_outbox();
    assert_eq!(run.state(&request_id), RequestState::Queued);
    (request_id, grant)
}

/// The dispatch check of a queued request at the raw time `raw_now`: `None` when it hands off,
/// the void reason otherwise.
fn handoff_at(run: &mut Run, request_id: &Id, raw_now: u64) -> Option<VoidReason> {
    let core = run.core();
    let now = core.now_ms(raw_now);
    let record = core
        .read(|r| r.request(request_id))
        .unwrap()
        .expect("the request");
    let check = core
        .read(|r| handoff(r, &core.activity, now, core.run_id, &record))
        .unwrap();
    match check {
        Handoff::Ready(_) => None,
        Handoff::Void(_, reason) => Some(reason),
        Handoff::Missing => panic!("no issued authorization"),
    }
}

/// One commit of `operation_type`, with no pump around it.
fn commit_now(
    run: &mut Run,
    operation_type: &'static str,
    f: impl FnOnce(&mut fabric_storage::WriteTx<'_>) -> Result<(), fabric_storage::StoreFault>,
) {
    let core = run.core();
    let meta = core.meta(operation_type, Actor::owner(), None, None);
    core.commit(meta, |tx, _| f(tx)).unwrap();
}

/// After a void: the next step hands nothing off, the authorization is voided and a fresh
/// proof starts (5.7 `queued -> proving_fresh_permit`).
fn assert_voided_on_the_next_step(run: &mut Run, request_id: &Id) {
    let out = run.tick();
    assert!(
        orders(&out).is_empty(),
        "a voided authorization was handed off"
    );
    let _ = job(&out, request_id, ProofPurpose::FreshPermit);
    assert_eq!(run.state(request_id), RequestState::ProvingFreshPermit);
    let authorization = run
        .core()
        .read(|r| r.latest_authorization_of(request_id))
        .unwrap()
        .unwrap();
    assert_eq!(authorization.state, AuthorizationState::Voided);
    run.verify();
}

/// c29 (unit). Design D34 and 5.7: an authorization issued before a backward clock step is
/// expired at issued + 60 000 on the floored clock: the wall clock goes back 30 s, time runs on
/// at wall rate from the last value, so it is handed off up to 59 999 ms of floored time and
/// voided `expired` at 60 000.
#[test]
fn an_authorization_issued_before_a_clock_step_expires_60_s_later_on_the_floored_clock() {
    let mut run = Run::new();
    let (request_id, _) = queued(&mut run);
    let issued = run.now;
    let step = 30_000;
    assert_eq!(handoff_at(&mut run, &request_id, issued - step), None);
    assert_eq!(
        handoff_at(&mut run, &request_id, issued - step + 59_999),
        None
    );
    assert_eq!(
        handoff_at(&mut run, &request_id, issued - step + 60_000),
        Some(VoidReason::Expired)
    );
    run.now = issued - step + 60_000;
    assert_voided_on_the_next_step(&mut run, &request_id);
}

/// c08 (unit). Design 5.6 and PLAN 8 step 7: a fence that changed while the request sat in
/// `queued` voids its authorization `fence_changed`.
#[test]
fn a_fence_changed_while_queued_voids_the_authorization() {
    let mut run = Run::new();
    let (request_id, _) = queued(&mut run);
    commit_now(&mut run, "owner.save_group", |tx| {
        super::epochs::raise(tx, fabric_storage::Counter::RevocationEpoch).map(|_| ())
    });
    let now = run.now;
    assert_eq!(
        handoff_at(&mut run, &request_id, now),
        Some(VoidReason::FenceChanged)
    );
    assert_voided_on_the_next_step(&mut run, &request_id);
}

/// c08 (unit). Design 5.5 and PLAN 8 step 7: a relied grant that ended while the request sat in
/// `queued` voids its authorization `grant_invalid`. The owner's revoke also raises the
/// revocation epoch, so it is caught as `fence_changed` first; here the grant ends alone.
#[test]
fn a_grant_ended_while_queued_voids_the_authorization() {
    let mut run = Run::new();
    let (request_id, grant_id) = queued(&mut run);
    let now = i64::try_from(run.now).unwrap();
    commit_now(&mut run, "owner.revoke_grant", |tx| {
        let grant = tx.read().grant(&grant_id)?.expect("the grant");
        tx.apply(Mutation::GrantEnded {
            grant_id: grant_id.clone(),
            revision: grant.revision.get() + 1,
            end: fabric_desktop_contracts::owner::GrantEnd {
                at: timestamp(now),
                reason: GrantEndReason::RevokedByOwner,
            },
        })
    });
    let raw = run.now;
    assert_eq!(
        handoff_at(&mut run, &request_id, raw),
        Some(VoidReason::GrantInvalid)
    );
    assert_voided_on_the_next_step(&mut run, &request_id);
}

/// c08 (unit). Design 5.6 and PLAN 8 step 7: an agent paused while its request sat in `queued`
/// has the authorization voided `agent_paused`. The owner's pause also raises the pause epoch,
/// so it is caught as `fence_changed` first; here the agent row changes alone.
#[test]
fn an_agent_paused_while_queued_voids_the_authorization() {
    let mut run = Run::new();
    let (request_id, _) = queued(&mut run);
    commit_now(&mut run, "owner.pause_agent", |tx| {
        let mut agent = tx.read().agent(&id(AGENT))?.expect("the agent");
        agent.paused = true;
        agent.revision += 1;
        tx.apply(Mutation::AgentPut { agent })
    });
    let now = run.now;
    assert_eq!(
        handoff_at(&mut run, &request_id, now),
        Some(VoidReason::AgentPaused)
    );
    assert_voided_on_the_next_step(&mut run, &request_id);
}
