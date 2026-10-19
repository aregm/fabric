//! The policy test corpus (task T2.4): permit and deny cases for every rule, decided by a live
//! Isabelle session. It needs `scripts/setup-demo.ps1`, so it is ignored by default:
//!
//!   .\scripts\run.ps1 cargo test -p fabric-policy --test corpus -- --ignored --nocapture
//!
//! A permit case passes when every rule is proven to hold. A deny case passes when exactly the
//! listed rules are proven to fail. `view_details_never` can never hold for the one action it
//! governs, so its "permit" side is every other action, where it holds trivially.

use std::path::Path;
use std::time::Instant;

use fabric_policy::time::parse_utc;
use fabric_policy::{
    Action, Answer, Candidate, CloseReason, Consent, Context, Decision, Gate, GateConfig,
    Negotiation, Parameters, Request, Role, Rule, RuleOutcome, Verdict,
};

const HALF_HOUR: i64 = 30 * 60 * 1_000;

fn at(text: &str) -> i64 {
    parse_utc(text).expect("fixture times are valid")
}

fn candidate(id: &str, start: &str) -> Candidate {
    Candidate {
        id: id.to_owned(),
        start_ms: at(start),
        length_ms: HALF_HOUR,
    }
}

fn proposal() -> Vec<Candidate> {
    vec![
        candidate("c1", "2027-01-12T14:00Z"),
        candidate("c2", "2027-01-13T15:00Z"),
        candidate("c3", "2027-01-14T14:30Z"),
    ]
}

fn request(peer: &str) -> Request {
    Request {
        peer: peer.to_owned(),
        duration_ms: HALF_HOUR,
        window_start_ms: at("2027-01-11T08:00Z"),
        window_end_ms: at("2027-01-18T08:00Z"),
        opened_at_ms: at("2027-01-11T08:00Z"),
        expires_at_ms: at("2027-01-12T08:00Z"),
    }
}

fn coordinator() -> Context {
    Context {
        owner: "alice".to_owned(),
        now_ms: at("2027-01-11T08:00Z"),
        request: request("bob"),
        negotiation: Negotiation {
            id: "n1".to_owned(),
            role: Role::Coordinator,
            round: 0,
            handshake_done: true,
            candidates: Vec::new(),
            mutual: Vec::new(),
        },
        consents: Vec::new(),
    }
}

fn invitee() -> Context {
    Context {
        owner: "bob".to_owned(),
        now_ms: at("2027-01-11T08:00Z"),
        request: request("alice"),
        negotiation: Negotiation {
            id: "n1".to_owned(),
            role: Role::Invitee,
            round: 1,
            handshake_done: true,
            candidates: proposal(),
            mutual: Vec::new(),
        },
        consents: Vec::new(),
    }
}

fn consent(signer: &str, start: &str, yes: bool, expires: &str) -> Consent {
    Consent {
        signer: signer.to_owned(),
        negotiation: "n1".to_owned(),
        start_ms: at(start),
        length_ms: HALF_HOUR,
        yes,
        expires_ms: at(expires),
    }
}

fn deciding(consent_expires: &str) -> Context {
    let mut context = coordinator();
    context.negotiation.round = 1;
    context.negotiation.candidates = proposal();
    context.negotiation.mutual = vec!["c2".to_owned(), "c3".to_owned()];
    context.consents = vec![
        consent("alice", "2027-01-13T15:00Z", true, consent_expires),
        consent("alice", "2027-01-14T14:30Z", true, consent_expires),
        consent("bob", "2027-01-13T15:00Z", false, consent_expires),
        consent("bob", "2027-01-14T14:30Z", true, consent_expires),
    ];
    context
}

fn propose(round: u32, candidates: Vec<Candidate>, note: &str) -> Action {
    Action::SendPropose {
        negotiation: "n1".to_owned(),
        round,
        candidates,
        note: note.to_owned(),
    }
}

fn respond(round: u32, answers: &[(&str, bool)]) -> Action {
    Action::SendRespond {
        negotiation: "n1".to_owned(),
        round,
        answers: answers
            .iter()
            .map(|(id, available)| Answer {
                candidate_id: (*id).to_owned(),
                available: *available,
            })
            .collect(),
        note: String::new(),
    }
}

fn decide(decisions: &[(&str, bool)]) -> Action {
    Action::SendDecide {
        negotiation: "n1".to_owned(),
        round: 1,
        decisions: decisions
            .iter()
            .map(|(id, accept)| Decision {
                candidate_id: (*id).to_owned(),
                accept: *accept,
            })
            .collect(),
        note: String::new(),
    }
}

fn add_meeting(start: &str, length_ms: i64) -> Action {
    Action::AddMeeting {
        negotiation: "n1".to_owned(),
        start_ms: at(start),
        length_ms,
    }
}

fn read(start: &str, end: &str) -> Action {
    Action::ReadFreeBusy {
        start_ms: at(start),
        end_ms: at(end),
    }
}

struct Case {
    name: &'static str,
    action: Action,
    deny: &'static [Rule],
}

fn case(name: &'static str, action: Action, deny: &'static [Rule]) -> Case {
    Case { name, action, deny }
}

fn groups() -> Vec<(&'static str, Context, Vec<Case>)> {
    use Rule::{
        AnswerCount, CandCount, Consent as ConsentRule, ConsentLifetime, Grid, HandshakeTarget,
        Horizon, Note, ReadWindow, RoundLimit, ViewDetailsNever, WellFormed,
    };

    let mut expired = coordinator();
    expired.now_ms = at("2027-01-12T09:00Z");
    let mut long_negotiation = coordinator();
    long_negotiation.request.expires_at_ms = at("2027-01-13T08:00Z");
    let mut wide_window = coordinator();
    wide_window.request.window_end_ms = at("2027-02-01T08:00Z");
    let mut third_round = coordinator();
    third_round.negotiation.round = 3;
    let mut no_session = coordinator();
    no_session.negotiation.handshake_done = false;
    let mut long_meeting = candidate("c1", "2027-01-12T14:00Z");
    long_meeting.length_ms = 45 * 60 * 1_000;

    vec![
        (
            "Alice before her first proposal",
            coordinator(),
            vec![
                case(
                    "read inside the window",
                    read("2027-01-11T08:00Z", "2027-01-18T08:00Z"),
                    &[],
                ),
                case(
                    "read past the window",
                    read("2027-01-11T08:00Z", "2027-01-25T08:00Z"),
                    &[ReadWindow],
                ),
                case(
                    "read event details",
                    Action::ReadEventDetails {
                        start_ms: at("2027-01-11T08:00Z"),
                        end_ms: at("2027-01-18T08:00Z"),
                    },
                    &[ViewDetailsNever],
                ),
                case(
                    "handshake with the requested peer",
                    Action::Handshake {
                        peer: "bob".to_owned(),
                    },
                    &[],
                ),
                case(
                    "handshake with someone else",
                    Action::Handshake {
                        peer: "carol".to_owned(),
                    },
                    &[HandshakeTarget],
                ),
                case(
                    "propose three good candidates",
                    propose(1, proposal(), "afternoons suit me"),
                    &[],
                ),
                case(
                    "propose five candidates",
                    propose(
                        1,
                        vec![
                            candidate("c1", "2027-01-12T14:00Z"),
                            candidate("c2", "2027-01-13T15:00Z"),
                            candidate("c3", "2027-01-14T14:30Z"),
                            candidate("c4", "2027-01-15T13:00Z"),
                            candidate("c5", "2027-01-12T16:00Z"),
                        ],
                        "",
                    ),
                    &[CandCount],
                ),
                case(
                    "propose no candidates",
                    propose(1, Vec::new(), ""),
                    &[CandCount],
                ),
                case(
                    "propose after the window",
                    propose(1, vec![candidate("c1", "2027-01-18T12:00Z")], ""),
                    &[Horizon],
                ),
                case(
                    "propose in the past",
                    propose(1, vec![candidate("c1", "2027-01-11T07:00Z")], ""),
                    &[Horizon],
                ),
                case(
                    "propose off the grid",
                    propose(1, vec![candidate("c1", "2027-01-12T14:07Z")], ""),
                    &[Grid],
                ),
                case(
                    "propose the wrong length",
                    propose(1, vec![long_meeting], ""),
                    &[Grid],
                ),
                case(
                    "note with digits",
                    propose(1, proposal(), "meet at 2pm"),
                    &[Note],
                ),
                case(
                    "note with non-ASCII text",
                    propose(1, proposal(), "caf\u{e9}"),
                    &[Note],
                ),
                case(
                    "note over 200 characters",
                    propose(1, proposal(), &"a".repeat(201)),
                    &[Note],
                ),
                case(
                    "repeated candidate IDs",
                    propose(
                        1,
                        vec![
                            candidate("c1", "2027-01-12T14:00Z"),
                            candidate("c1", "2027-01-13T15:00Z"),
                        ],
                        "",
                    ),
                    &[WellFormed],
                ),
                case(
                    "propose for another negotiation",
                    Action::SendPropose {
                        negotiation: "n2".to_owned(),
                        round: 1,
                        candidates: proposal(),
                        note: String::new(),
                    },
                    &[WellFormed],
                ),
                case(
                    "propose round two first",
                    propose(2, proposal(), ""),
                    &[WellFormed],
                ),
                case(
                    "add a meeting without consent",
                    add_meeting("2027-01-14T14:30Z", HALF_HOUR),
                    &[ConsentRule],
                ),
            ],
        ),
        (
            "Alice after three rounds",
            third_round,
            vec![case(
                "propose a fourth round",
                propose(4, proposal(), ""),
                &[RoundLimit],
            )],
        ),
        (
            "Alice without an open session",
            no_session,
            vec![case(
                "propose before the handshake",
                propose(1, proposal(), ""),
                &[WellFormed],
            )],
        ),
        (
            "Alice after the negotiation expired",
            expired,
            vec![case(
                "propose after expiry",
                propose(1, vec![candidate("c3", "2027-01-14T14:30Z")], ""),
                &[ConsentLifetime],
            )],
        ),
        (
            "Alice with a 48-hour negotiation",
            long_negotiation,
            vec![case(
                "propose in a negotiation over a day",
                propose(1, proposal(), ""),
                &[ConsentLifetime],
            )],
        ),
        (
            "Alice with a 21-day window",
            wide_window,
            vec![case(
                "propose in a window over fourteen days",
                propose(1, proposal(), ""),
                &[Horizon],
            )],
        ),
        (
            "Bob with Alice's proposal",
            invitee(),
            vec![
                case(
                    "answer every candidate in order",
                    respond(1, &[("c1", false), ("c2", true), ("c3", true)]),
                    &[],
                ),
                case(
                    "answer out of order",
                    respond(1, &[("c2", true), ("c1", false), ("c3", true)]),
                    &[WellFormed],
                ),
                case(
                    "answer four candidates",
                    respond(
                        1,
                        &[("c1", false), ("c2", true), ("c3", true), ("c4", true)],
                    ),
                    &[AnswerCount, WellFormed],
                ),
                case(
                    "answer for the wrong round",
                    respond(2, &[("c1", false), ("c2", true), ("c3", true)]),
                    &[WellFormed],
                ),
                case(
                    "the invitee proposes",
                    propose(2, proposal(), ""),
                    &[WellFormed],
                ),
                case(
                    "close the negotiation",
                    Action::SendClose {
                        negotiation: "n1".to_owned(),
                        reason: CloseReason::NoMatch,
                        note: "no luck this week".to_owned(),
                    },
                    &[],
                ),
            ],
        ),
        (
            "Alice with both owners' decisions",
            deciding("2027-01-12T08:00Z"),
            vec![
                case(
                    "send the owner's decisions",
                    decide(&[("c2", true), ("c3", true)]),
                    &[],
                ),
                case(
                    "send a decision the owner did not make",
                    decide(&[("c2", false), ("c3", true)]),
                    &[WellFormed],
                ),
                case(
                    "skip a mutual option",
                    decide(&[("c3", true)]),
                    &[WellFormed],
                ),
                case(
                    "add the meeting both accepted",
                    add_meeting("2027-01-14T14:30Z", HALF_HOUR),
                    &[],
                ),
                case(
                    "add a meeting one owner declined",
                    add_meeting("2027-01-13T15:00Z", HALF_HOUR),
                    &[ConsentRule],
                ),
                case(
                    "add the right meeting at the wrong length",
                    add_meeting("2027-01-14T14:30Z", 45 * 60 * 1_000),
                    &[ConsentRule],
                ),
                case(
                    "close as agreed",
                    Action::SendClose {
                        negotiation: "n1".to_owned(),
                        reason: CloseReason::Agreed,
                        note: "thanks".to_owned(),
                    },
                    &[],
                ),
                case(
                    "close another negotiation",
                    Action::SendClose {
                        negotiation: "n9".to_owned(),
                        reason: CloseReason::Agreed,
                        note: String::new(),
                    },
                    &[WellFormed],
                ),
            ],
        ),
        (
            "Alice after the consents expired",
            deciding("2027-01-11T08:00Z"),
            vec![case(
                "add a meeting after consent expired",
                add_meeting("2027-01-14T14:30Z", HALF_HOUR),
                &[ConsentRule],
            )],
        ),
    ]
}

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the repository root")
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.ps1; run with --ignored"]
fn every_rule_is_decided_as_expected_by_isabelle() {
    let mut config = GateConfig::for_repository(repository());
    config.work_dir = repository().join(".local").join("policy-corpus");
    config.server_name = "fabric-policy-corpus".to_owned();
    config.audit_log = Some(config.work_dir.join("audit.jsonl"));

    let started = Instant::now();
    let mut gate = Gate::start(config).expect("the gate starts");
    eprintln!("gate started in {:.1}s", started.elapsed().as_secs_f64());
    assert_eq!(
        gate.parameters(),
        &Parameters::DEFAULT,
        "the theory's parameters"
    );

    let mut failures = Vec::new();
    let mut checked = 0;
    let mut covered: Vec<(Rule, bool)> = Vec::new();
    for (group, context, cases) in groups() {
        let actions: Vec<Action> = cases.iter().map(|case| case.action.clone()).collect();
        let report = gate.screen(&context, &actions).expect("the check runs");
        eprintln!(
            "\n{group}: {} actions in {:.2}s (proof {})",
            actions.len(),
            report.seconds,
            &report.proof_id[..12]
        );
        for (case, verdict) in cases.iter().zip(&report.verdicts) {
            checked += 1;
            let failed: Vec<Rule> = verdict
                .rules
                .iter()
                .filter(|result| result.outcome == RuleOutcome::Fails)
                .map(|result| result.rule)
                .collect();
            let as_expected = match &verdict.verdict {
                Verdict::Permit => case.deny.is_empty(),
                Verdict::Deny { .. } => failed.as_slice() == case.deny,
                Verdict::Undecided { .. } => false,
            };
            let shown = match &verdict.verdict {
                Verdict::Permit => "permit".to_owned(),
                other => other.summary(),
            };
            eprintln!(
                "  {} {:<42} {shown}",
                if as_expected { "ok " } else { "BAD" },
                case.name
            );
            if !as_expected {
                failures.push(format!(
                    "{group} / {}: expected deny {:?}, got {shown}",
                    case.name, case.deny
                ));
            }
            for rule in case.deny {
                covered.push((*rule, false));
            }
            if case.deny.is_empty() {
                covered.extend(
                    verdict
                        .rules
                        .iter()
                        .filter(|result| result.outcome == RuleOutcome::Holds)
                        .map(|result| (result.rule, true)),
                );
            }
        }
    }
    gate.stop().expect("the gate stops");

    for rule in Rule::ALL {
        assert!(
            covered.contains(&(rule, false)),
            "no deny case for {}",
            rule.name()
        );
        assert!(
            covered.contains(&(rule, true)),
            "no permit case for {}",
            rule.name()
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("\nall {checked} cases decided as expected");
}
