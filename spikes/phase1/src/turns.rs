//! Five scripted turns from the demo scenario (spec section 14), with what a correct
//! plan does in each. Times are UTC; the demo clock is Monday 11 January 2027, 08:00.

use serde_json::{Value, json};

use crate::plans::{Context, Expectation};
use crate::time::parse_utc;

pub struct Turn {
    pub id: &'static str,
    pub summary: &'static str,
    pub state: Value,
    pub context: Context,
    pub expectation: Expectation,
}

const NOW: &str = "2027-01-11T08:00:00Z";
const WINDOW_START: &str = "2027-01-11T08:00:00Z";
const WINDOW_END: &str = "2027-01-18T08:00:00Z";
const MINUTES: u32 = 30;

const ALICE_BUSY: [(&str, &str); 7] = [
    ("2027-01-11T12:00:00Z", "2027-01-11T17:00:00Z"),
    ("2027-01-12T09:00:00Z", "2027-01-12T11:00:00Z"),
    ("2027-01-12T15:00:00Z", "2027-01-12T16:00:00Z"),
    ("2027-01-13T13:00:00Z", "2027-01-13T14:30:00Z"),
    ("2027-01-14T16:00:00Z", "2027-01-14T17:30:00Z"),
    ("2027-01-15T12:00:00Z", "2027-01-15T18:00:00Z"),
    ("2027-01-16T00:00:00Z", "2027-01-18T08:00:00Z"),
];

const BOB_BUSY: [(&str, &str); 3] = [
    ("2027-01-12T13:30:00Z", "2027-01-12T15:00:00Z"),
    ("2027-01-13T09:00:00Z", "2027-01-13T10:00:00Z"),
    ("2027-01-14T10:00:00Z", "2027-01-14T12:00:00Z"),
];

/// Alice's round-one proposal, used as fixed input for Bob's and Alice's later turns.
const PROPOSAL: [(&str, &str); 3] = [
    ("c1", "2027-01-12T14:00:00Z"),
    ("c2", "2027-01-13T15:00:00Z"),
    ("c3", "2027-01-14T14:30:00Z"),
];

fn at(text: &str) -> i64 {
    parse_utc(text).expect("fixture times are valid")
}

fn busy(list: &[(&str, &str)]) -> (Value, Vec<(i64, i64)>) {
    let json_list = list
        .iter()
        .map(|(start, end)| json!({"start": start, "end": end}))
        .collect::<Vec<_>>();
    let intervals = list
        .iter()
        .map(|(start, end)| (at(start), at(end)))
        .collect();
    (Value::Array(json_list), intervals)
}

fn request(peer: &str, preference: &str) -> Value {
    json!({
        "meet_with": peer,
        "duration_minutes": MINUTES,
        "window": {"start": WINDOW_START, "end": WINDOW_END},
        "preference": preference
    })
}

fn proposal_message() -> Value {
    let candidates = PROPOSAL
        .iter()
        .map(|(id, start)| json!({"id": id, "start": start, "minutes": MINUTES}))
        .collect::<Vec<_>>();
    json!({
        "type": "proposal",
        "from": "alice",
        "round": 1,
        "candidates": candidates,
        "note": "afternoons suit me"
    })
}

fn received() -> Vec<(String, i64)> {
    PROPOSAL
        .iter()
        .map(|(id, start)| ((*id).to_owned(), at(start)))
        .collect()
}

fn context(peer: &'static str, busy: Vec<(i64, i64)>) -> Context {
    Context {
        peer,
        now: at(NOW),
        window: (at(WINDOW_START), at(WINDOW_END)),
        duration_minutes: MINUTES,
        busy,
        received: Vec::new(),
        consented: None,
    }
}

pub fn all() -> Vec<Turn> {
    let alice_request = request(
        "bob",
        "Afternoons, between 12:00 and 17:00 UTC, work best. Avoid Monday.",
    );
    let bob_request = request("alice", "Any weekday works; mornings are tighter.");
    let (alice_busy_json, alice_busy) = busy(&ALICE_BUSY);
    let (bob_busy_json, bob_busy) = busy(&BOB_BUSY);

    let mut turn_c_context = context("alice", Vec::new());
    turn_c_context.received = received();
    let mut turn_d_context = context("alice", bob_busy);
    turn_d_context.received = received();
    let mut turn_e_context = context("bob", alice_busy.clone());
    turn_e_context.consented = Some((at(PROPOSAL[2].1), MINUTES));

    vec![
        Turn {
            id: "A",
            summary: "Alice starts; she has not read her calendar yet",
            state: json!({
                "you_act_for": "alice",
                "your_role": "coordinator: you propose candidate times to the other person",
                "now": NOW,
                "owner_request": alice_request.clone(),
                "negotiation": {"stage": "not_started", "rounds_used": 0, "handshake_done": false},
                "known_free_busy": Value::Null,
                "inbound": [],
                "recent_denials": []
            }),
            context: context("bob", Vec::new()),
            expectation: Expectation::ReadFreeBusy,
        },
        Turn {
            id: "B",
            summary: "Alice has read her free/busy for the whole window",
            state: json!({
                "you_act_for": "alice",
                "your_role": "coordinator: you propose candidate times to the other person",
                "now": NOW,
                "owner_request": alice_request,
                "negotiation": {"stage": "not_started", "rounds_used": 0, "handshake_done": false},
                "known_free_busy": {
                    "window": {"start": WINDOW_START, "end": WINDOW_END},
                    "busy": alice_busy_json
                },
                "inbound": [],
                "recent_denials": []
            }),
            context: context("bob", alice_busy),
            expectation: Expectation::ProposeFreeSlots,
        },
        Turn {
            id: "C",
            summary: "Bob: Alice is calling with a three-candidate proposal",
            state: json!({
                "you_act_for": "bob",
                "your_role": "invitee: you answer the other person's proposals",
                "now": NOW,
                "owner_request": bob_request.clone(),
                "negotiation": {"stage": "proposal_received", "rounds_used": 1, "handshake_done": false},
                "known_free_busy": Value::Null,
                "inbound": [
                    {"type": "handshake_request", "from": "alice"},
                    proposal_message()
                ],
                "recent_denials": []
            }),
            context: turn_c_context,
            expectation: Expectation::HandshakeThenRead,
        },
        Turn {
            id: "D",
            summary: "Bob has read his free/busy around the candidates",
            state: json!({
                "you_act_for": "bob",
                "your_role": "invitee: you answer the other person's proposals",
                "now": NOW,
                "owner_request": bob_request,
                "negotiation": {"stage": "proposal_received", "rounds_used": 1, "handshake_done": true},
                "known_free_busy": {
                    "window": {"start": "2027-01-12T00:00:00Z", "end": "2027-01-15T00:00:00Z"},
                    "busy": bob_busy_json
                },
                "inbound": [proposal_message()],
                "recent_denials": []
            }),
            context: turn_d_context,
            expectation: Expectation::Respond(vec![false, true, true]),
        },
        Turn {
            id: "E",
            summary: "Alice: both owners decided; Bob's decisions arrived",
            state: json!({
                "you_act_for": "alice",
                "your_role": "coordinator: you propose candidate times to the other person",
                "now": NOW,
                "owner_request": request(
                    "bob",
                    "Afternoons, between 12:00 and 17:00 UTC, work best. Avoid Monday."
                ),
                "negotiation": {
                    "stage": "bob_decided_your_decisions_not_sent_yet",
                    "rounds_used": 1,
                    "handshake_done": true,
                    "your_decisions_sent": false,
                    "proposed": proposal_message()["candidates"].clone(),
                    "answers_from_bob": [
                        {"candidate_id": "c1", "available": false},
                        {"candidate_id": "c2", "available": true},
                        {"candidate_id": "c3", "available": true}
                    ],
                    "mutual_options": ["c2", "c3"],
                    "your_owner_decisions": [
                        {"candidate_id": "c2", "accept": true},
                        {"candidate_id": "c3", "accept": true}
                    ],
                    "decisions_from_bob": [
                        {"candidate_id": "c2", "accept": false},
                        {"candidate_id": "c3", "accept": true}
                    ],
                    "bob_decisions_signature_verified": true
                },
                "known_free_busy": {
                    "window": {"start": WINDOW_START, "end": WINDOW_END},
                    "busy": busy(&ALICE_BUSY).0
                },
                "inbound": [],
                "recent_denials": []
            }),
            context: turn_e_context,
            expectation: Expectation::DecideAndAdd {
                start: at(PROPOSAL[2].1),
                minutes: MINUTES,
            },
        },
    ]
}
