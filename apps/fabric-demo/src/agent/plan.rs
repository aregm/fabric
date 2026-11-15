//! The plans an agent's LLM writes (section 10.1): 2 or 3 alternatives per turn, each an ordered
//! list of actions, as JSON that matches [`plan_schema`]. Times are ISO 8601 UTC and durations
//! are minutes; the runtime turns them into policy actions for the active negotiation.

use fabric_policy::time::parse_utc;
use fabric_policy::{Action, Answer, Candidate, CloseReason, Decision};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSet {
    pub plans: Vec<Plan>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub label: String,
    pub rationale: String,
    /// No actions means "wait for the other person or the owner".
    pub actions: Vec<PlanAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanAction {
    ReadFreeBusy {
        start: String,
        end: String,
    },
    ReadEventDetails {
        start: String,
        end: String,
    },
    Handshake {
        peer: String,
    },
    SendPropose {
        round: u32,
        candidates: Vec<Slot>,
        note: String,
    },
    SendRespond {
        round: u32,
        answers: Vec<Answer>,
        note: String,
    },
    SendDecide {
        round: u32,
        decisions: Vec<Decision>,
        note: String,
    },
    SendClose {
        reason: CloseReason,
        note: String,
    },
    AddMeeting {
        start: String,
        minutes: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub start: String,
    pub minutes: i64,
}

/// The ID the runtime gives a proposal's candidate: `c<round>-<position>`.
#[must_use]
pub fn candidate_id(round: u32, position: usize) -> String {
    format!("c{round}-{}", position + 1)
}

fn time(text: &str) -> Result<i64, String> {
    parse_utc(text).ok_or_else(|| format!("{text:?} is not a UTC time like 2027-01-12T14:00:00Z"))
}

fn length(minutes: i64) -> Result<i64, String> {
    minutes
        .checked_mul(60 * 1_000)
        .ok_or_else(|| format!("{minutes} minutes is out of range"))
}

impl PlanAction {
    /// The policy action this plan action asks for, in negotiation `negotiation`.
    pub fn to_action(&self, negotiation: &str) -> Result<Action, String> {
        let negotiation = negotiation.to_owned();
        Ok(match self.clone() {
            Self::ReadFreeBusy { start, end } => Action::ReadFreeBusy {
                start_ms: time(&start)?,
                end_ms: time(&end)?,
            },
            Self::ReadEventDetails { start, end } => Action::ReadEventDetails {
                start_ms: time(&start)?,
                end_ms: time(&end)?,
            },
            Self::Handshake { peer } => Action::Handshake { peer },
            Self::SendPropose {
                round,
                candidates,
                note,
            } => Action::SendPropose {
                negotiation,
                round,
                candidates: candidates
                    .iter()
                    .enumerate()
                    .map(|(position, slot)| {
                        Ok(Candidate {
                            id: candidate_id(round, position),
                            start_ms: time(&slot.start)?,
                            length_ms: length(slot.minutes)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
                note,
            },
            Self::SendRespond {
                round,
                answers,
                note,
            } => Action::SendRespond {
                negotiation,
                round,
                answers,
                note,
            },
            Self::SendDecide {
                round,
                decisions,
                note,
            } => Action::SendDecide {
                negotiation,
                round,
                decisions,
                note,
            },
            Self::SendClose { reason, note } => Action::SendClose {
                negotiation,
                reason,
                note,
            },
            Self::AddMeeting { start, minutes } => Action::AddMeeting {
                negotiation,
                start_ms: time(&start)?,
                length_ms: length(minutes)?,
            },
        })
    }
}

/// JSON Schema for the planner's output. It shapes the output but enforces no limits; those are
/// Isabelle's job. Every action lists `kind` first, and every plan lists `label` and `rationale`
/// before `actions`, because constrained decoding writes properties in schema order (G1).
#[must_use]
pub fn plan_schema() -> Value {
    let time = json!({"type": "string", "format": "date-time"});
    let text = json!({"type": "string"});
    let integer = json!({"type": "integer"});
    let boolean = json!({"type": "boolean"});

    let slot = object(
        &[("start", time.clone()), ("minutes", integer.clone())],
        &["start", "minutes"],
    );
    let answer = object(
        &[
            ("candidate_id", text.clone()),
            ("available", boolean.clone()),
        ],
        &["candidate_id", "available"],
    );
    let decision = object(
        &[("candidate_id", text.clone()), ("accept", boolean)],
        &["candidate_id", "accept"],
    );
    let list = |items: Value| json!({"type": "array", "items": items});
    let actions = vec![
        variant(
            "read_free_busy",
            &[("start", time.clone()), ("end", time.clone())],
        ),
        variant(
            "read_event_details",
            &[("start", time.clone()), ("end", time.clone())],
        ),
        variant("handshake", &[("peer", text.clone())]),
        variant(
            "send_propose",
            &[
                ("round", integer.clone()),
                ("candidates", list(slot)),
                ("note", text.clone()),
            ],
        ),
        variant(
            "send_respond",
            &[
                ("round", integer.clone()),
                ("answers", list(answer)),
                ("note", text.clone()),
            ],
        ),
        variant(
            "send_decide",
            &[
                ("round", integer.clone()),
                ("decisions", list(decision)),
                ("note", text.clone()),
            ],
        ),
        variant(
            "send_close",
            &[
                (
                    "reason",
                    json!({"type": "string", "enum": ["agreed", "no_match", "withdrawn"]}),
                ),
                ("note", text.clone()),
            ],
        ),
        variant("add_meeting", &[("start", time), ("minutes", integer)]),
    ];
    let plan = object(
        &[
            ("label", text.clone()),
            ("rationale", text),
            ("actions", list(json!({"anyOf": actions}))),
        ],
        &["label", "rationale", "actions"],
    );
    object(&[("plans", list(plan))], &["plans"])
}

fn object(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let mut ordered = Map::new();
    for (name, schema) in properties {
        ordered.insert((*name).to_owned(), schema.clone());
    }
    json!({
        "type": "object",
        "properties": Value::Object(ordered),
        "required": required,
        "additionalProperties": false
    })
}

/// An action object with `kind` as its first property.
fn variant(kind: &str, properties: &[(&str, Value)]) -> Value {
    let mut all = vec![("kind", json!({"type": "string", "enum": [kind]}))];
    all.extend(properties.iter().cloned());
    let required: Vec<&str> = all.iter().map(|(name, _)| *name).collect();
    object(&all, &required)
}

#[cfg(test)]
mod tests {
    use fabric_policy::Action;

    use super::{PlanAction, PlanSet, Slot, plan_schema};

    #[test]
    fn the_schema_lists_kind_first_and_label_and_rationale_before_actions() {
        let schema = plan_schema();
        let plan = &schema["properties"]["plans"]["items"]["properties"];
        let order: Vec<&String> = plan.as_object().unwrap().keys().collect();
        assert_eq!(order, ["label", "rationale", "actions"]);
        for variant in plan["actions"]["items"]["anyOf"].as_array().unwrap() {
            let first = variant["properties"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap();
            assert_eq!(first, "kind");
        }
    }

    #[test]
    fn plan_actions_become_policy_actions_of_the_active_negotiation() {
        let propose = PlanAction::SendPropose {
            round: 1,
            candidates: vec![
                Slot {
                    start: "2027-01-12T14:00:00Z".to_owned(),
                    minutes: 30,
                },
                Slot {
                    start: "2027-01-14T14:30Z".to_owned(),
                    minutes: 30,
                },
            ],
            note: "afternoons suit me".to_owned(),
        };
        let Action::SendPropose {
            negotiation,
            candidates,
            ..
        } = propose.to_action("n-1").unwrap()
        else {
            panic!("a proposal");
        };
        assert_eq!(negotiation, "n-1");
        assert_eq!(candidates[1].id, "c1-2");
        assert_eq!(candidates[0].start_ms, 1_799_762_400_000);
        assert_eq!(candidates[0].length_ms, 1_800_000);

        let unreadable = PlanAction::AddMeeting {
            start: "next Tuesday".to_owned(),
            minutes: 30,
        };
        assert!(unreadable.to_action("n-1").is_err());

        let text = r#"{"plans": [{"label": "wait", "rationale": "nothing to do", "actions": []}]}"#;
        let plans: PlanSet = serde_json::from_str(text).unwrap();
        assert!(plans.plans[0].actions.is_empty());
        let extra = r#"{"plans": [], "thoughts": "x"}"#;
        assert!(serde_json::from_str::<PlanSet>(extra).is_err());
    }
}
