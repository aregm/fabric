//! Replay: the projection is the fold of the ledger's mutations (design 4, D3; 9.2 s13).
//!
//! Seeded random sequences of writes, each the kind of write the core makes (section 4's
//! operation types, revisions that rise by one, lifecycle moves the contract allows), touch
//! every projection table; `verify_replay` must rebuild all of them from the ledger. A
//! projection changed outside `apply` must make it fail.

mod common;

use std::path::Path;

use common::{
    T0, TempDir, agent_row, apply_all, contract, count_rows, id, meta, new_store, open,
    preferences, raw, ts, verify,
};
use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::digest::sha256_hex;
use fabric_desktop_contracts::owner::{
    AttemptOutcome, CustomProfile, GrantEnd, GrantEndReason, LifecycleStatus, ProfileRows,
};
use fabric_desktop_contracts::scalar::{CustomProfileId, Digest, Label, PositiveCount};
use fabric_storage::{
    Actor, AttemptRow, AuthorizationRecord, AuthorizationState, CommandRow, Counter, EffectRow,
    Fence, IdempotencyScope, Mutation, OpMeta, ProofRecord, ProverKind, RequestRow,
    SessionEndReason, SessionRow, Store, UndecidedCause,
};
use serde_json::{Value, json};

/// Every projection table (3.5: every table except `meta`, `ledger` and `audit`).
const PROJECTION_TABLES: [&str; 17] = [
    "runs",
    "counters",
    "profile_versions",
    "profile_state",
    "preferences",
    "recipient_sets",
    "recipient_set_versions",
    "accounts",
    "agents",
    "sessions",
    "requests",
    "grants",
    "proof_reports",
    "authorizations",
    "effects",
    "attempts",
    "commands",
];

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

fn digest_of(text: &str) -> Digest {
    Digest::new(format!("sha256:{}", sha256_hex(text))).unwrap()
}

fn status(state: &str) -> LifecycleStatus {
    contract(json!({ "state": state }))
}

fn scope() -> Value {
    json!({
        "calendars": [{ "account_id": "acct-1", "calendar_id": "cal-primary" }],
        "window": null,
        "fields": null,
        "recipients": null,
        "send_updates": null
    })
}

fn ruling(rule: &str, proof_id: &str) -> Value {
    json!({ "rule": rule, "explanation": "The profile asks the owner.", "proof_id": proof_id })
}

/// Where a request is in the pipeline the test drives.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    Received,
    Validating,
    ProvingClassification,
    PendingApproval,
    Approved,
    ProvingFreshPermit,
    Queued,
    Dispatching,
    Done,
}

#[derive(Clone, Debug)]
struct Pipeline {
    request_id: String,
    agent_id: String,
    revision: u64,
    stage: Stage,
    action: Action,
    grant_id: Option<String>,
    authorization_id: Option<String>,
    fresh_proof_id: Option<String>,
    effect_id: Option<String>,
    approved_path: bool,
}

/// What the test knows of the store, so that each write it makes is one the core could make.
struct Model {
    rng: Rng,
    seed: u64,
    now: i64,
    run: i64,
    next: u64,
    agents: Vec<(String, u64, bool)>,
    sessions: Vec<(String, String)>,
    pause_epoch: u64,
    revocation_epoch: u64,
    preferences_revision: u64,
    sets: Vec<(String, u64, bool)>,
    accounts: Vec<(String, u64)>,
    custom_profiles: Vec<(String, u64)>,
    profiles_revision: u64,
    pipelines: Vec<Pipeline>,
    completed_approved: usize,
    undecided_with_cause: usize,
}

/// One write: its metadata, its mutations and what it is, for the failure message.
struct Step {
    meta: OpMeta,
    mutations: Vec<Mutation>,
    what: String,
}

impl Model {
    fn new(seed: u64) -> Model {
        Model {
            rng: Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1),
            seed,
            now: T0,
            run: 1,
            next: 0,
            agents: Vec::new(),
            sessions: Vec::new(),
            pause_epoch: 0,
            revocation_epoch: 0,
            preferences_revision: 0,
            sets: Vec::new(),
            accounts: Vec::new(),
            custom_profiles: Vec::new(),
            profiles_revision: 0,
            pipelines: Vec::new(),
            completed_approved: 0,
            undecided_with_cause: 0,
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}-{}-{}", self.seed, self.next)
    }

    fn tick(&mut self) -> i64 {
        self.now += 1 + i64::try_from(self.rng.below(3_000)).unwrap();
        self.now
    }

    fn fence(&self) -> Fence {
        Fence {
            pause_epoch: self.pause_epoch,
            revocation_epoch: self.revocation_epoch,
            event_revision: None,
        }
    }

    fn step(&self, meta: OpMeta, mutations: Vec<Mutation>, what: impl Into<String>) -> Step {
        Step {
            meta,
            mutations,
            what: what.into(),
        }
    }

    /// An owner command's metadata and its dedup row (D23).
    fn command(&mut self, operation_type: &'static str, method: &str) -> (OpMeta, Mutation) {
        let now = self.now;
        let command_id = self.fresh("command");
        let mut m = meta(operation_type, Actor::owner(), now);
        m.idempotency = Some((IdempotencyScope::OwnerCommand, command_id.clone()));
        m.causation_id = Some(command_id.clone());
        let row = CommandRow {
            command_id: id(&command_id),
            method: method.to_owned(),
            digest: digest_of(&command_id),
            answer: format!(
                r#"{{"protocol":"fabric-desktop/1","id":"{command_id}","result":{{}}}}"#
            ),
        };
        (m, Mutation::CommandAnswered { row })
    }

    fn add_agent(&mut self) -> Step {
        let now = self.tick();
        let agent_id = self.fresh("agent");
        self.agents.push((agent_id.clone(), 1, false));
        let mut row = agent_row(&agent_id, 1, "Random agent");
        row.enrolled_at_ms = now;
        self.step(
            meta("broker.seed", Actor::broker(), now),
            vec![Mutation::AgentPut { agent: row }],
            format!("add {agent_id}"),
        )
    }

    fn pause_agent(&mut self) -> Option<Step> {
        if self.agents.is_empty() {
            return None;
        }
        self.tick();
        let i = usize::try_from(self.rng.below(self.agents.len() as u64)).unwrap();
        let (agent_id, revision, paused) = self.agents[i].clone();
        self.agents[i] = (agent_id.clone(), revision + 1, !paused);
        let mut row = agent_row(&agent_id, revision + 1, "Random agent");
        row.paused = !paused;
        let (m, answered) = self.command("owner.pause_agent", "pause_agent");
        let mut mutations = vec![Mutation::AgentPut { agent: row }];
        if !paused {
            self.pause_epoch += 1;
            mutations.push(Mutation::CounterRaised {
                counter: Counter::PauseEpoch,
                to: self.pause_epoch,
            });
        }
        mutations.push(answered);
        Some(self.step(m, mutations, format!("pause {agent_id} -> {}", !paused)))
    }

    fn start_session(&mut self) -> Option<Step> {
        let free: Vec<String> = self
            .agents
            .iter()
            .map(|a| a.0.clone())
            .filter(|a| !self.sessions.iter().any(|s| &s.1 == a))
            .collect();
        if free.is_empty() {
            return None;
        }
        let now = self.tick();
        let agent_id = free[usize::try_from(self.rng.below(free.len() as u64)).unwrap()].clone();
        let session_id = self.fresh("session");
        self.sessions.push((session_id.clone(), agent_id.clone()));
        let session = SessionRow {
            session_id: id(&session_id),
            agent_id: id(&agent_id),
            run_id: self.run,
            started_at_ms: now,
            ended: None,
        };
        Some(self.step(
            meta("agent.connect", Actor::agent(id(&agent_id)), now),
            vec![Mutation::SessionStarted { session }],
            format!("connect {agent_id}"),
        ))
    }

    fn end_session(&mut self) -> Option<Step> {
        if self.sessions.is_empty() {
            return None;
        }
        let now = self.tick();
        let i = usize::try_from(self.rng.below(self.sessions.len() as u64)).unwrap();
        let (session_id, agent_id) = self.sessions.remove(i);
        Some(self.step(
            meta("agent.disconnect", Actor::agent(id(&agent_id)), now),
            vec![Mutation::SessionEnded {
                session_id: id(&session_id),
                at_ms: now,
                reason: SessionEndReason::Disconnected,
            }],
            format!("disconnect {session_id}"),
        ))
    }

    fn raise_revocation(&mut self) -> Step {
        self.tick();
        self.revocation_epoch += 1;
        let (m, answered) = self.command("owner.revoke_grant", "revoke_grant");
        let to = self.revocation_epoch;
        self.step(
            m,
            vec![
                Mutation::CounterRaised {
                    counter: Counter::RevocationEpoch,
                    to,
                },
                answered,
            ],
            format!("revocation epoch -> {to}"),
        )
    }

    fn save_preferences(&mut self) -> Step {
        self.tick();
        self.preferences_revision += 1;
        let (m, answered) = self.command("owner.save_preferences", "save_preferences");
        let revision = self.preferences_revision;
        self.step(
            m,
            vec![
                Mutation::PreferencesPut {
                    preferences: preferences(revision),
                },
                answered,
            ],
            format!("preferences -> {revision}"),
        )
    }

    fn save_group(&mut self) -> Step {
        let now = self.tick();
        let live: Vec<usize> = (0..self.sets.len()).filter(|&i| !self.sets[i].2).collect();
        let (set_id, revision) = if live.is_empty() || self.rng.chance(40) {
            let set_id = self.fresh("rset");
            self.sets.push((set_id.clone(), 1, false));
            (set_id, 1)
        } else {
            let i = live[usize::try_from(self.rng.below(live.len() as u64)).unwrap()];
            self.sets[i].1 += 1;
            (self.sets[i].0.clone(), self.sets[i].1)
        };
        let set = contract(json!({
            "recipient_set_id": set_id,
            "revision": revision,
            "source": { "kind": "local" },
            "definition": {
                "name": format!("Group {revision}"),
                "members": [format!("member{revision}@example.net")],
                "domains": []
            }
        }));
        let (m, answered) = self.command("owner.save_group", "save_group");
        self.step(
            m,
            vec![
                Mutation::RecipientSetVersionAdded { set, at_ms: now },
                answered,
            ],
            format!("group {set_id} -> {revision}"),
        )
    }

    fn delete_group(&mut self) -> Option<Step> {
        let live: Vec<usize> = (0..self.sets.len()).filter(|&i| !self.sets[i].2).collect();
        if live.is_empty() {
            return None;
        }
        let now = self.tick();
        let i = live[usize::try_from(self.rng.below(live.len() as u64)).unwrap()];
        self.sets[i].2 = true;
        let set_id = self.sets[i].0.clone();
        let (m, answered) = self.command("owner.save_group", "save_group");
        Some(self.step(
            m,
            vec![
                Mutation::RecipientSetDeleted {
                    recipient_set_id: id(&set_id),
                    at_ms: now,
                },
                answered,
            ],
            format!("delete group {set_id}"),
        ))
    }

    fn put_account(&mut self) -> Step {
        let now = self.tick();
        let (account_id, revision) = if self.accounts.is_empty() || self.rng.chance(30) {
            let account_id = if self.accounts.is_empty() {
                "acct-1".to_owned()
            } else {
                self.fresh("acct")
            };
            self.accounts.push((account_id.clone(), 1));
            (account_id, 1)
        } else {
            let i = usize::try_from(self.rng.below(self.accounts.len() as u64)).unwrap();
            self.accounts[i].1 += 1;
            (self.accounts[i].0.clone(), self.accounts[i].1)
        };
        let account = contract(json!({
            "account_id": account_id,
            "revision": revision,
            "state": "connected",
            "email": "owner@example.com",
            "calendars": [{
                "calendar_id": "cal-primary",
                "name": "Owner",
                "access": "owner",
                "selected": true,
                "agent_drafts": false
            }]
        }));
        self.step(
            meta("broker.seed", Actor::broker(), now),
            vec![Mutation::AccountPut { account }],
            format!("account {account_id} -> {revision}"),
        )
    }

    /// One `save_profile` commit: maybe a new custom version, and exactly one
    /// `ProfileStateSet` with the profiles revision plus one (D20).
    fn save_profile(&mut self, force_custom: bool) -> Step {
        let now = self.tick();
        let mut mutations = Vec::new();
        let (active_id, active_version) = if force_custom || self.rng.chance(70) {
            let (profile_id, version) = if self.custom_profiles.is_empty() || self.rng.chance(40) {
                let profile_id = self.fresh("custom");
                self.custom_profiles.push((profile_id.clone(), 1));
                (profile_id, 1)
            } else {
                let i = usize::try_from(self.rng.below(self.custom_profiles.len() as u64)).unwrap();
                self.custom_profiles[i].1 += 1;
                self.custom_profiles[i].clone()
            };
            mutations.push(Mutation::ProfileVersionAdded {
                profile: CustomProfile {
                    profile_id: CustomProfileId::new(profile_id.clone()).unwrap(),
                    name: Label::new("Assistant hours").unwrap(),
                    version: PositiveCount::new(version).unwrap(),
                    rows: ProfileRows::CAREFUL,
                },
                at_ms: now,
            });
            if self.rng.chance(50) {
                (profile_id, version)
            } else {
                ("careful".to_owned(), 1)
            }
        } else if self.rng.chance(50) {
            ("flexible".to_owned(), 1)
        } else {
            ("careful".to_owned(), 1)
        };
        self.profiles_revision += 1;
        mutations.push(Mutation::ProfileStateSet {
            active_profile_id: id(&active_id),
            active_version,
            revision: self.profiles_revision,
        });
        let (m, answered) = self.command("owner.save_profile", "save_profile");
        mutations.push(answered);
        self.step(
            m,
            mutations,
            format!("profiles -> {}", self.profiles_revision),
        )
    }

    /// Ends the run (its live sessions end with it) and starts the next, as quit and the next
    /// start do. Not while an authorization is issued: it is bound to its run.
    fn restart(&mut self, store: &mut Store) -> Option<String> {
        if self
            .pipelines
            .iter()
            .any(|p| matches!(p.stage, Stage::Queued))
        {
            return None;
        }
        let now = self.tick();
        let mut mutations: Vec<Mutation> = self
            .sessions
            .drain(..)
            .map(|(session_id, _)| Mutation::SessionEnded {
                session_id: id(&session_id),
                at_ms: now,
                reason: SessionEndReason::AppQuit,
            })
            .collect();
        mutations.push(Mutation::RunEnded {
            run_id: self.run,
            at_ms: now,
            clean: true,
        });
        let what = format!("quit run {}", self.run);
        commit(
            store,
            self.step(meta("broker.quit", Actor::broker(), now), mutations, what),
        );
        self.run += 1;
        let now = self.tick();
        common::start_run(store, self.run, now);
        Some(format!("restart as run {}", self.run))
    }

    fn insert_request(&mut self) -> Option<Step> {
        if self.agents.is_empty() {
            return None;
        }
        let now = self.tick();
        let agent_id = self.agents
            [usize::try_from(self.rng.below(self.agents.len() as u64)).unwrap()]
        .0
        .clone();
        let request_id = self.fresh("request");
        let call_id = self.fresh("call");
        let action: Action = contract(json!({
            "tool": "fabric_calendar_propose_create",
            "arguments": {
                "account_id": "acct-1",
                "calendar_id": "cal-primary",
                "event": {
                    "title": "Planning",
                    "start": { "date_time": "2026-11-20T14:00:00-06:00", "zone": "America/Chicago" },
                    "end": { "date_time": "2026-11-20T15:00:00-06:00", "zone": "America/Chicago" },
                    "description": "",
                    "location": ""
                },
                "task": "Block time to plan next week."
            }
        }));
        let request = contract(json!({
            "request_id": request_id,
            "revision": 1,
            "agent_id": agent_id,
            "tool": "fabric_calendar_propose_create",
            "account_id": "acct-1",
            "received_at": ts(now),
            "updated_at": ts(now),
            "request_digest": digest_of(&request_id),
            "status": { "state": "received" }
        }));
        let row = RequestRow {
            request,
            action: action.clone(),
            scope: None,
            session_id: None,
            call_id: id(&call_id),
        };
        self.pipelines.push(Pipeline {
            request_id: request_id.clone(),
            agent_id: agent_id.clone(),
            revision: 1,
            stage: Stage::Received,
            action,
            grant_id: None,
            authorization_id: None,
            fresh_proof_id: None,
            effect_id: None,
            approved_path: false,
        });
        let mut m = meta("agent.call", Actor::agent(id(&agent_id)), now);
        m.idempotency = Some((IdempotencyScope::AgentCall, format!("{agent_id}/{call_id}")));
        m.correlation_id = Some(id(&request_id));
        Some(self.step(
            m,
            vec![Mutation::RequestInserted {
                row,
                due_at_ms: None,
            }],
            format!("insert {request_id}"),
        ))
    }

    fn proof(&self, p: &Pipeline, proof_id: &str, purpose: &str, outcome: &str) -> ProofRecord {
        let failing_rule = match outcome {
            "need_approval" => json!("owner_approval_required"),
            "block" => json!("recurring_mutation_unsupported"),
            _ => Value::Null,
        };
        let ruling = match outcome {
            "need_approval" => Some(contract(ruling("owner_approval_required", proof_id))),
            "block" => Some(contract(ruling("recurring_mutation_unsupported", proof_id))),
            _ => None,
        };
        ProofRecord {
            report: contract(json!({
                "proof_id": proof_id,
                "request_id": p.request_id,
                "purpose": purpose,
                "outcome": outcome,
                "failing_rule": failing_rule,
                "rule_verdicts": [],
                "isabelle_version": "Isabelle2025",
                "theory_hash": digest_of("theory"),
                "context_digest": digest_of(proof_id),
                "action_digest": p.action.digest(),
                "started_at": ts(self.now),
                "elapsed_ms": 287
            })),
            prover: ProverKind::Isabelle,
            ruling,
            authorizing_grants: p.grant_id.iter().map(|g| id(g.as_str())).collect(),
            undecided_reason: None,
            fence: self.fence(),
            check_theory_text: None,
        }
    }

    /// Moves one request a step along the lifecycle of the contract.
    fn advance(&mut self, index: usize) -> Step {
        let now = self.tick();
        let mut p = self.pipelines[index].clone();
        let request_id = id(&p.request_id);
        let moved_with = |p: &mut Pipeline, state: LifecycleStatus, cause| {
            p.revision += 1;
            Mutation::RequestMoved {
                request_id: request_id.clone(),
                revision: p.revision,
                status: state,
                at_ms: now,
                due_at_ms: None,
                undecided_cause: cause,
            }
        };
        let moved = |p: &mut Pipeline, state: LifecycleStatus| moved_with(p, state, None);
        let mut m = meta("request.advance", Actor::broker(), now);
        m.correlation_id = Some(request_id.clone());
        let (m, mutations, what) = match p.stage.clone() {
            Stage::Received => {
                p.stage = Stage::Validating;
                (m, vec![moved(&mut p, status("validating"))], "validating")
            }
            Stage::Validating => {
                p.stage = Stage::ProvingClassification;
                let mutation = moved(&mut p, status("proving_classification"));
                (m, vec![mutation], "proving_classification")
            }
            Stage::ProvingClassification => {
                let proof_id = format!("{}-classification", p.request_id);
                let mut m = m;
                m.operation_type = "request.proof";
                m.causation_id = Some(proof_id.clone());
                if self.rng.chance(10) {
                    // The undecided end: the move states the broker's cause; the write is
                    // correlated to the request, as every move that states one is.
                    let cause = UndecidedCause::ALL[usize::try_from(
                        self.rng.below(UndecidedCause::ALL.len() as u64),
                    )
                    .unwrap()];
                    p.stage = Stage::Done;
                    self.undecided_with_cause += 1;
                    let mutation = moved_with(&mut p, status("undecided"), Some(cause));
                    (m, vec![mutation], "undecided")
                } else if self.rng.chance(20) {
                    let proof = self.proof(&p, &proof_id, "classification", "block");
                    p.stage = Stage::Done;
                    let blocked = contract(json!({
                        "state": "blocked",
                        "denial": ruling("recurring_mutation_unsupported", &proof_id)
                    }));
                    let mutations = vec![
                        Mutation::ProofRecorded { proof, at_ms: now },
                        moved(&mut p, blocked),
                    ];
                    (m, mutations, "blocked")
                } else {
                    let proof = self.proof(&p, &proof_id, "classification", "need_approval");
                    let expires = ts(now + 86_400_000);
                    let card = contract(json!({
                        "scope": scope(),
                        "proposed_duration": { "kind": "one_request" },
                        "insufficiency": ruling("owner_approval_required", &proof_id),
                        "expires_at": expires
                    }));
                    p.stage = Stage::PendingApproval;
                    let pending = contract(json!({
                        "state": "pending_approval",
                        "approval_expires_at": expires
                    }));
                    let mutations = vec![
                        Mutation::ProofRecorded { proof, at_ms: now },
                        Mutation::CardOpened {
                            request_id: request_id.clone(),
                            card,
                            opened_at_ms: now,
                        },
                        moved(&mut p, pending),
                    ];
                    (m, mutations, "pending_approval")
                }
            }
            Stage::PendingApproval => {
                let (mut m, answered) = self.command("owner.decide_request", "decide_request");
                m.correlation_id = Some(request_id.clone());
                let command_id = match &answered {
                    Mutation::CommandAnswered { row } => row.command_id.as_str().to_owned(),
                    _ => unreachable!(),
                };
                if self.rng.chance(25) {
                    let approval = contract(json!({
                        "decision": "deny",
                        "command_id": command_id,
                        "decided_at": ts(now),
                        "approval_digest": digest_of(&command_id),
                        "scope": null,
                        "duration": null,
                        "effective_request_digest": null,
                        "grant_id": null
                    }));
                    p.stage = Stage::Done;
                    let mutations = vec![
                        Mutation::ApprovalRecorded {
                            request_id: request_id.clone(),
                            approval,
                        },
                        moved(&mut p, status("denied")),
                        answered,
                    ];
                    (m, mutations, "denied")
                } else {
                    let grant_id = self.fresh("grant");
                    let approval = contract(json!({
                        "decision": "approve",
                        "command_id": command_id,
                        "decided_at": ts(now),
                        "approval_digest": digest_of(&command_id),
                        "scope": scope(),
                        "duration": { "kind": "one_request" },
                        "effective_request_digest": digest_of(&p.request_id),
                        "grant_id": grant_id
                    }));
                    let grant = contract(json!({
                        "grant_id": grant_id,
                        "revision": 1,
                        "agent_id": p.agent_id,
                        "purpose": "Block time to plan next week.",
                        "tools": ["fabric_calendar_propose_create"],
                        "scope": scope(),
                        "term": { "kind": "one_request", "action_digest": p.action.digest() },
                        "origin": { "request_id": p.request_id, "approval_digest": digest_of(&command_id) },
                        "policy": { "profile_id": "careful", "version": 1 },
                        "revocation_epoch": self.revocation_epoch,
                        "created_at": ts(now),
                        "ended": null
                    }));
                    p.grant_id = Some(grant_id.clone());
                    p.stage = Stage::Approved;
                    p.approved_path = true;
                    let mutations = vec![
                        Mutation::ApprovalRecorded {
                            request_id: request_id.clone(),
                            approval,
                        },
                        Mutation::GrantInserted { grant },
                        Mutation::ReliedGrantSet {
                            request_id: request_id.clone(),
                            grant_ids: vec![id(&grant_id)],
                        },
                        moved(&mut p, status("approved")),
                        answered,
                    ];
                    (m, mutations, "approved")
                }
            }
            Stage::Approved => {
                p.stage = Stage::ProvingFreshPermit;
                let mutation = moved(&mut p, status("proving_fresh_permit"));
                (m, vec![mutation], "proving_fresh_permit")
            }
            Stage::ProvingFreshPermit => {
                let proof_id = format!("{}-permit-1", p.request_id);
                let authorization_id = format!("authz-{}", p.request_id);
                let proof = self.proof(&p, &proof_id, "fresh_permit", "permit");
                let grant_id = p.grant_id.clone().unwrap();
                let record = AuthorizationRecord {
                    authorization: contract(json!({
                        "profile": "fabric-broker-authorization/1",
                        "authorization_id": authorization_id,
                        "request_id": p.request_id,
                        "agent_id": p.agent_id,
                        "action_digest": p.action.digest(),
                        "proof_id": proof_id,
                        "grant_id": grant_id,
                        "issued_at": ts(now),
                        "expires_at": ts(now + 60_000),
                        "lifetime_seconds": 60,
                        "consumed_at": null
                    })),
                    grant_ids: vec![id(&grant_id)],
                    attempt: 1,
                    run_id: self.run,
                    fence: self.fence(),
                    state: AuthorizationState::Issued,
                };
                p.fresh_proof_id = Some(proof_id.clone());
                p.authorization_id = Some(authorization_id);
                p.stage = Stage::Queued;
                let mut m = m;
                m.operation_type = "request.authorize";
                m.causation_id = Some(proof_id);
                let mutations = vec![
                    Mutation::ProofRecorded { proof, at_ms: now },
                    Mutation::AuthorizationIssued { record },
                    moved(&mut p, status("queued")),
                ];
                (m, mutations, "queued")
            }
            Stage::Queued => {
                let effect_id = format!("effect-{}", p.request_id);
                let authorization_id = p.authorization_id.clone().unwrap();
                let effect = EffectRow {
                    effect_id: id(&effect_id),
                    request_id: request_id.clone(),
                    tool: p.action.tool(),
                    account_id: id("acct-1"),
                    calendar_id: id("cal-primary"),
                    action_digest: p.action.digest(),
                    provider_event_id: Some(format!("evt{}", p.request_id.replace('-', ""))),
                    created_at_ms: now,
                    attempts_started: 0,
                    first_attempt_at_ms: None,
                };
                let attempt = AttemptRow {
                    effect_id: id(&effect_id),
                    authorization_id: id(&authorization_id),
                    attempt: contract(json!({
                        "number": 1,
                        "proof_id": p.fresh_proof_id.clone().unwrap(),
                        "started_at": ts(now),
                        "finished_at": null,
                        "outcome": null
                    })),
                };
                p.effect_id = Some(effect_id);
                p.stage = Stage::Dispatching;
                let mut m = m;
                m.operation_type = "request.dispatch";
                let mutations = vec![
                    Mutation::AuthorizationConsumed {
                        authorization_id: id(&authorization_id),
                        at_ms: now,
                    },
                    Mutation::EffectReserved { effect },
                    Mutation::DispatchIntent { attempt },
                    moved(&mut p, status("dispatching")),
                ];
                (m, mutations, "dispatching")
            }
            Stage::Dispatching => {
                p.stage = Stage::Done;
                let mut m = m;
                m.operation_type = "request.attempt";
                let grant_id = p.grant_id.clone().unwrap();
                let mutations = vec![
                    Mutation::AttemptFinished {
                        effect_id: id(p.effect_id.as_deref().unwrap()),
                        number: 1,
                        finished_at_ms: now,
                        outcome: AttemptOutcome::Applied,
                    },
                    moved(&mut p, status("applied")),
                    Mutation::GrantEnded {
                        grant_id: id(&grant_id),
                        revision: 2,
                        end: GrantEnd {
                            at: ts(now),
                            reason: GrantEndReason::Consumed,
                        },
                    },
                ];
                (m, mutations, "applied")
            }
            Stage::Done => unreachable!("a done request is not advanced"),
        };
        if p.stage == Stage::Done && p.approved_path {
            self.completed_approved += 1;
        }
        let what = format!("{} -> {what}", p.request_id);
        self.pipelines[index] = p;
        self.step(m, mutations, what)
    }

    fn open_pipeline(&mut self) -> Option<usize> {
        let open: Vec<usize> = (0..self.pipelines.len())
            .filter(|&i| self.pipelines[i].stage != Stage::Done)
            .collect();
        if open.is_empty() {
            return None;
        }
        Some(open[usize::try_from(self.rng.below(open.len() as u64)).unwrap()])
    }
}

fn commit(store: &mut Store, step: Step) {
    let ops: Vec<String> = step
        .mutations
        .iter()
        .map(|m| serde_json::to_value(m).unwrap()["op"].to_string())
        .collect();
    if let Err(e) = apply_all(store, step.meta, step.mutations) {
        panic!(
            "the write `{}` ({}) was refused: {e:?}",
            step.what,
            ops.join(", ")
        );
    }
}

/// Advances request `index` one step; an issued authorization is consumed by the dispatch
/// that follows it at once, as the core does, well inside its 60 s lifetime.
fn advance(store: &mut Store, model: &mut Model, index: usize) {
    let step = model.advance(index);
    commit(store, step);
    if model.pipelines[index].stage == Stage::Queued {
        let step = model.advance(index);
        commit(store, step);
    }
}

/// Runs one seeded sequence of `steps` random writes on a new store in `data_dir`, then drives
/// every open request to its end.
fn run_sequence(data_dir: &Path, seed: u64, steps: usize) -> Store {
    let mut store = new_store(data_dir);
    let mut model = Model::new(seed);
    // A prologue that writes each simple kind once, so no table depends on chance.
    commit(&mut store, model.add_agent());
    commit(&mut store, model.put_account());
    let session = model.start_session().unwrap();
    commit(&mut store, session);
    commit(&mut store, model.save_preferences());
    commit(&mut store, model.save_group());
    commit(&mut store, model.raise_revocation());
    commit(&mut store, model.save_profile(true));
    for _ in 0..steps {
        let step = match model.rng.below(16) {
            0 => Some(model.add_agent()),
            1 => model.pause_agent(),
            2 => model.start_session(),
            3 => model.end_session(),
            4 => Some(model.raise_revocation()),
            5 => Some(model.save_preferences()),
            6 => Some(model.save_group()),
            7 => model.delete_group(),
            8 => Some(model.put_account()),
            9 => Some(model.save_profile(false)),
            10 => {
                model.restart(&mut store);
                None
            }
            11 | 12 => model.insert_request(),
            _ => {
                if let Some(index) = model.open_pipeline() {
                    advance(&mut store, &mut model, index);
                }
                None
            }
        };
        if let Some(step) = step {
            commit(&mut store, step);
        }
    }
    // Every table is touched: at least one request goes the whole approved way, and at least one
    // ends undecided with its cause.
    while model.completed_approved == 0 || model.undecided_with_cause == 0 {
        let step = model.insert_request().unwrap();
        commit(&mut store, step);
        let index = model.pipelines.len() - 1;
        while model.pipelines[index].stage != Stage::Done {
            advance(&mut store, &mut model, index);
        }
    }
    while let Some(index) = model.open_pipeline() {
        advance(&mut store, &mut model, index);
    }
    store
}

/// s13 (4, D3): for seeded random sequences of writes that touch every projection table, the
/// projection rebuilt from the ledger equals the stored one, before and after a reopen.
#[test]
fn replay_rebuilds_every_projection_table() {
    for seed in 1..=6_u64 {
        let dir = TempDir::new(&format!("s13-{seed}"));
        let data = dir.data_dir();
        let store = run_sequence(&data, seed, 120);
        verify(&store);
        store.close().unwrap();
        for table in PROJECTION_TABLES {
            assert!(
                count_rows(&data, table) > 0,
                "seed {seed}: the sequence left `{table}` empty"
            );
        }
        let store = open(&data, T0 + 100_000_000);
        verify(&store);
    }
}

/// s13 (4): replay compares the projection; a projection row changed outside `apply` (by a
/// second connection with the key) makes `verify_replay` fail, while the chain still verifies.
#[test]
fn replay_detects_a_projection_changed_outside_apply() {
    for (table, sql) in [
        ("agents", "UPDATE agents SET paused = 1 - paused"),
        (
            "preferences",
            "UPDATE preferences SET revision = revision + 7",
        ),
        ("requests", "UPDATE requests SET revision = revision + 1"),
        ("grants", "DELETE FROM grants"),
    ] {
        let dir = TempDir::new(&format!("s13-tamper-{table}"));
        let data = dir.data_dir();
        let store = run_sequence(&data, 42, 40);
        verify(&store);
        raw(&data).execute_batch(sql).unwrap();
        store
            .verify_chain()
            .unwrap_or_else(|e| panic!("{table}: the chain failed: {e:?}"));
        assert!(
            store.verify_replay().is_err(),
            "{table}: a projection changed outside apply passed replay"
        );
    }
}
