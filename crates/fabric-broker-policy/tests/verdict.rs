//! Reading Isabelle's report on a check theory fails closed: every way a result can be
//! incomplete, stray or contradictory is undecided. The context is the golden fixture's,
//! whose grants are `g-invite`, `g-once` and `g-standing`.

mod common;

use fabric_broker_policy::check::{
    self, CheckTheory, NodeReport, Polarity, RuleOutcome, Statement, Verdict, implied_outcome,
};
use fabric_broker_policy::{Action, Context, Outcome, Rule};

fn theory() -> CheckTheory {
    check::generate("Check_1", &common::context(), &common::action()).unwrap()
}

/// Isabelle's report when exactly `failing` fail, every other rule holds, and exactly the
/// grants at `authorizing` authorize the action.
fn report_with(theory: &CheckTheory, failing: &[Rule], authorizing: &[usize]) -> NodeReport {
    let errors = theory
        .lemmas
        .iter()
        .filter(|lemma| match lemma.statement {
            Statement::Rule { rule, polarity } => {
                failing.contains(&rule) == (polarity == Polarity::Holds)
            }
            Statement::Grant { index, authorizes } => authorizing.contains(&index) != authorizes,
        })
        .map(|lemma| (lemma.line, "Failed to finish proof".to_owned()))
        .collect();
    NodeReport {
        complete: true,
        errors,
        writeln: Vec::new(),
    }
}

/// As [`report_with`], with the first grant, `g-invite`, authorizing.
fn report(theory: &CheckTheory, failing: &[Rule]) -> NodeReport {
    report_with(theory, failing, &[0])
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

fn grant_line(theory: &CheckTheory, index: usize, authorizes: bool) -> usize {
    line(theory, Statement::Grant { index, authorizes })
}

#[test]
fn the_authorizing_grants_are_listed_in_context_order() {
    let theory = theory();
    assert_eq!(theory.grant_ids, ["g-invite", "g-once", "g-standing"]);
    for (authorizing, expected) in [
        (vec![0], vec!["g-invite"]),
        (vec![1, 0], vec!["g-invite", "g-once"]),
        (vec![2], vec!["g-standing"]),
        (vec![0, 1, 2], vec!["g-invite", "g-once", "g-standing"]),
    ] {
        let reading = check::read(&theory, &report_with(&theory, &[], &authorizing));
        assert!(reading.verdict.is_permit());
        assert_eq!(reading.authorizing_grants, expected);
        assert_eq!(reading.grants.len(), 3);
        for (index, grant) in reading.grants.iter().enumerate() {
            let expected = if authorizing.contains(&index) {
                RuleOutcome::Holds
            } else {
                RuleOutcome::Fails
            };
            assert_eq!(grant.outcome, expected);
        }
    }
}

#[test]
fn a_grant_proven_both_ways_or_neither_way_is_undecided() {
    let theory = theory();
    for index in 0..3 {
        let mut both = report(&theory, &[]);
        let refused = grant_line(&theory, index, false);
        let authorizes = grant_line(&theory, index, true);
        both.errors
            .retain(|(line, _)| *line != refused && *line != authorizes);
        assert!(is_undecided(&theory, &both), "grant {index} both");

        let mut neither = report(&theory, &[]);
        neither
            .errors
            .push((authorizes, "Failed to finish proof".to_owned()));
        neither
            .errors
            .push((refused, "Failed to finish proof".to_owned()));
        assert!(is_undecided(&theory, &neither), "grant {index} neither");

        // An undecided grant makes even a block undecided.
        let mut blocked = report(&theory, &[Rule::Actor]);
        blocked
            .errors
            .retain(|(line, _)| *line != refused && *line != authorizes);
        assert!(is_undecided(&theory, &blocked), "grant {index} in a block");
    }
}

#[test]
fn a_permit_no_grant_authorizes_is_undecided_when_the_action_needs_one() {
    let theory = theory();
    let reading = check::read(&theory, &report_with(&theory, &[], &[]));
    assert!(matches!(reading.verdict, Verdict::Undecided { .. }));
    assert!(reading.authorizing_grants.is_empty());
    assert_eq!(reading.grants.len(), 3);

    // A refusal needs no authorizing grant.
    for (failing, outcome) in [
        (vec![Rule::Authority], Outcome::NeedApproval),
        (vec![Rule::AgentNotPaused], Outcome::Block),
    ] {
        let reading = check::read(&theory, &report_with(&theory, &failing, &[]));
        assert_eq!(reading.verdict.outcome(), Some(outcome));
        assert!(reading.authorizing_grants.is_empty());
    }
}

fn theory_for(context: &Context, action: &Action) -> CheckTheory {
    check::generate("Check_1", context, action).unwrap()
}

#[test]
fn a_permit_that_needs_no_grant_may_rest_on_none() {
    let s = |text: &str| text.to_owned();
    let create = Action::CreateEvent {
        account: s("acct-1"),
        calendar: s("team"),
    };
    let read = Action::ListCalendars;
    let refresh = Action::SystemRefresh {
        account: s("acct-1"),
        calendar: s("team"),
    };
    let reconcile = Action::SystemReconcile {
        account: s("acct-1"),
        calendar: s("team"),
        event_id: s("e"),
    };
    let no_grants = Context {
        grants: Vec::new(),
        ..common::context()
    };
    for (action, needs_none) in [
        (refresh, true),
        (reconcile, true),
        (create.clone().disclosed(), true),
        (common::action().disclosed(), true),
        (read.clone().disclosed().disclosed(), true),
        (read.clone().disclosed(), false),
        (
            Action::ReadAvailability {
                account: s("acct-1"),
                calendars: vec![s("team")],
                start_ms: 0,
                end_ms: 1,
            }
            .disclosed(),
            false,
        ),
        (create, false),
        (read, false),
        (common::action(), false),
    ] {
        assert_eq!(action.needs_no_grant(), needs_none, "{action:?}");
        for context in [common::context(), no_grants.clone()] {
            let theory = theory_for(&context, &action);
            assert_eq!(theory.needs_no_grant, needs_none);
            let reading = check::read(&theory, &report_with(&theory, &[], &[]));
            assert_eq!(reading.verdict.is_permit(), needs_none, "{action:?}");
            assert_eq!(
                matches!(reading.verdict, Verdict::Undecided { .. }),
                !needs_none,
                "{action:?}"
            );
        }
    }
}

#[test]
fn an_undecided_reading_names_no_authorizing_grant() {
    let theory = theory();
    let mut incomplete = report_with(&theory, &[], &[0, 1]);
    incomplete.complete = false;
    let reading = check::read(&theory, &incomplete);
    assert!(matches!(reading.verdict, Verdict::Undecided { .. }));
    assert!(reading.authorizing_grants.is_empty());
    assert_eq!(reading.grants[0].outcome, RuleOutcome::Holds);

    let mut stray = report_with(&theory, &[], &[0]);
    stray.errors.push((1, "Bad theory import".to_owned()));
    assert!(check::read(&theory, &stray).authorizing_grants.is_empty());
}

#[test]
fn a_context_without_grants_states_no_grant_lemmas() {
    let context = Context {
        grants: Vec::new(),
        ..common::context()
    };
    let theory = theory_for(&context, &common::action());
    assert!(theory.grant_ids.is_empty());
    assert_eq!(theory.lemmas.len(), 2 * Rule::ALL.len());
    assert!(!theory.text.contains("authorizes"));
}
