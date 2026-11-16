//! The generated check theory must match the hand-written one exactly, so a change to the
//! renderer shows up as a reviewable diff to `fixtures/Check_000001.thy`; and no input can
//! change the theory's syntax.

mod common;

use fabric_broker_policy::check::{self, Polarity, Statement};
use fabric_broker_policy::{
    Action, Actor, Context, Duration, PendingEffect, RecipientRule, Rule, Target,
};

#[test]
fn check_theory_matches_the_hand_written_fixture() {
    let theory = check::generate("Check_000001", &common::context(), &common::action()).unwrap();
    let expected = include_str!("fixtures/Check_000001.thy").replace("\r\n", "\n");
    assert_eq!(theory.text, expected);
}

#[test]
fn lemma_lines_point_at_the_right_statements() {
    let theory = check::generate("Check_000001", &common::context(), &common::action()).unwrap();
    let lines: Vec<&str> = theory.text.lines().collect();
    assert_eq!(theory.lemmas.len(), 2 * Rule::ALL.len());
    for lemma in &theory.lemmas {
        let (prefix, statement) = match lemma.statement {
            Statement::Rule { rule, polarity } => {
                let holds = format!("holds {} ctx act", rule.hol_constructor());
                match polarity {
                    Polarity::Holds => (format!("lemma rule_{}_holds:", rule.name()), holds),
                    Polarity::Fails => (
                        format!("lemma rule_{}_fails:", rule.name()),
                        format!("\\<not> {holds}"),
                    ),
                }
            }
        };
        let line = lines[lemma.line - 1];
        assert_eq!(
            line,
            format!("{prefix} \"{statement}\" by {}", check::METHOD),
            "line {}",
            lemma.line
        );
    }
    assert!(!theory.text.contains("outcome ctx act"));
}

const HOSTILE: [&str; 9] = [
    "\"",
    "''",
    "\\",
    "\\<close>",
    "*)",
    "(*",
    "a\nlemma x: \"True\" by simp\n",
    "\\<lparr>",
    "x'' \\<or> True \\<or> ''",
];

/// Puts `text` into every field of the context and action that holds outside text.
fn with_outside_text(text: &str) -> (Context, Action) {
    let mut context = common::context();
    let event = context.event.as_mut().unwrap();
    event.id = text.to_owned();
    event.revision = text.to_owned();
    event.attendees[0].address = text.to_owned();
    event.attendees[0].domain = text.to_owned();
    context.recipient_sets[0].members = vec![text.to_owned()];
    context.recipient_sets[0].domains = vec![text.to_owned()];
    context.grants[1].recipients = Some(RecipientRule::Exact(vec![text.to_owned()]));
    context.pending_effects = vec![PendingEffect {
        account: "acct-1".to_owned(),
        calendar: "team".to_owned(),
        event_id: text.to_owned(),
    }];
    let action = Action::ChangeInvitation {
        account: "acct-1".to_owned(),
        calendar: "team".to_owned(),
        event_id: text.to_owned(),
        revision: text.to_owned(),
        attendees: vec![common::recipient(text, text)],
        send_updates: fabric_broker_policy::SendUpdates::NotifyAll,
    };
    (context, action)
}

fn shape(text: &str) -> Vec<(usize, usize, usize)> {
    let stripped = text
        .replace("\\<lparr>", "")
        .replace("\\<rparr>", "")
        .replace("\\<not>", "");
    stripped
        .lines()
        .map(|line| {
            (
                line.matches('"').count(),
                line.matches("''").count(),
                line.matches('\\').count(),
            )
        })
        .collect()
}

#[test]
fn outside_text_cannot_change_the_theory_syntax() {
    let (context, action) = with_outside_text("x");
    let benign = check::generate("Check_1", &context, &action).unwrap();
    let (context, action) = with_outside_text("\u{10FFFF}");
    let sentinel = check::generate("Check_1", &context, &action).unwrap();
    assert!(!benign.text.contains("1114111"));
    for hostile in HOSTILE {
        let (context, action) = with_outside_text(hostile);
        let theory = check::generate("Check_1", &context, &action).unwrap();
        assert_eq!(theory.lemmas, benign.lemmas, "{hostile:?}");
        assert_eq!(shape(&theory.text), shape(&benign.text), "{hostile:?}");
        assert!(!theory.text.contains("\\<close>"), "{hostile:?}");
        assert!(!theory.text.contains("*)"), "{hostile:?}");
        // The theory is the one with a sentinel character, with the sentinel's code point
        // replaced by the hostile text's code points: nothing else differs.
        let codes = hostile
            .chars()
            .map(|c| u32::from(c).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(
            theory.text,
            sentinel.text.replace("1114111", &codes),
            "{hostile:?}"
        );
    }
}

#[test]
fn hostile_identifiers_are_refused_everywhere() {
    type Edit = fn(&mut Context, &mut Action, &str);
    let edits: [(&str, Edit); 10] = [
        ("agent", |c, _, s| c.actor = Actor::Agent(s.to_owned())),
        ("connected", |c, _, s| c.connected = vec![s.to_owned()]),
        ("calendar", |c, _, s| c.calendars[0].id = s.to_owned()),
        ("grant", |c, _, s| c.grants[0].id = s.to_owned()),
        ("session", |c, _, s| {
            c.grants[0].duration = Duration::Session {
                session_id: s.to_owned(),
            };
        }),
        ("set", |c, _, s| c.recipient_sets[0].id = s.to_owned()),
        ("relied", |c, _, s| c.relied = Some(s.to_owned())),
        ("digest", |c, _, s| c.digest = s.to_owned()),
        ("one request digest", |c, _, s| {
            c.grants[1].duration = Duration::OneRequest {
                digest: s.to_owned(),
                used: false,
            };
        }),
        ("action account", |_, a, s| {
            *a = Action::Respond {
                account: s.to_owned(),
                calendar: "team".to_owned(),
                target: Target::WholeEvent {
                    event_id: "e".to_owned(),
                    revision: "r".to_owned(),
                },
                response: fabric_broker_policy::Response::Accepted,
                send_updates: fabric_broker_policy::SendUpdates::NotifyNone,
            };
        }),
    ];
    for (name, edit) in edits {
        for hostile in HOSTILE.iter().copied().chain(["", "a b", "é"]) {
            let mut context = common::context();
            let mut action = common::action();
            edit(&mut context, &mut action, hostile);
            assert!(
                check::generate("Check_1", &context, &action).is_err(),
                "{name} accepted {hostile:?}"
            );
        }
    }
}

#[test]
fn invalid_theory_names_are_refused() {
    for name in ["1Check", "Check-1", "", "Check 1", "Check_1\"", "Ä"] {
        assert!(check::generate(name, &common::context(), &common::action()).is_err());
    }
}

#[test]
fn every_action_constructor_renders_in_the_theorys_argument_order() {
    let s = |v: &str| v.to_owned();
    let cases = [
        (Action::ListCalendars, "List_calendars"),
        (
            Action::ReadAvailability {
                account: s("a"),
                calendars: vec![s("c"), s("d")],
                start_ms: -1,
                end_ms: 2,
            },
            "Read_availability ''a'' [''c'', ''d''] (-1) 2",
        ),
        (
            Action::ReadEvents {
                account: s("a"),
                calendars: vec![s("c")],
                start_ms: 1,
                end_ms: 2,
                fields: vec![
                    fabric_broker_policy::EventField::OwnResponse,
                    fabric_broker_policy::EventField::Title,
                ],
            },
            "Read_events ''a'' [''c''] 1 2 [Field_own_response, Field_title]",
        ),
        (
            Action::CreateEvent {
                account: s("a"),
                calendar: s("c"),
            },
            "Create_event ''a'' ''c''",
        ),
        (
            Action::UpdateEvent {
                account: s("a"),
                calendar: s("c"),
                event_id: s("e"),
                revision: s("r"),
                send_updates: fabric_broker_policy::SendUpdates::NotifyExternal,
            },
            "Update_event ''a'' ''c'' [101] [114] Notify_external",
        ),
        (
            Action::Respond {
                account: s("a"),
                calendar: s("c"),
                target: Target::Occurrence {
                    instance_id: s("i"),
                    original_start_ms: -5,
                    revision: s("r"),
                },
                response: fabric_broker_policy::Response::Declined,
                send_updates: fabric_broker_policy::SendUpdates::NotifyNone,
            },
            "Respond ''a'' ''c'' (Occurrence [105] (-5) [114]) Declined Notify_none",
        ),
        (
            Action::DeleteEvent {
                account: s("a"),
                calendar: s("c"),
                event_id: s("e"),
                revision: s("r"),
                send_updates: fabric_broker_policy::SendUpdates::NotifyAll,
            },
            "Delete_event ''a'' ''c'' [101] [114] Notify_all",
        ),
        (Action::DiscloseWarning, "Disclose_warning"),
        (
            Action::DeleteCalendar {
                account: s("a"),
                calendar: s("c"),
            },
            "Delete_calendar ''a'' ''c''",
        ),
        (
            Action::ChangeSharing {
                account: s("a"),
                calendar: s("c"),
            },
            "Change_sharing ''a'' ''c''",
        ),
        (
            Action::SystemRefresh {
                account: s("a"),
                calendar: s("c"),
            },
            "System_refresh ''a'' ''c''",
        ),
        (
            Action::SystemReconcile {
                account: s("a"),
                calendar: s("c"),
                event_id: s("e"),
            },
            "System_reconcile ''a'' ''c'' [101]",
        ),
    ];
    for (action, expected) in cases {
        assert_eq!(
            fabric_broker_policy::hol::action(&action).unwrap(),
            expected
        );
    }
}
