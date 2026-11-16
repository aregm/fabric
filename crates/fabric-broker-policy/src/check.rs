//! Check theories for one action, and the verdict read back from Isabelle's messages.
//!
//! A check theory imports `Fabric_Broker`, defines the concrete context `ctx` and action
//! `act`, and states for every rule `holds r ctx act` and its negation, each as a lemma
//! proven by the simplifier. Isabelle can prove at most one lemma of each pair. The outcome is
//! the one the eighteen results imply, by the theory's lemmas `permit_iff`, `block_iff` and
//! `need_approval_iff`, which the session proves once for every context. The verdict is an
//! outcome only when every rule is decided; anything else fails closed as undecided (plan
//! section 8).
//!
//! After the rule lemmas the theory states, for every grant of the context in order,
//! `authorizes ctx act (ctx_grants ctx ! i)` and its negation, so the proof itself says which
//! grants a permitted action rests on (lemma `permit_rests_on_an_authorizing_grant`).
//!
//! A check does not also prove `outcome ctx act = o` directly. One such lemma costs as much
//! as all the rule lemmas together and cannot run in parallel; measured on 2026-11-16 it made
//! a check take 2.5 s on average.

use std::collections::HashSet;

use fabric_policy::PolicyError;
pub use fabric_policy::check::{NodeReport, RuleOutcome};
use serde::{Deserialize, Serialize};

use crate::hol;
use crate::model::{Action, Context, Outcome, Rule};

/// The session-qualified name of the rule theory every check imports.
pub const BROKER_THEORY: &str = "Fabric_Broker.Fabric_Broker";

/// The proof method of every rule lemma of a check theory.
pub const METHOD: &str = "(simp add: act_def ctx_def broker_simps)";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// `holds r ctx act`.
    Holds,
    /// `\<not> holds r ctx act`.
    Fails,
}

/// What a lemma of a check theory states.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statement {
    Rule {
        rule: Rule,
        polarity: Polarity,
    },
    /// `authorizes ctx act (ctx_grants ctx ! index)`, or its negation when `authorizes` is
    /// false.
    Grant {
        index: usize,
        authorizes: bool,
    },
}

/// Where a lemma sits in a check theory (1-based line).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LemmaLine {
    pub line: usize,
    pub statement: Statement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckTheory {
    pub name: String,
    pub text: String,
    pub lemmas: Vec<LemmaLine>,
    /// The ids of the context's grants, in order: grant `i` of the lemmas is `grant_ids[i]`.
    pub grant_ids: Vec<String>,
    /// [`Action::needs_no_grant`] of the checked action.
    pub needs_no_grant: bool,
}

impl CheckTheory {
    /// The line of the lemma that states `statement`.
    #[must_use]
    pub fn line_of(&self, statement: Statement) -> Option<usize> {
        self.lemmas
            .iter()
            .find(|lemma| lemma.statement == statement)
            .map(|lemma| lemma.line)
    }
}

/// Writes the check theory for one context and one action.
pub fn generate(
    name: &str,
    context: &Context,
    action: &Action,
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

    // Grant ids name grants in `ctx_relied` and in the result of a check, so two grants with
    // one id would make both ambiguous. The broker keeps them unique; refuse anything else.
    let mut seen = HashSet::new();
    if let Some(grant) = context.grants.iter().find(|grant| !seen.insert(&grant.id)) {
        return Err(PolicyError::Render(format!(
            "two grants of the context have the id {:?}",
            grant.id
        )));
    }

    let mut lines = vec![
        format!("theory {name}"),
        format!("  imports \"{BROKER_THEORY}\""),
        "begin".to_owned(),
        String::new(),
        format!(
            "definition ctx :: broker_context where \"ctx = {}\"",
            hol::context(context)?
        ),
        String::new(),
        format!(
            "definition act :: action where \"act = {}\"",
            hol::action(action)?
        ),
        String::new(),
    ];
    let mut lemmas = Vec::new();
    for rule in Rule::ALL {
        let statement = format!("holds {} ctx act", rule.hol_constructor());
        for (polarity, suffix, text) in [
            (Polarity::Holds, "holds", statement.clone()),
            (Polarity::Fails, "fails", format!("\\<not> {statement}")),
        ] {
            lines.push(format!(
                "lemma rule_{}_{suffix}: \"{text}\" by {METHOD}",
                rule.name()
            ));
            lemmas.push(LemmaLine {
                line: lines.len(),
                statement: Statement::Rule { rule, polarity },
            });
        }
    }
    for index in 0..context.grants.len() {
        let statement = format!("authorizes ctx act (ctx_grants ctx ! {index})");
        for (authorizes, suffix, text) in [
            (true, "authorizes", statement.clone()),
            (false, "refused", format!("\\<not> {statement}")),
        ] {
            lines.push(format!(
                "lemma grant_{index}_{suffix}: \"{text}\" by {METHOD}"
            ));
            lemmas.push(LemmaLine {
                line: lines.len(),
                statement: Statement::Grant { index, authorizes },
            });
        }
    }
    lines.push(String::new());
    lines.push("end".to_owned());

    Ok(CheckTheory {
        name: name.to_owned(),
        text: lines.join("\n") + "\n",
        lemmas,
        grant_ids: context
            .grants
            .iter()
            .map(|grant| grant.id.clone())
            .collect(),
        needs_no_grant: action.needs_no_grant(),
    })
}

/// What Isabelle proved about one grant of the context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GrantResult {
    pub id: String,
    /// `Holds` when the grant authorizes the action, `Fails` when it is refused.
    pub outcome: RuleOutcome,
}

/// One rule's result in a check.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: Rule,
    pub outcome: RuleOutcome,
}

/// The verdict of a check: an outcome Isabelle proved, or undecided with the reason.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Decided { outcome: Outcome },
    Undecided { reason: String },
}

impl Verdict {
    /// The proven outcome, or `None` when undecided.
    #[must_use]
    pub const fn outcome(&self) -> Option<Outcome> {
        match self {
            Verdict::Decided { outcome } => Some(*outcome),
            Verdict::Undecided { .. } => None,
        }
    }

    #[must_use]
    pub const fn is_permit(&self) -> bool {
        matches!(
            self,
            Verdict::Decided {
                outcome: Outcome::Permit
            }
        )
    }
}

/// What the gate read from Isabelle's report on one check theory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub verdict: Verdict,
    /// The first failing rule in theory order, when the verdict is Block or Need_approval.
    pub failing_rule: Option<Rule>,
    /// Every rule in theory order, with what Isabelle proved about it.
    pub rules: Vec<RuleResult>,
    /// Every grant of the context in order, with what Isabelle proved about it.
    pub grants: Vec<GrantResult>,
    /// The ids of the grants proven to authorize the action, in context order; empty when
    /// undecided. The broker names them in `ctx_relied` of the request's later checks.
    pub authorizing_grants: Vec<String>,
}

impl Reading {
    /// The same reading with the verdict made undecided for `reason`. The rule results stay
    /// as read, and no failing rule is reported.
    #[must_use]
    pub fn undecided(self, reason: String) -> Self {
        Reading {
            verdict: Verdict::Undecided { reason },
            failing_rule: None,
            rules: self.rules,
            grants: self.grants,
            authorizing_grants: Vec::new(),
        }
    }
}

/// The outcome the rule results imply, as the theory's `outcome` defines it: a failing block
/// rule blocks; otherwise a failing approval rule asks; otherwise permit.
#[must_use]
pub fn implied_outcome(rules: &[RuleResult]) -> Outcome {
    let fails = |block: bool| {
        rules
            .iter()
            .any(|r| r.rule.is_block_rule() == block && r.outcome == RuleOutcome::Fails)
    };
    if fails(true) {
        Outcome::Block
    } else if fails(false) {
        Outcome::NeedApproval
    } else {
        Outcome::Permit
    }
}

/// Reads Isabelle's report on a check theory. Fails closed: the verdict is undecided unless
/// the node is complete, no error lies outside a lemma line, and every rule and every grant is
/// decided. The outcome is then the one the rule results imply; a permit that no grant
/// authorizes is undecided too, unless the action needs no grant.
#[must_use]
pub fn read(theory: &CheckTheory, report: &NodeReport) -> Reading {
    let failed: HashSet<usize> = report.errors.iter().map(|(line, _)| *line).collect();
    let proven = |statement| {
        theory
            .line_of(statement)
            .map(|line| !failed.contains(&line))
    };
    let rules: Vec<RuleResult> = Rule::ALL
        .iter()
        .map(|&rule| {
            let holds = proven(Statement::Rule {
                rule,
                polarity: Polarity::Holds,
            });
            let fails = proven(Statement::Rule {
                rule,
                polarity: Polarity::Fails,
            });
            let outcome = match (holds, fails) {
                (Some(true), Some(false)) => RuleOutcome::Holds,
                (Some(false), Some(true)) => RuleOutcome::Fails,
                _ => RuleOutcome::Undecided,
            };
            RuleResult { rule, outcome }
        })
        .collect();

    let grants: Vec<GrantResult> = theory
        .grant_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let authorizes = proven(Statement::Grant {
                index,
                authorizes: true,
            });
            let refused = proven(Statement::Grant {
                index,
                authorizes: false,
            });
            let outcome = match (authorizes, refused) {
                (Some(true), Some(false)) => RuleOutcome::Holds,
                (Some(false), Some(true)) => RuleOutcome::Fails,
                _ => RuleOutcome::Undecided,
            };
            GrantResult {
                id: id.clone(),
                outcome,
            }
        })
        .collect();

    let undecided = |reason: String| Reading {
        verdict: Verdict::Undecided { reason },
        failing_rule: None,
        rules: rules.clone(),
        grants: grants.clone(),
        authorizing_grants: Vec::new(),
    };
    if !report.complete {
        return undecided("Isabelle did not finish the check".to_owned());
    }
    let lemma_lines: HashSet<usize> = theory.lemmas.iter().map(|lemma| lemma.line).collect();
    if let Some((line, message)) = report
        .errors
        .iter()
        .find(|(line, _)| !lemma_lines.contains(line))
    {
        return undecided(format!(
            "the check theory failed outside a lemma, at line {line}: {}",
            message.lines().next().unwrap_or("")
        ));
    }
    if let Some(result) = rules
        .iter()
        .find(|result| result.outcome == RuleOutcome::Undecided)
    {
        return undecided(format!(
            "{}: Isabelle proved neither, or both, of the rule and its negation",
            result.rule.name()
        ));
    }
    if let Some((index, grant)) = grants
        .iter()
        .enumerate()
        .find(|(_, grant)| grant.outcome == RuleOutcome::Undecided)
    {
        return undecided(format!(
            "grant {index} ({}): Isabelle proved neither, or both, that it authorizes the \
             action and that it does not",
            grant.id
        ));
    }
    let outcome = implied_outcome(&rules);
    let authorizing_grants: Vec<String> = grants
        .iter()
        .filter(|grant| grant.outcome == RuleOutcome::Holds)
        .map(|grant| grant.id.clone())
        .collect();
    if outcome == Outcome::Permit && authorizing_grants.is_empty() && !theory.needs_no_grant {
        return undecided(
            "the rules permit the action but no grant was proven to authorize it, and the \
             action needs one"
                .to_owned(),
        );
    }
    let failing_rule = rules
        .iter()
        .find(|result| result.outcome == RuleOutcome::Fails)
        .map(|result| result.rule);
    Reading {
        verdict: Verdict::Decided { outcome },
        failing_rule,
        rules,
        grants,
        authorizing_grants,
    }
}
