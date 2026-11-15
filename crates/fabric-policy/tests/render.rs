//! The generated check theory must match a hand-written one exactly (task T2.4), so a change to
//! the renderer shows up as a reviewable diff to `fixtures/Check_000001.thy`.

use fabric_policy::check::{self, Polarity, Quantity};
use fabric_policy::{Action, Candidate, Context, Negotiation, Request, Role, Rule};

fn context() -> Context {
    Context {
        owner: "alice".to_owned(),
        now_ms: 0,
        request: Request {
            peer: "bob".to_owned(),
            duration_ms: 1_800_000,
            window_start_ms: 0,
            window_end_ms: 3_600_000,
            opened_at_ms: 0,
            expires_at_ms: 3_600_000,
        },
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

fn actions() -> Vec<Action> {
    vec![
        Action::Handshake {
            peer: "bob".to_owned(),
        },
        Action::SendPropose {
            negotiation: "n1".to_owned(),
            round: 1,
            candidates: vec![Candidate {
                id: "c1".to_owned(),
                start_ms: 900_000,
                length_ms: 1_800_000,
            }],
            note: "hi".to_owned(),
        },
    ]
}

#[test]
fn check_theory_matches_the_hand_written_fixture() {
    let theory = check::generate("Check_000001", &context(), &actions()).unwrap();
    let expected = include_str!("fixtures/Check_000001.thy").replace("\r\n", "\n");
    assert_eq!(theory.text, expected);
}

#[test]
fn lemma_and_value_lines_point_at_the_right_statements() {
    let theory = check::generate("Check_000001", &context(), &actions()).unwrap();
    let lines: Vec<&str> = theory.text.lines().collect();
    assert_eq!(theory.lemmas.len(), 2 * 2 * Rule::ALL.len());
    for lemma in &theory.lemmas {
        let expected = format!(
            "lemma a{}_{}_{}:",
            lemma.action,
            lemma.rule.name(),
            match lemma.polarity {
                Polarity::Holds => "holds",
                Polarity::Fails => "fails",
            }
        );
        assert!(
            lines[lemma.line - 1].starts_with(&expected),
            "line {} is {:?}, expected {expected}",
            lemma.line,
            lines[lemma.line - 1]
        );
    }
    let values: Vec<(usize, Quantity)> = theory
        .values
        .iter()
        .map(|value| (value.line, value.quantity))
        .collect();
    assert_eq!(
        values,
        vec![(58, Quantity::PayloadCount), (59, Quantity::NoteLength)]
    );
}
