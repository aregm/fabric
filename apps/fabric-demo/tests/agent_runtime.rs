//! The agent runtime's control flow (task T6.1), against a stand-in for the host and calendar
//! service that replays fixed answers. The verdicts here are test inputs, not policy: every
//! policy decision in the demo comes from Isabelle (D28), as the live tests check.

use std::collections::VecDeque;
use std::sync::Mutex;

use fabric_demo::agent::turn::FreeBusyView;
use fabric_demo::agent::{
    Agent, Faults, Outcome, Plan, PlanAction, PlanSet, Planner, Scripted, Slot, World,
};
use fabric_demo::audit::AuditLog;
use fabric_demo::calendar::Busy;
use fabric_demo::client::CallError;
use fabric_demo::host::{
    AgentState, AuthorizeAnswer, Delivery, Event, NegotiationRecord, OwnerRequest, ScreenAnswer,
};
use fabric_policy::time::parse_utc;
use fabric_policy::{Action, Denial, Request, Role, Rule, Verdict};
use serde_json::{Value, json};

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

fn deny(rule: Rule) -> Verdict {
    Verdict::Deny {
        denials: vec![Denial {
            rule,
            detail: "scripted".to_owned(),
        }],
    }
}

fn state(with_request: bool) -> AgentState {
    AgentState {
        person: "alice".to_owned(),
        peer: "bob".to_owned(),
        now_ms: at("2027-01-11T08:00Z"),
        request: with_request.then(|| OwnerRequest {
            request: Request {
                peer: "bob".to_owned(),
                duration_ms: 30 * 60 * 1_000,
                window_start_ms: at("2027-01-11T08:00Z"),
                window_end_ms: at("2027-01-18T08:00Z"),
                opened_at_ms: at("2027-01-11T08:00Z"),
                expires_at_ms: at("2027-01-12T08:00Z"),
            },
            preference: "afternoons".to_owned(),
        }),
        link: "idle".to_owned(),
        negotiation: NegotiationRecord::new(Role::Coordinator, Some("n-1".to_owned())),
        owner_decisions: Vec::new(),
    }
}

/// Replays queued answers and records what the agent asked for.
#[derive(Default)]
struct Script {
    state: Mutex<Option<AgentState>>,
    deliveries: Mutex<VecDeque<Vec<Delivery>>>,
    screens: Mutex<VecDeque<Vec<Verdict>>>,
    authorizations: Mutex<VecDeque<Verdict>>,
    executions: Mutex<VecDeque<Result<Value, CallError>>>,
    screened: Mutex<Vec<Vec<Action>>>,
    authorized: Mutex<Vec<Action>>,
    executed: Mutex<Vec<(Action, Vec<u8>)>>,
}

impl World for Script {
    async fn state(&self) -> Result<AgentState, CallError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| state(false)))
    }

    async fn take_faults(&self) -> Result<Faults, CallError> {
        Ok(Faults::default())
    }

    async fn deliveries(&self, _after: u64, _wait_ms: u64) -> Result<Vec<Delivery>, CallError> {
        Ok(self
            .deliveries
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default())
    }

    async fn screen(&self, actions: &[Action]) -> Result<ScreenAnswer, CallError> {
        self.screened.lock().unwrap().push(actions.to_vec());
        let verdicts = self
            .screens
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![Verdict::Permit; actions.len()]);
        assert_eq!(verdicts.len(), actions.len(), "one verdict per action");
        Ok(ScreenAnswer {
            theory: "Check_000001".to_owned(),
            proof_id: "00".repeat(32),
            seconds: 0.0,
            verdicts,
        })
    }

    async fn authorize(&self, action: &Action) -> Result<AuthorizeAnswer, CallError> {
        self.authorized.lock().unwrap().push(action.clone());
        let verdict = self
            .authorizations
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Verdict::Permit);
        let count = self.authorized.lock().unwrap().len();
        Ok(AuthorizeAnswer {
            theory: "Check_000002".to_owned(),
            proof_id: "11".repeat(32),
            seconds: 0.0,
            token: verdict.is_permit().then(|| format!("{count:02x}")),
            verdict,
        })
    }

    async fn execute(&self, action: &Action, token: &[u8]) -> Result<Value, CallError> {
        self.executed
            .lock()
            .unwrap()
            .push((action.clone(), token.to_vec()));
        self.executions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(json!({ "ok": true })))
    }
}

fn plan(label: &str, actions: Vec<PlanAction>) -> Plan {
    Plan {
        label: label.to_owned(),
        rationale: "test".to_owned(),
        actions,
    }
}

fn set(plans: Vec<Plan>) -> PlanSet {
    PlanSet { plans }
}

fn read(start: &str, end: &str) -> PlanAction {
    PlanAction::ReadFreeBusy {
        start: start.to_owned(),
        end: end.to_owned(),
    }
}

fn propose() -> PlanAction {
    PlanAction::SendPropose {
        round: 1,
        candidates: vec![Slot {
            start: "2027-01-12T14:00:00Z".to_owned(),
            minutes: 30,
        }],
        note: "afternoons suit me".to_owned(),
    }
}

fn handshake() -> PlanAction {
    PlanAction::Handshake {
        peer: "bob".to_owned(),
    }
}

fn agent(script: Script, plans: Vec<PlanSet>) -> Agent<Script> {
    *script.state.lock().unwrap() = Some(state(true));
    Agent::new(
        script,
        Planner::Scripted(Scripted::fixed(plans)),
        "alice",
        AuditLog::disabled(),
    )
}

#[tokio::test]
async fn the_first_fully_permitted_plan_runs_and_a_read_ends_it() {
    let script = Script::default();
    script.screens.lock().unwrap().push_back(vec![
        deny(Rule::ReadWindow),
        Verdict::Permit,
        Verdict::Permit,
    ]);
    script.executions.lock().unwrap().push_back(Ok(json!({
        "busy": [{ "start_ms": at("2027-01-12T09:00Z"), "end_ms": at("2027-01-12T10:00Z") }]
    })));
    let mut agent = agent(
        script,
        vec![set(vec![
            plan(
                "too wide",
                vec![read("2027-01-01T00:00:00Z", "2027-02-01T00:00:00Z")],
            ),
            plan(
                "the window",
                vec![
                    read("2027-01-11T08:00:00Z", "2027-01-18T08:00:00Z"),
                    handshake(),
                ],
            ),
        ])],
    );
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::ReadMore));

    let executed = agent.world().executed.lock().unwrap().clone();
    assert_eq!(
        executed.len(),
        1,
        "the read ended the plan before the handshake"
    );
    assert!(matches!(executed[0].0, Action::ReadFreeBusy { .. }));
    assert_eq!(
        executed[0].1,
        vec![1],
        "the token from the authorize answer"
    );
    assert_eq!(
        agent.memory().free_busy,
        Some(FreeBusyView::new(
            at("2027-01-11T08:00Z"),
            at("2027-01-18T08:00Z"),
            &[Busy {
                start_ms: at("2027-01-12T09:00Z"),
                end_ms: at("2027-01-12T10:00Z"),
            }]
        ))
    );
    let screened = agent.world().screened.lock().unwrap().clone();
    assert_eq!(screened.len(), 1, "one screening check for every plan");
    assert_eq!(screened[0].len(), 3);
}

#[tokio::test]
async fn denials_come_back_for_at_most_three_replans() {
    let script = Script::default();
    for _ in 0..4 {
        script
            .screens
            .lock()
            .unwrap()
            .push_back(vec![deny(Rule::CandCount)]);
    }
    let attempts = (0..4)
        .map(|_| set(vec![plan("propose", vec![propose()])]))
        .collect();
    let mut agent = agent(script, attempts);
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::GaveUp));
    assert_eq!(agent.world().screened.lock().unwrap().len(), 4);
    assert!(agent.world().authorized.lock().unwrap().is_empty());
    let denials = &agent.memory().denials;
    assert_eq!(denials.len(), 4);
    assert_eq!(denials[0]["denial"]["reasons"][0], "cand_count: scripted");
}

#[tokio::test]
async fn a_denial_when_authorizing_stops_the_plan_and_replans() {
    let script = Script::default();
    script
        .authorizations
        .lock()
        .unwrap()
        .extend([Verdict::Permit, deny(Rule::WellFormed)]);
    let mut agent = agent(
        script,
        vec![
            set(vec![plan("call and propose", vec![handshake(), propose()])]),
            set(vec![plan("wait", Vec::new())]),
        ],
    );
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));
    let executed = agent.world().executed.lock().unwrap().clone();
    assert_eq!(executed.len(), 1);
    assert!(matches!(executed[0].0, Action::Handshake { .. }));
    assert_eq!(agent.memory().denials[0]["at"], "authorize");
    assert_eq!(agent.memory().past_actions[0]["kind"], "handshake");
}

#[tokio::test]
async fn a_plan_without_actions_waits() {
    let mut agent = agent(Script::default(), vec![set(vec![plan("wait", Vec::new())])]);
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));
    assert!(agent.world().screened.lock().unwrap().is_empty());
    assert!(agent.world().authorized.lock().unwrap().is_empty());
}

#[tokio::test]
async fn turns_start_only_when_something_new_happens() {
    let script = Script::default();
    let mut agent = Agent::new(
        script,
        Planner::Scripted(Scripted::fixed(Vec::new())),
        "alice",
        AuditLog::disabled(),
    );
    assert_eq!(agent.step(0).await.unwrap(), None, "no request yet");
    *agent.world().state.lock().unwrap() = Some(state(true));
    assert_eq!(
        agent.step(0).await.unwrap(),
        Some(Outcome::Waiting),
        "the request arrived"
    );
    assert_eq!(agent.step(0).await.unwrap(), None, "nothing new");
    agent
        .world()
        .deliveries
        .lock()
        .unwrap()
        .push_back(vec![Delivery {
            seq: 1,
            at_ms: 0,
            event: Event::PeerCalling {
                peer: "bob".to_owned(),
            },
        }]);
    assert_eq!(
        agent.step(0).await.unwrap(),
        Some(Outcome::Waiting),
        "a delivery"
    );
    assert_eq!(
        agent.memory().inbound[0],
        json!({ "type": "calling", "from": "bob" })
    );
    assert_eq!(agent.memory().delivery_cursor, 1);
}

#[tokio::test]
async fn screening_switches_distort_the_first_attempt_only() {
    // The event-details plan, the widened read, and a proposal padded to 5 are denied, as
    // Isabelle would deny them; the re-plan is screened undistorted.
    let script = Script::default();
    script.screens.lock().unwrap().extend([
        vec![
            deny(Rule::ViewDetailsNever),
            deny(Rule::ReadWindow),
            deny(Rule::CandCount),
        ],
        vec![Verdict::Permit],
    ]);
    let mut agent = agent(
        script,
        vec![
            set(vec![
                plan(
                    "read",
                    vec![read("2027-01-11T08:00:00Z", "2027-01-18T08:00:00Z")],
                ),
                plan("propose", vec![propose()]),
            ]),
            set(vec![plan("call", vec![handshake()])]),
        ],
    );
    agent.inject(Faults {
        over_broad_read: true,
        too_many_candidates: true,
        ..Faults::default()
    });
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));

    let screened = agent.world().screened.lock().unwrap().clone();
    assert!(matches!(screened[0][0], Action::ReadEventDetails { .. }));
    let Action::ReadFreeBusy { end_ms, .. } = screened[0][1] else {
        panic!("the widened read");
    };
    assert!(end_ms > at("2027-01-18T08:00Z"));
    let Action::SendPropose { candidates, .. } = &screened[0][2] else {
        panic!("the padded proposal");
    };
    assert_eq!(candidates.len(), 5);
    assert_eq!(screened[1].len(), 1);
    assert!(matches!(screened[1][0], Action::Handshake { .. }));
}

#[tokio::test]
async fn a_write_without_consent_is_put_first_and_its_denial_replans() {
    let script = Script::default();
    script
        .authorizations
        .lock()
        .unwrap()
        .push_back(deny(Rule::Consent));
    let mut agent = agent(
        script,
        vec![
            set(vec![plan("call", vec![handshake()])]),
            set(vec![plan("call again", vec![handshake()])]),
        ],
    );
    agent.inject(Faults {
        write_without_consent: true,
        ..Faults::default()
    });
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));
    let authorized = agent.world().authorized.lock().unwrap().clone();
    assert!(matches!(authorized[0], Action::AddMeeting { .. }));
    assert!(matches!(authorized[1], Action::Handshake { .. }));
    assert_eq!(agent.memory().denials[0]["reasons"][0], "consent: scripted");
    assert_eq!(agent.world().executed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_replayed_token_goes_out_once_more_after_its_action_ran() {
    let mut agent = agent(
        Script::default(),
        vec![set(vec![plan("call", vec![handshake()])])],
    );
    agent.inject(Faults {
        replay_token: true,
        ..Faults::default()
    });
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));
    let executed = agent.world().executed.lock().unwrap().clone();
    assert_eq!(executed.len(), 2);
    assert_eq!(
        executed[0], executed[1],
        "the same request, with the same token"
    );
}

#[tokio::test]
async fn a_switch_with_nothing_to_act_on_stays_armed_without_turns_of_its_own() {
    let mut agent = agent(
        Script::default(),
        vec![
            set(vec![plan("wait", Vec::new())]),
            set(vec![plan("call", vec![handshake()])]),
        ],
    );
    agent.inject(Faults {
        replay_token: true,
        too_many_candidates: true,
        ..Faults::default()
    });
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));
    assert!(agent.world().executed.lock().unwrap().is_empty());
    assert_eq!(
        agent.step(0).await.unwrap(),
        None,
        "armed switches start no turn"
    );

    agent
        .world()
        .deliveries
        .lock()
        .unwrap()
        .push_back(vec![Delivery {
            seq: 1,
            at_ms: 0,
            event: Event::PeerCalling {
                peer: "bob".to_owned(),
            },
        }]);
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::Waiting));
    let executed = agent.world().executed.lock().unwrap().clone();
    assert_eq!(executed.len(), 2, "the handshake, then its replay");
    assert_eq!(executed[0], executed[1]);
    assert_eq!(
        agent.pending_faults(),
        Faults {
            too_many_candidates: true,
            ..Faults::default()
        },
        "the replay fired; no proposal came up, so that switch is still armed"
    );
}

#[tokio::test]
async fn a_token_moved_to_another_slot_is_tried_then_replaced() {
    let script = Script::default();
    script.executions.lock().unwrap().extend([
        Err(CallError::Refused {
            status: 403,
            error: "token_argument_mismatch".to_owned(),
            detail: "scripted".to_owned(),
        }),
        Ok(json!({ "busy": [] })),
    ]);
    let mut agent = agent(
        script,
        vec![set(vec![plan(
            "read",
            vec![read("2027-01-11T08:00:00Z", "2027-01-18T08:00:00Z")],
        )])],
    );
    agent.inject(Faults {
        token_for_different_slot: true,
        ..Faults::default()
    });
    assert_eq!(agent.step(0).await.unwrap(), Some(Outcome::ReadMore));
    let executed = agent.world().executed.lock().unwrap().clone();
    let Action::ReadFreeBusy { start_ms, .. } = executed[0].0 else {
        panic!("a read");
    };
    assert_eq!(start_ms, at("2027-01-11T08:15Z"), "moved by 15 minutes");
    assert_ne!(
        executed[0].1, executed[1].1,
        "a fresh token for the real read"
    );
    assert_eq!(agent.world().authorized.lock().unwrap().len(), 2);
}
