//! Reading Isabelle's report on a check theory fails closed: every way a result can be
//! incomplete, stray or contradictory is undecided.

mod common;

use fabric_broker_policy::check::{
    self, CheckTheory, NodeReport, Polarity, RuleOutcome, Statement, Verdict, implied_outcome,
};
use fabric_broker_policy::{Outcome, Rule};

fn theory() -> CheckTheory {
    check::generate("Check_1", &common::context(), &common::action()).unwrap()
}

/// Isabelle's report when exactly `failing` fail and every other rule holds.
fn report(theory: &CheckTheory, failing: &[Rule]) -> NodeReport {
    let errors = theory
        .lemmas
        .iter()
        .filter(|lemma| match lemma.statement {
            Statement::Rule { rule, polarity } => {
                failing.contains(&rule) == (polarity == Polarity::Holds)
            }
        })
        .map(|lemma| (lemma.line, "Failed to finish proof".to_owned()))
        .collect();
    NodeReport {
        complete: true,
        errors,
        writeln: Vec::new(),
    }
}

fn line(theory: &CheckTheory, statement: Statement) -> usize {
    theory.line_of(statement).unwrap()
}

fn rule_line(theory: &CheckTheory, rule: Rule, polarity: Polarity) -> usize {
    line(theory, Statement::Rule { rule, polarity })
}

fn is_undecided(theory: &CheckTheory, report: &NodeReport) -> bool {
    let reading = check::read(theory, report);
    matches!(reading.verdict, Verdict::Undecided { .. }) && reading.failing_rule.is_none()
}

#[test]
fn every_rule_holding_permits() {
    let theory = theory();
    let reading = check::read(&theory, &report(&theory, &[]));
    assert!(reading.verdict.is_permit());
    assert_eq!(reading.failing_rule, None);
    assert_eq!(reading.rules.len(), Rule::ALL.len());
    assert!(
        reading
            .rules
            .iter()
            .all(|r| r.outcome == RuleOutcome::Holds)
    );
}

#[test]
fn a_failing_block_rule_blocks_with_the_first_failing_rule() {
    let theory = theory();
    let failing = [Rule::NoStrangers, Rule::RevisionCurrent, Rule::EventKnown];
    let reading = check::read(&theory, &report(&theory, &failing));
    assert_eq!(reading.verdict.outcome(), Some(Outcome::Block));
    assert_eq!(reading.failing_rule, Some(Rule::EventKnown));
}

#[test]
fn a_failing_approval_rule_alone_asks() {
    let theory = theory();
    let failing = [Rule::Authority, Rule::InvitationAuthority];
    let reading = check::read(&theory, &report(&theory, &failing));
    assert_eq!(reading.verdict.outcome(), Some(Outcome::NeedApproval));
    assert_eq!(reading.failing_rule, Some(Rule::InvitationAuthority));
}

#[test]
fn the_implied_outcome_follows_the_theorys_outcome() {
    let theory = theory();
    for (failing, expected) in [
        (vec![], Outcome::Permit),
        (vec![Rule::Authority], Outcome::NeedApproval),
        (vec![Rule::SystemAuthority], Outcome::Block),
        (vec![Rule::Actor, Rule::NoStrangers], Outcome::Block),
    ] {
        let reading = check::read(&theory, &report(&theory, &failing));
        assert_eq!(implied_outcome(&reading.rules), expected);
        assert_eq!(reading.verdict.outcome(), Some(expected));
    }
}

#[test]
fn an_incomplete_node_is_undecided() {
    let theory = theory();
    let mut incomplete = report(&theory, &[]);
    incomplete.complete = false;
    assert!(is_undecided(&theory, &incomplete));
    // A node Isabelle never reported: no errors at all, so every lemma would look proven.
    assert!(is_undecided(&theory, &NodeReport::default()));
    let everything_proven = NodeReport {
        complete: true,
        ..NodeReport::default()
    };
    assert!(is_undecided(&theory, &everything_proven));
}

#[test]
fn an_error_outside_a_lemma_is_undecided() {
    let theory = theory();
    for stray in [0, 1, 5, 7, 8, theory.text.lines().count(), 10_000] {
        let mut stray_report = report(&theory, &[]);
        stray_report
            .errors
            .push((stray, "Type unification failed".to_owned()));
        assert!(is_undecided(&theory, &stray_report), "line {stray}");
    }
}

#[test]
fn a_rule_proven_both_ways_or_neither_way_is_undecided() {
    let theory = theory();
    for rule in Rule::ALL {
        let mut both = report(&theory, &[]);
        let fails = rule_line(&theory, rule, Polarity::Fails);
        both.errors.retain(|(line, _)| *line != fails);
        assert!(is_undecided(&theory, &both), "{} both", rule.name());

        let mut neither = report(&theory, &[]);
        neither.errors.push((
            rule_line(&theory, rule, Polarity::Holds),
            "Failed to finish proof".to_owned(),
        ));
        assert!(is_undecided(&theory, &neither), "{} neither", rule.name());
    }
}

#[test]
fn an_undecided_reading_keeps_the_rule_results() {
    let theory = theory();
    let mut incomplete = report(&theory, &[Rule::Authority]);
    incomplete.complete = false;
    let reading = check::read(&theory, &incomplete);
    assert!(matches!(reading.verdict, Verdict::Undecided { .. }));
    assert_eq!(reading.rules.len(), Rule::ALL.len());
    assert_eq!(reading.failing_rule, None);
}
