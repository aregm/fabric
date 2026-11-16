//! Authorizations across a restart, retries and the clock (design 5.1, 5.7, 5.8, D16, D34;
//! section 9.3 c05, c26, c29). Gate G2a rows DEF-4, "Expire grants; restart" and "Kill Isabelle /
//! budget (store half)" (design 7.3).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::RetryPolicy;
use fabric_broker_core::ports::AttemptReport;
use fabric_desktop_contracts::Limits;
use fabric_desktop_contracts::owner::ProofPurpose;
use fabric_storage::AuthorizationState;
use serde_json::json;

/// c05. Design DEF-4, 5.8 and 3.5 (`authorizations`): an authorization from before a restart is
/// never handed off, even within its 60 s: a consumed one stays consumed and its call is not
/// repeated; an issued one is voided `broker_restart` and a new proof issues a new one.
#[test]
fn an_authorization_from_before_a_restart_is_never_handed_off() {
    // Consumed stays consumed.
    let mut h = Harness::new("c05a", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let order = h.dispatch(&fresh, &[&grant]);
    h.kill();
    h.advance(10_000);
    let output = h.start();
    assert!(
        dispatches(&output).is_empty(),
        "a consumed authorization was handed off again"
    );
    assert_eq!(h.state(&request_id), "outcome_unknown");
    h.kill();
    let authorization = h
        .read(|r| r.authorization(&order.authorization.authorization_id))
        .expect("the authorization stays");
    assert_eq!(authorization.state, AuthorizationState::Consumed);
    h.verify();

    // Issued is voided.
    let mut h = Harness::new("c05b", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    h.inject_fault("request.dispatch");
    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(output.halt.is_some());
    h.kill();
    h.clear_faults();
    let issued = h
        .read(|r| r.latest_authorization_of(&id(&request_id)))
        .expect("an authorization was issued");
    assert_eq!(issued.state, AuthorizationState::Issued);

    h.advance(10_000);
    let output = h.start();
    assert!(
        dispatches(&output).is_empty(),
        "an issued authorization outlived its run"
    );
    let fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    let order = h.dispatch(&fresh, &[&grant]);
    assert_ne!(
        order.authorization.authorization_id, issued.authorization.authorization_id,
        "the old authorization was handed off"
    );
    h.kill();
    let old = h
        .read(|r| r.authorization(&issued.authorization.authorization_id))
        .expect("the old authorization stays");
    assert_eq!(old.state, AuthorizationState::Voided);
    assert_eq!(
        h.void_reason(issued.authorization.authorization_id.as_str())
            .as_deref(),
        Some("broker_restart")
    );
    h.verify();
}

/// c26. Design 5.7 (`dispatching -> retry_wait`), D16 and `RetryPolicy::PLAN`: transient
/// failures wait 2, 4 and 8 s; the fourth transient attempt ends `terminal_failure` with
/// `retry_budget_exhausted`; a provider hint that would wake after the 120 s budget ends it at
/// once.
#[test]
fn the_retry_budget_ends_in_retry_budget_exhausted() {
    let mut h = Harness::new("c26", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let mut order = h.dispatch(&fresh, &[&grant]);
    assert_eq!(order.attempt, 1);

    for (n, wait) in RetryPolicy::PLAN.backoff_ms.into_iter().enumerate() {
        let output = h.attempt(
            &order,
            AttemptReport::Transient {
                retry_after_ms: None,
            },
        );
        assert!(output.jobs.is_empty());
        assert_eq!(h.state(&request_id), "retry_wait");
        assert_eq!(h.broker().next_due_ms(), Some(h.now + wait));

        h.advance(wait - 1);
        let output = h.tick();
        assert!(output.jobs.is_empty(), "a retry started early");
        h.advance(1);
        let output = h.tick();
        let job = job_for(&output, &request_id, ProofPurpose::FreshPermit);
        order = h.dispatch(&job, &[&grant]);
        assert_eq!(order.attempt, u32::try_from(n).expect("a small number") + 2);
    }

    let output = h.attempt(
        &order,
        AttemptReport::Transient {
            retry_after_ms: None,
        },
    );
    assert!(output.jobs.is_empty());
    let details = h.details(&request_id);
    assert_eq!(
        details["request"]["status"]["state"],
        json!("terminal_failure")
    );
    assert_eq!(
        details["request"]["status"]["failure"]["code"],
        json!("retry_budget_exhausted")
    );
    let attempts = details["attempts"].as_array().expect("attempts").clone();
    assert_eq!(attempts.len(), 4);
    assert!(
        attempts
            .iter()
            .all(|a| a["outcome"] == json!("transient_failure"))
    );

    // A hint past the budget.
    let (hinted, output) = h.propose(1, "call-2");
    let job = job_for(&output, &hinted, ProofPurpose::Classification);
    let output = h.prove(&job, Outcome::Permit, &[&grant]);
    let fresh = job_for(&output, &hinted, ProofPurpose::FreshPermit);
    let order = h.dispatch(&fresh, &[&grant]);
    let output = h.attempt(
        &order,
        AttemptReport::Transient {
            retry_after_ms: Some(RetryPolicy::PLAN.budget_ms + 1_000),
        },
    );
    assert!(output.jobs.is_empty());
    let details = h.details(&hinted);
    assert_eq!(
        details["request"]["status"]["state"],
        json!("terminal_failure")
    );
    assert_eq!(
        details["request"]["status"]["failure"]["code"],
        json!("retry_budget_exhausted")
    );
    h.verify();
}

/// c29. Design D34 and 5.1 (clock): a wall clock that goes back mid-run neither halts the broker
/// nor lowers time: the step reports the change on stderr, new records carry the last time, the
/// ledger's times never decrease, and lifetimes run on at wall rate from the floored clock.
///
/// Section 9.3 checks the lifetime with an authorization issued before the step. An issued
/// authorization never survives a step through the public API (5.1: an accepted permit is
/// dispatched in the same step), so the 24 h approval card stands in for it.
#[test]
fn a_backward_clock_step_neither_halts_nor_lowers_time() {
    let mut h = Harness::new("c29", isabelle());
    h.setup();
    let card = h.pending(1, "call-1");
    let opened = h.now;
    let step = 600_000;

    h.now = opened - step;
    let output = h.tick();
    assert!(output.halt.is_none(), "a backward clock halted the broker");
    assert_eq!(
        output.stderr.len(),
        1,
        "the clock change was not reported once"
    );

    let (later, output) = h.propose(1, "call-2");
    assert!(output.halt.is_none());
    assert_eq!(
        h.details(&later)["request"]["received_at"],
        timestamp(opened),
        "a record was dated before the last commit"
    );

    let expires = opened + Limits::CD_G0.approval_expiry_seconds.get() * 1_000;
    h.now = expires - step - 1;
    let _ = h.tick();
    assert_eq!(h.state(&card), "pending_approval");
    h.now = expires - step + 1;
    let _ = h.tick();
    assert_eq!(h.state(&card), "expired");

    let ledger = h.ledger();
    assert!(
        ledger
            .windows(2)
            .all(|w| w[0].committed_at_ms <= w[1].committed_at_ms),
        "the ledger's time went back"
    );
    h.verify();
}
