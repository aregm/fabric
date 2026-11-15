//! Check theories and the verdicts read back from Isabelle's messages.
//!
//! A check theory defines the concrete context and actions, then states, for every rule and
//! action, the rule and its negation as two lemmas. Isabelle can prove at most one of them, so
//! the lemma without an error decides the rule. Anything else fails closed as undecided.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::PolicyError;
use crate::explain;
use crate::hol;
use crate::model::{Action, Context, Parameters, Rule};

/// The session-qualified name of the policy theory every check imports.
pub const POLICY_THEORY: &str = "Fabric_Policy.Fabric_Policy";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Polarity {
    Holds,
    Fails,
}

/// Where a lemma sits in a check theory (1-based line).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LemmaLine {
    pub line: usize,
    pub action: usize,
    pub rule: Rule,
    pub polarity: Polarity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quantity {
    PayloadCount,
    NoteLength,
}

/// Where a `value` command sits in a check theory (1-based line).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueLine {
    pub line: usize,
    pub action: usize,
    pub quantity: Quantity,
}

#[derive(Clone, Debug)]
pub struct CheckTheory {
    pub name: String,
    pub text: String,
    pub lemmas: Vec<LemmaLine>,
    pub values: Vec<ValueLine>,
}

/// Writes the check theory for one context and a batch of actions.
pub fn generate(
    name: &str,
    context: &Context,
    actions: &[Action],
) -> Result<CheckTheory, PolicyError> {
    let valid_name = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !valid_name {
        return Err(PolicyError::Render(format!(
            "theory name {name:?} must start with a letter and use only letters, digits and '_'"
        )));
    }

    let mut lines = vec![
        format!("theory {name}"),
        format!("  imports \"{POLICY_THEORY}\""),
        "begin".to_owned(),
        String::new(),
        format!(
            "definition ctx :: gate_context where \"ctx = {}\"",
            hol::context(context)?
        ),
    ];
    let mut lemmas = Vec::new();
    let mut values = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        let constant = format!("a{index}");
        lines.push(String::new());
        lines.push(format!(
            "definition {constant} :: action where \"{constant} = {}\"",
            hol::action(action)?
        ));
        let method = format!("(simp add: {constant}_def ctx_def policy_simps)");
        for rule in Rule::ALL {
            let statement = format!("holds {} ctx {constant}", rule.hol_constructor());
            lines.push(format!(
                "lemma {constant}_{}_holds: \"{statement}\" by {method}",
                rule.name()
            ));
            lemmas.push(LemmaLine {
                line: lines.len(),
                action: index,
                rule,
                polarity: Polarity::Holds,
            });
            lines.push(format!(
                "lemma {constant}_{}_fails: \"\\<not> {statement}\" by {method}",
                rule.name()
            ));
            lemmas.push(LemmaLine {
                line: lines.len(),
                action: index,
                rule,
                polarity: Polarity::Fails,
            });
        }
        if action.note().is_some() {
            lines.push(format!("value \"payload_count {constant}\""));
            values.push(ValueLine {
                line: lines.len(),
                action: index,
                quantity: Quantity::PayloadCount,
            });
            lines.push(format!("value \"length (note_of {constant})\""));
            values.push(ValueLine {
                line: lines.len(),
                action: index,
                quantity: Quantity::NoteLength,
            });
        }
    }
    lines.push(String::new());
    lines.push("end".to_owned());

    Ok(CheckTheory {
        name: name.to_owned(),
        text: lines.join("\n") + "\n",
        lemmas,
        values,
    })
}

/// What Isabelle reported for a check theory's node.
#[derive(Clone, Debug, Default)]
pub struct NodeReport {
    /// True only when the node is consolidated with nothing unprocessed, running or canceled.
    pub complete: bool,
    pub errors: Vec<(usize, String)>,
    pub writeln: Vec<(usize, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOutcome {
    Holds,
    Fails,
    Undecided,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: Rule,
    pub outcome: RuleOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Denial {
    pub rule: Rule,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Permit,
    Deny { denials: Vec<Denial> },
    Undecided { reason: String },
}

impl Verdict {
    #[must_use]
    pub const fn is_permit(&self) -> bool {
        matches!(self, Self::Permit)
    }

    /// The failing rule names, or the reason when undecided; empty for a permit.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Permit => String::new(),
            Self::Deny { denials } => denials
                .iter()
                .map(|denial| format!("{}: {}", denial.rule.name(), denial.detail))
                .collect::<Vec<_>>()
                .join("; "),
            Self::Undecided { reason } => format!("undecided: {reason}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionVerdict {
    pub action: Action,
    pub verdict: Verdict,
    pub rules: Vec<RuleResult>,
}

/// Numbers Isabelle computed with `value` for one action.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Quantities {
    pub payload_count: Option<u64>,
    pub note_length: Option<u64>,
}

/// Turns Isabelle's report into one verdict per action.
#[must_use]
pub fn verdicts(
    theory: &CheckTheory,
    report: &NodeReport,
    context: &Context,
    actions: &[Action],
    parameters: &Parameters,
) -> Vec<ActionVerdict> {
    let all_undecided = |reason: String| {
        actions
            .iter()
            .map(|action| ActionVerdict {
                action: action.clone(),
                verdict: Verdict::Undecided {
                    reason: reason.clone(),
                },
                rules: Vec::new(),
            })
            .collect()
    };
    if !report.complete {
        return all_undecided("Isabelle did not finish the check".to_owned());
    }
    let lemma_lines: HashSet<usize> = theory.lemmas.iter().map(|lemma| lemma.line).collect();
    if let Some((line, message)) = report
        .errors
        .iter()
        .find(|(line, _)| !lemma_lines.contains(line))
    {
        return all_undecided(format!(
            "the check theory failed outside a rule lemma, at line {line}: {}",
            message.lines().next().unwrap_or("")
        ));
    }

    let failed: HashSet<usize> = report.errors.iter().map(|(line, _)| *line).collect();
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let rules: Vec<RuleResult> = Rule::ALL
                .iter()
                .map(|&rule| {
                    let line_of = |polarity| {
                        theory
                            .lemmas
                            .iter()
                            .find(|lemma| {
                                lemma.action == index
                                    && lemma.rule == rule
                                    && lemma.polarity == polarity
                            })
                            .map(|lemma| lemma.line)
                    };
                    let outcome = match (line_of(Polarity::Holds), line_of(Polarity::Fails)) {
                        (Some(holds), Some(fails)) => {
                            match (failed.contains(&holds), failed.contains(&fails)) {
                                (false, true) => RuleOutcome::Holds,
                                (true, false) => RuleOutcome::Fails,
                                _ => RuleOutcome::Undecided,
                            }
                        }
                        _ => RuleOutcome::Undecided,
                    };
                    RuleResult { rule, outcome }
                })
                .collect();

            let verdict = if let Some(undecided) = rules
                .iter()
                .find(|result| result.outcome == RuleOutcome::Undecided)
            {
                Verdict::Undecided {
                    reason: format!(
                        "{}: Isabelle proved neither, or both, of the rule and its negation",
                        undecided.rule.name()
                    ),
                }
            } else {
                let quantities = quantities(theory, report, index);
                let denials: Vec<Denial> = rules
                    .iter()
                    .filter(|result| result.outcome == RuleOutcome::Fails)
                    .map(|result| Denial {
                        rule: result.rule,
                        detail: explain::detail(
                            result.rule,
                            context,
                            action,
                            quantities,
                            parameters,
                        ),
                    })
                    .collect();
                if denials.is_empty() {
                    Verdict::Permit
                } else {
                    Verdict::Deny { denials }
                }
            };
            ActionVerdict {
                action: action.clone(),
                verdict,
                rules,
            }
        })
        .collect()
}

fn quantities(theory: &CheckTheory, report: &NodeReport, action: usize) -> Quantities {
    let mut quantities = Quantities::default();
    for value in theory.values.iter().filter(|value| value.action == action) {
        let number = report
            .writeln
            .iter()
            .find(|(line, _)| *line == value.line)
            .and_then(|(_, text)| parse_number(text));
        match value.quantity {
            Quantity::PayloadCount => quantities.payload_count = number,
            Quantity::NoteLength => quantities.note_length = number,
        }
    }
    quantities
}

/// Reads the number from `value` output such as `"5" :: "nat"`.
#[must_use]
pub fn parse_number(text: &str) -> Option<u64> {
    let digits: String = text
        .trim()
        .trim_start_matches('"')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{NodeReport, Polarity, Verdict, generate, parse_number, verdicts};
    use crate::model::{Action, Context, Negotiation, Parameters, Request, Role, Rule};

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

    fn handshake() -> Vec<Action> {
        vec![Action::Handshake {
            peer: "carol".to_owned(),
        }]
    }

    /// Isabelle's report when exactly the given rules fail and every other rule holds.
    fn report_where_failing(theory: &super::CheckTheory, failing: &[Rule]) -> NodeReport {
        let errors = theory
            .lemmas
            .iter()
            .filter(|lemma| {
                let fails = failing.contains(&lemma.rule);
                (fails && lemma.polarity == Polarity::Holds)
                    || (!fails && lemma.polarity == Polarity::Fails)
            })
            .map(|lemma| (lemma.line, "Failed to finish proof".to_owned()))
            .collect();
        NodeReport {
            complete: true,
            errors,
            writeln: Vec::new(),
        }
    }

    #[test]
    fn a_proven_negation_denies_with_the_rule_and_detail() {
        let actions = handshake();
        let theory = generate("Check_1", &context(), &actions).unwrap();
        let report = report_where_failing(&theory, &[Rule::HandshakeTarget]);
        let verdict =
            &verdicts(&theory, &report, &context(), &actions, &Parameters::DEFAULT)[0].verdict;
        assert_eq!(
            verdict.summary(),
            "handshake_target: carol is not the requested peer bob"
        );
    }

    #[test]
    fn every_rule_holding_permits() {
        let actions = handshake();
        let theory = generate("Check_1", &context(), &actions).unwrap();
        let report = report_where_failing(&theory, &[]);
        assert!(
            verdicts(&theory, &report, &context(), &actions, &Parameters::DEFAULT)[0]
                .verdict
                .is_permit()
        );
    }

    #[test]
    fn incomplete_stray_or_ambiguous_results_fail_closed() {
        let actions = handshake();
        let theory = generate("Check_1", &context(), &actions).unwrap();
        let undecided = |report: &NodeReport| {
            matches!(
                verdicts(&theory, report, &context(), &actions, &Parameters::DEFAULT)[0].verdict,
                Verdict::Undecided { .. }
            )
        };

        let mut incomplete = report_where_failing(&theory, &[]);
        incomplete.complete = false;
        assert!(undecided(&incomplete));

        let mut stray = report_where_failing(&theory, &[]);
        stray.errors.push((5, "Type unification failed".to_owned()));
        assert!(undecided(&stray));

        let mut both_proven = report_where_failing(&theory, &[]);
        both_proven.errors.retain(|(line, _)| {
            theory.lemmas.iter().any(|lemma| {
                lemma.line == *line
                    && !(lemma.rule == Rule::Note && lemma.polarity == Polarity::Fails)
            })
        });
        assert!(undecided(&both_proven));

        let mut neither_proven = report_where_failing(&theory, &[]);
        let note_holds = theory
            .lemmas
            .iter()
            .find(|lemma| lemma.rule == Rule::Note && lemma.polarity == Polarity::Holds)
            .unwrap()
            .line;
        neither_proven
            .errors
            .push((note_holds, "Failed to finish proof".to_owned()));
        assert!(undecided(&neither_proven));
    }

    #[test]
    fn invalid_theory_names_and_identifiers_are_rejected() {
        assert!(generate("1Check", &context(), &handshake()).is_err());
        assert!(generate("Check-1", &context(), &handshake()).is_err());
        let injected = vec![Action::Handshake {
            peer: "bob'' \\<or> True".to_owned(),
        }];
        assert!(generate("Check_1", &context(), &injected).is_err());
    }

    #[test]
    fn value_output_is_parsed() {
        assert_eq!(parse_number("\"5\" :: \"nat\""), Some(5));
        assert_eq!(parse_number("\"900000\"\n  :: \"int\""), Some(900_000));
        assert_eq!(parse_number("True"), None);
    }
}
