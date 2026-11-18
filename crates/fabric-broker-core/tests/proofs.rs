//! Prover results: what is accepted, what is refused, relied grants and re-proof counters
//! (design 5.7, D13, D14, D16, D17, D36, L7; section 9.3 c18, c19, c28, c31, c33, c35, c36). Gate G2a rows
//! "Kill Isabelle / budget (store half)" and DEF-7 (design 7.3). The cause of every move to
//! `undecided` and the details' `undecided_reason` (5.7; PW 2.4).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::PROOF_WATCHDOG_MS;
use fabric_broker_core::ports::{AttemptReport, ProverCause};
use fabric_desktop_contracts::owner::ProofPurpose;
use fabric_desktop_contracts::scalar::Digest;
use fabric_storage::ProverKind;
use serde_json::{Value, json};

fn relied_in_store(h: &mut Harness, request_id: &str) -> Vec<String> {
    let request = h
        .read(|r| r.request(&id(request_id)))
        .unwrap_or_else(|| panic!("no request {request_id}"));
    strings(&request.relied_grant_ids)
}

fn other_digest() -> Digest {
    Digest::new(format!("sha256:{}", "e".repeat(64))).expect("a digest")
}

fn proof_ids(h: &mut Harness, request_id: &str) -> Vec<Value> {
    h.details(request_id)["proof_ids"]
        .as_array()
        .expect("proof ids")
        .clone()
}

/// The `undecided_reason` of a request's redacted details.
fn reason(h: &mut Harness, request_id: &str) -> Value {
    h.details(request_id)["undecided_reason"].clone()
}

/// Asserts that a request is `undecided`, with `cause` stored on its move and `expected` in its
/// details.
fn assert_undecided(h: &mut Harness, request_id: &str, cause: &str, expected: Value) {
    let details = h.details(request_id);
    assert_eq!(details["request"]["status"]["state"], json!("undecided"));
    assert_eq!(
        h.stored_cause(request_id).as_deref(),
        Some(cause),
        "{request_id}"
    );
    assert_eq!(
        details["undecided_reason"], expected,
        "{request_id}: {cause}"
    );
}

/// The payload of one forged move of `request_id` to `undecided`; `cause` is written as given,
/// and left out when `None`.
fn forged_move(request_id: &str, cause: Option<&str>) -> String {
    let mut moved = json!({
        "at_ms": T0, "due_at_ms": null, "op": "request_moved", "request_id": request_id,
        "revision": 99, "status": {"state": "undecided"}
    });
    if let Some(cause) = cause {
        moved["undecided_cause"] = json!(cause);
    }
    json!({"mutations": [moved]}).to_string()
}

/// c18. Design D14 and 5.7: with no prover attached every request ends `undecided` at once,
/// with no proof job and no report.
#[test]
fn no_prover_ends_every_request_undecided_with_no_report() {
    let mut h = Harness::new("c18a", no_prover());
    h.setup();
    let mut requests = Vec::new();
    for call in ["call-1", "call-2"] {
        let (request_id, output) = h.propose(1, call);
        assert!(proof_jobs(&output).is_empty(), "a job without a prover");
        let details = h.details(&request_id);
        assert_eq!(details["request"]["status"]["state"], json!("undecided"));
        assert_eq!(details["proof_ids"], json!([]));
        assert_undecided(
            &mut h,
            &request_id,
            "no_prover",
            json!("prover_unavailable"),
        );
        requests.push(request_id);
    }
    // A run with no prover says so: unavailable, not configured.
    assert_eq!(
        h.snapshot()["health"],
        json!({"prover": "unavailable", "prover_problem": "not_configured", "store": "ready",
            "agent_channel": "not_listening", "agent_channel_problem": null})
    );
    h.kill();
    for request_id in &requests {
        for kind in [ProverKind::Isabelle, ProverKind::Synthetic] {
            assert!(
                h.read(|r| r.proof_ids_of(&id(request_id), kind, 200))
                    .is_empty()
            );
        }
    }
    h.verify();
}

/// c18. Design D13 and 6.1 `get_proof`: only an Isabelle permit can lead anywhere; a permit from
/// the synthetic classifier ends `undecided`, and its report is never shown as a proof.
#[test]
fn a_synthetic_permit_is_never_accepted() {
    let mut h = Harness::new("c18b", synthetic());
    h.setup();
    let (request_id, output) = h.propose(1, "call-1");
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let done = proof_done(&job, Outcome::Permit, &[]);
    let proof = proof_id(&done);
    let output = h.prove_with(done);
    assert!(output.halt.is_none());
    assert!(
        jobs_of(&output, &request_id).is_empty(),
        "a synthetic permit moved on"
    );
    assert_eq!(h.state(&request_id), "undecided");
    assert_undecided(
        &mut h,
        &request_id,
        "no_prover",
        json!("prover_unavailable"),
    );
    assert_eq!(proof_ids(&mut h, &request_id), Vec::<Value>::new());
    let refusal = h.refused("get_proof", json!({"proof_id": proof}));
    assert_eq!(refusal["code"], json!("not_found"));
    h.verify();
}

/// c18. Design 5.7 and D16: a proof not back within 35 s makes the request `undecided`; a
/// result that arrives later is ignored and not recorded.
#[test]
fn a_proof_past_35_s_is_undecided_and_a_late_result_is_ignored() {
    let mut h = Harness::new("c18c", isabelle());
    h.setup();
    let (request_id, output) = h.propose(1, "call-1");
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    assert_eq!(job.deadline_ms, job.started_at_ms + PROOF_WATCHDOG_MS);

    h.now = job.started_at_ms + PROOF_WATCHDOG_MS - 1;
    let _ = h.tick();
    assert_eq!(h.state(&request_id), "proving_classification");

    h.now = job.deadline_ms + 1;
    let _ = h.tick();
    assert_undecided(&mut h, &request_id, "deadline_passed", json!("out_of_time"));

    let head = h.ledger_head();
    let output = h.prove(&job, Outcome::Permit, &[]);
    assert!(output.halt.is_none());
    assert!(output.jobs.is_empty());
    assert_eq!(h.state(&request_id), "undecided");
    assert_eq!(h.ledger_head(), head, "a late result was recorded");
    assert_undecided(&mut h, &request_id, "deadline_passed", json!("out_of_time"));
    assert_eq!(proof_ids(&mut h, &request_id), Vec::<Value>::new());
    h.verify();
}

/// c18. Design D17 and 5.7: a fresh permit cannot authorize when no provider is attached; the
/// request ends `undecided` with no authorization.
#[test]
fn permit_without_a_provider_is_undecided() {
    let mut h = Harness::new("c18d", isabelle_without_provider());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(output.halt.is_none());
    assert!(dispatches(&output).is_empty());
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("undecided"));
    assert_eq!(details["authorization"], Value::Null);
    // The proof decided; the broker could not act, which the contract has no value for.
    assert_undecided(&mut h, &request_id, "no_provider", Value::Null);
    h.verify();
}

/// c18. Design L7(d) and D17: a fresh permit that names no authorizing grant
/// issues no authorization; the request ends `undecided`.
#[test]
fn a_fresh_permit_with_no_authorizing_grant_is_undecided() {
    let mut h = Harness::new("c18e", isabelle());
    h.setup();
    let (request_id, output) = h.propose(1, "call-1");
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let output = h.prove(&job, Outcome::Permit, &[]);
    let fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert!(relied(&fresh).is_empty());

    let output = h.prove(&fresh, Outcome::Permit, &[]);
    assert!(output.halt.is_none());
    assert!(dispatches(&output).is_empty());
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("undecided"));
    assert_eq!(details["authorization"], Value::Null);
    assert_undecided(&mut h, &request_id, "no_grant", json!("failed"));
    h.verify();
}

/// c18. Design 5.7 (grant check) and D17: a result naming a grant outside its job's facts is
/// refused before anything else, for any purpose and verdict: recorded, relied list unchanged,
/// request `undecided`, never a halt, and a restart repeats nothing.
#[test]
fn a_result_naming_a_grant_outside_its_facts_is_undecided_and_never_halts() {
    let mut h = Harness::new("c18f", isabelle());
    let _ = h.seed(&[AGENT, OTHER], vec![account(ACCOUNT, &[CALENDAR])]);
    let _ = h.connect(1, AGENT);
    let _ = h.connect(2, OTHER);
    let (_, others_grant, _) = h.approved(2, "other-1", "until_revoked");

    // A classification permit naming another agent's grant.
    let (classified, output) = h.propose(1, "call-a");
    let job = job_for(&output, &classified, ProofPurpose::Classification);
    let done = proof_done(&job, Outcome::Permit, &[&others_grant]);
    let first = proof_id(&done);
    let output = h.prove_with(done);
    assert!(output.halt.is_none());
    assert!(jobs_of(&output, &classified).is_empty());
    assert_eq!(h.state(&classified), "undecided");
    assert!(proof_ids(&mut h, &classified).contains(&json!(first)));

    // A fresh permit naming an unknown id.
    let (permitted, grant_b, fresh) = h.approved(1, "call-b", "until_revoked");
    let done = proof_done(&fresh, Outcome::Permit, &["grant-unknown"]);
    let second = proof_id(&done);
    let output = h.prove_with(done);
    assert!(output.halt.is_none());
    assert!(output.jobs.is_empty());
    assert_eq!(h.state(&permitted), "undecided");
    assert!(proof_ids(&mut h, &permitted).contains(&json!(second)));

    // A fresh Block naming one.
    let (blocked, grant_c, fresh) = h.approved(1, "call-c", "until_revoked");
    let done = proof_done(&fresh, Outcome::Block, &["grant-unknown"]);
    let output = h.prove_with(done);
    assert!(output.halt.is_none());
    assert_eq!(h.state(&blocked), "undecided");
    for request in [&classified, &permitted, &blocked] {
        assert_undecided(&mut h, request, "report_unusable", json!("failed"));
    }

    h.kill();
    assert!(relied_in_store(&mut h, &classified).is_empty());
    assert_eq!(relied_in_store(&mut h, &permitted), vec![grant_b]);
    assert_eq!(relied_in_store(&mut h, &blocked), vec![grant_c]);
    let recorded = h
        .read(|r| r.proof(&id(&second)))
        .expect("the refused result is recorded");
    assert_eq!(
        strings(&recorded.authorizing_grants),
        vec!["grant-unknown".to_owned()]
    );
    assert_eq!(
        recorded.undecided_reason.as_deref(),
        Some("authorizing grant outside the facts")
    );

    h.advance(1_000);
    let output = h.start();
    assert!(output.halt.is_none());
    for request in [&classified, &permitted, &blocked] {
        assert!(
            jobs_of(&output, request).is_empty(),
            "a restart repeated a refused result"
        );
        assert_eq!(h.state(request), "undecided");
    }
    h.verify();
}

/// c18. Design 5.7 (a prover result never becomes a `StoreFault`) and PLAN 8 (proof binding):
/// a report whose request, purpose or action digest is not its job's proves nothing about the
/// request. It neither authorizes nor halts: nothing is recorded, the request is `undecided`, and
/// no proof is filed under the request the report names.
#[test]
fn a_report_bound_to_another_job_neither_authorizes_nor_halts() {
    let mut h = Harness::new("c18g", isabelle());
    h.setup();
    let (other, _) = h.propose(1, "call-other");

    // A fresh Permit for one request whose report names another, the classification purpose
    // and another action.
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let mut done = proof_done(&fresh, Outcome::Permit, &[&grant]);
    let report = done.report.as_mut().expect("a report");
    report.request_id = id(&other);
    report.purpose = ProofPurpose::Classification;
    report.action_digest = other_digest();
    let proof = proof_id(&done);
    let head = h.ledger_head();
    let output = h.prove_with(done);
    assert!(output.halt.is_none());
    assert!(
        dispatches(&output).is_empty(),
        "an unbound report dispatched"
    );
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("undecided"));
    assert_eq!(details["authorization"], Value::Null);
    assert!(!proof_ids(&mut h, &request_id).contains(&json!(proof)));
    assert!(!proof_ids(&mut h, &other).contains(&json!(proof)));
    assert!(h.ledger_head() > head, "the request did not move");
    assert_undecided(&mut h, &request_id, "report_unusable", json!("failed"));

    // Each field alone unbinds the report.
    for field in ["request_id", "purpose", "action_digest"] {
        let (request_id, grant, fresh) = h.approved(1, &format!("call-{field}"), "until_revoked");
        let mut done = proof_done(&fresh, Outcome::Permit, &[&grant]);
        let report = done.report.as_mut().expect("a report");
        match field {
            "request_id" => report.request_id = id("request-unknown"),
            "purpose" => report.purpose = ProofPurpose::Classification,
            _ => report.action_digest = other_digest(),
        }
        let proof = proof_id(&done);
        let output = h.prove_with(done);
        assert!(output.halt.is_none(), "{field}: a mismatched report halted");
        assert!(dispatches(&output).is_empty(), "{field}: it dispatched");
        assert_eq!(h.state(&request_id), "undecided", "{field}");
        assert_undecided(&mut h, &request_id, "report_unusable", json!("failed"));
        assert!(
            !proof_ids(&mut h, &request_id).contains(&json!(proof)),
            "{field}: the report was recorded"
        );
    }

    h.advance(1_000);
    let output = h.restart();
    assert!(output.halt.is_none(), "a restart replayed a halt");
    h.verify();
}

/// c19. Design DEF-7 (store half) and 6.1 `get_proof`: a blocked request keeps its ruling and
/// its proof across a restart.
#[test]
fn a_blocked_request_keeps_rule_and_proof_after_restart() {
    let mut h = Harness::new("c19", isabelle());
    h.setup();
    let (request_id, output) = h.propose(1, "call-1");
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let done = proof_done(&job, Outcome::Block, &[]);
    let proof = proof_id(&done);
    let _ = h.prove_with(done);
    let before = h.details(&request_id);
    assert_eq!(before["request"]["status"]["state"], json!("blocked"));
    let denial = before["request"]["status"]["denial"].clone();
    assert_eq!(denial["rule"], json!(RULE));
    assert_eq!(denial["proof_id"], json!(proof));

    h.advance(1_000);
    let _ = h.restart();
    let after = h.details(&request_id);
    assert_eq!(after["request"]["status"]["state"], json!("blocked"));
    assert_eq!(after["request"]["status"]["denial"], denial);
    assert!(
        after["proof_ids"]
            .as_array()
            .expect("ids")
            .contains(&json!(proof))
    );
    let report = h.ok("get_proof", json!({"proof_id": proof}));
    assert_eq!(report["outcome"], json!("block"));
    assert_eq!(report["failing_rule"], json!(RULE));
    assert_eq!(report["request_id"], json!(request_id));
    h.verify();
}

/// c19b. Design DEF-7 (store half) and 6.1 `get_proof`, for an action denied when authorizing:
/// a request whose fresh-permit proof blocks keeps the failing rule and the link to that proof
/// across a restart.
#[test]
fn a_request_blocked_by_its_fresh_proof_keeps_rule_and_proof_after_restart() {
    let mut h = Harness::new("c19b", isabelle());
    h.setup();
    let (request_id, _grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let done = proof_done(&fresh, Outcome::Block, &[]);
    let proof = proof_id(&done);
    let output = h.prove_with(done);
    assert!(output.halt.is_none());
    assert!(dispatches(&output).is_empty());
    let before = h.details(&request_id);
    assert_eq!(before["request"]["status"]["state"], json!("blocked"));
    assert_eq!(before["authorization"], Value::Null);
    let denial = before["request"]["status"]["denial"].clone();
    assert_eq!(denial["rule"], json!(RULE));
    assert_eq!(denial["proof_id"], json!(proof));

    h.advance(1_000);
    let _ = h.restart();
    let after = h.details(&request_id);
    assert_eq!(after["request"]["status"]["state"], json!("blocked"));
    assert_eq!(after["request"]["status"]["denial"], denial);
    assert!(
        after["proof_ids"]
            .as_array()
            .expect("ids")
            .contains(&json!(proof))
    );
    let report = h.ok("get_proof", json!({"proof_id": proof}));
    assert_eq!(report["outcome"], json!("block"));
    assert_eq!(report["failing_rule"], json!(RULE));
    assert_eq!(report["request_id"], json!(request_id));
    assert_eq!(report["purpose"], json!("fresh_permit"));
    h.verify();
}

/// c28. Design D36 and 5.7: an `allowed` request rests on the classification's authorizing
/// grants; its fresh proof is offered exactly that list.
#[test]
fn an_allowed_request_rests_on_the_classification_grants() {
    let mut h = Harness::new("c28a", isabelle());
    h.setup();
    let (_, grant, _) = h.approved(1, "call-0", "until_revoked");
    let (request_id, output) = h.propose(1, "call-1");
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    assert!(
        relied(&job).is_empty(),
        "a classification was offered a relied list"
    );
    let output = h.prove(&job, Outcome::Permit, &[&grant]);
    let fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(relied(&fresh), vec![grant.clone()]);
    h.kill();
    assert_eq!(relied_in_store(&mut h, &request_id), vec![grant]);
    h.verify();
}

/// c28. Design D36 and 6.1 `decide_request`: an `approved` request rests on exactly the grant
/// its approval created.
#[test]
fn an_approved_request_rests_on_the_approval_grant() {
    let mut h = Harness::new("c28b", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    assert_eq!(relied(&fresh), vec![grant.clone()]);
    h.kill();
    assert_eq!(relied_in_store(&mut h, &request_id), vec![grant]);
    h.verify();
}

/// c28. Design L7(c) and 5.7 (`proving_fresh_permit -> queued`): the authorization names the
/// first authorizing grant, and the store keeps the whole list.
#[test]
fn the_authorization_names_the_first_authorizing_grant_and_the_store_keeps_the_list() {
    let mut h = Harness::new("c28c", isabelle());
    h.setup();
    let first = h.pending(1, "call-1");
    let (_, second_grant, _) = h.approved(1, "call-2", "until_revoked");
    let (grant, fresh) = h.approve(&first, "until_revoked");

    let order = h.dispatch(&fresh, &[&second_grant, &grant]);
    assert_eq!(order.authorization.grant_id.as_str(), second_grant);
    h.kill();
    let authorization = h
        .read(|r| r.latest_authorization_of(&id(&first)))
        .expect("the authorization");
    assert_eq!(
        strings(&authorization.grant_ids),
        vec![second_grant.clone(), grant.clone()]
    );
    assert_eq!(authorization.authorization.grant_id.as_str(), second_grant);
    assert_eq!(relied_in_store(&mut h, &first), vec![second_grant, grant]);
    h.verify();
}

/// c28. Design D36: every accepted fresh proof replaces the relied list, and a retry's fresh
/// proof is offered the list the last accepted proof left.
#[test]
fn each_accepted_fresh_proof_replaces_the_relied_list() {
    let mut h = Harness::new("c28d", isabelle());
    h.setup();
    let first = h.pending(1, "call-1");
    let (_, other, _) = h.approved(1, "call-2", "until_revoked");
    let (grant, fresh) = h.approve(&first, "until_revoked");

    let order = h.dispatch(&fresh, &[&grant, &other]);
    let _ = h.attempt(
        &order,
        AttemptReport::Transient {
            retry_after_ms: None,
        },
    );
    h.advance(2_000);
    let output = h.tick();
    let retry = job_for(&output, &first, ProofPurpose::FreshPermit);
    assert_eq!(relied(&retry), vec![grant.clone(), other.clone()]);

    let _ = h.dispatch(&retry, &[&other]);
    h.kill();
    assert_eq!(relied_in_store(&mut h, &first), vec![other]);
    h.verify();
}

/// c28. Design D36 and 11.3 W1: a fresh result that is not accepted (the fence changed, or an
/// authorizing grant is no longer valid at acceptance) leaves the relied list unchanged.
#[test]
fn a_discarded_fresh_result_leaves_the_relied_list_unchanged() {
    let mut h = Harness::new("c28e", isabelle());
    h.setup();
    let first = h.pending(1, "call-1");
    let (_, other, _) = h.approved(1, "call-2", "until_revoked");
    let (grant, fresh) = h.approve(&first, "until_revoked");

    // The fence changed.
    h.change_fence();
    let output = h.prove(&fresh, Outcome::Permit, &[&other]);
    assert!(dispatches(&output).is_empty());
    let again = job_for(&output, &first, ProofPurpose::FreshPermit);
    assert_eq!(relied(&again), vec![grant.clone()]);

    // An authorizing grant ended while the proof ran.
    let _ = h.revoke(&other);
    let output = h.prove(&again, Outcome::Permit, &[&other]);
    assert!(dispatches(&output).is_empty());
    let third = job_for(&output, &first, ProofPurpose::FreshPermit);
    assert_eq!(relied(&third), vec![grant.clone()]);

    h.kill();
    assert_eq!(relied_in_store(&mut h, &first), vec![grant]);
    h.verify();
}

/// c28. Design 5.7 (relied grants) and 10.3 `TrustedFacts.grants`: a relied grant that ended is
/// still offered to the fresh proof, marked ended, so the theory can block on it.
#[test]
fn a_relied_grant_that_ended_is_offered_to_the_proof_as_ended() {
    let mut h = Harness::new("c28f", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    assert!(offered_grants(&fresh).contains(&(grant.clone(), false)));
    let _ = h.revoke(&grant);
    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    let again = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(relied(&again), vec![grant.clone()]);
    assert!(offered_grants(&again).contains(&(grant, true)));
    h.verify();
}

/// c31. Design D16 and 5.7 (re-proof counters): each purpose has its own counter of discarded
/// results, a retry never resets it, and once it is at `MAX_REPROOFS` the next needed re-proof
/// ends the request (`undecided` for a classification, `stale` for a fresh permit).
#[test]
fn the_reproof_counter_is_per_purpose_and_survives_a_retry() {
    let mut h = Harness::new("c31", isabelle());
    h.setup();

    // A classification discarded twice, then asked.
    let (request_id, output) = h.propose(1, "call-1");
    let mut job = job_for(&output, &request_id, ProofPurpose::Classification);
    for _ in 0..2 {
        h.change_fence();
        let output = h.prove(&job, Outcome::NeedApproval, &[]);
        job = job_for(&output, &request_id, ProofPurpose::Classification);
    }
    let _ = h.prove(&job, Outcome::NeedApproval, &[]);
    assert_eq!(h.state(&request_id), "pending_approval");

    // The fresh-permit counter starts on its own: one discard, then a dispatch.
    let (grant, fresh) = h.approve(&request_id, "until_revoked");
    h.change_fence();
    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    let fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    let order = h.dispatch(&fresh, &[&grant]);

    // A retry keeps the count: two more discards bring it to three...
    let _ = h.attempt(
        &order,
        AttemptReport::Transient {
            retry_after_ms: None,
        },
    );
    h.advance(2_000);
    let output = h.tick();
    let mut fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    for _ in 0..2 {
        h.change_fence();
        let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
        fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    }
    // ...and the fourth ends the request.
    h.change_fence();
    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(
        output.jobs.is_empty(),
        "a fourth fresh re-proof was started"
    );
    assert_eq!(h.state(&request_id), "stale");

    // A classification discarded four times ends undecided after three re-proofs.
    let (second, output) = h.propose(1, "call-2");
    let mut job = job_for(&output, &second, ProofPurpose::Classification);
    for _ in 0..3 {
        h.change_fence();
        let output = h.prove(&job, Outcome::NeedApproval, &[]);
        job = job_for(&output, &second, ProofPurpose::Classification);
    }
    h.change_fence();
    let output = h.prove(&job, Outcome::NeedApproval, &[]);
    assert!(
        output.jobs.is_empty(),
        "a fourth classification re-proof was started"
    );
    assert_eq!(h.state(&second), "undecided");
    assert_undecided(&mut h, &second, "reproofs_spent", json!("failed"));

    h.kill();
    let reproofs = h
        .read(|r| r.request(&id(&request_id)))
        .expect("the request")
        .reproofs;
    assert_eq!(reproofs.classification, 2);
    h.verify();
}

/// c33. PW 2.4 and README section 7: a result with no report goes `undecided` with the cause its
/// prover stated, and its details read that cause's value; a result that states none is stored as
/// `not_stated` and reads null.
#[test]
fn each_stated_no_report_cause_reads_its_value() {
    let mut h = Harness::new("c6", isabelle());
    h.setup();
    let rows = [
        (
            Some(ProverCause::Starting),
            "prover_starting",
            json!("prover_unavailable"),
        ),
        (
            Some(ProverCause::NotRunning),
            "prover_not_running",
            json!("prover_unavailable"),
        ),
        (
            Some(ProverCause::DeadlinePassed),
            "deadline_passed",
            json!("out_of_time"),
        ),
        (
            Some(ProverCause::CheckLimitReached),
            "check_limit_reached",
            json!("out_of_time"),
        ),
        (
            Some(ProverCause::JobRefused),
            "job_refused",
            json!("failed"),
        ),
        (
            Some(ProverCause::CheckNotDecided),
            "check_not_decided",
            json!("failed"),
        ),
        (
            Some(ProverCause::ReportUnusable),
            "report_unusable",
            json!("failed"),
        ),
        (None, "not_stated", Value::Null),
    ];
    for (n, (cause, stored, expected)) in rows.into_iter().enumerate() {
        let (request_id, output) = h.propose(1, &format!("call-{n}"));
        let job = job_for(&output, &request_id, ProofPurpose::Classification);
        let output = h.prove_with(no_report_done(&job, cause));
        assert!(output.halt.is_none());
        assert!(output.jobs.is_empty());
        assert_undecided(&mut h, &request_id, stored, expected);
        assert_eq!(proof_ids(&mut h, &request_id), Vec::<Value>::new());
    }
    h.verify();
}

/// c33. PW 2.4 and W04 5.7 (`no_prover`): a result with no report from a prover whose kind is not
/// Isabelle is stored as `no_prover` whatever cause it states, and reads `prover_unavailable`.
#[test]
fn a_synthetic_result_with_no_report_stores_no_prover() {
    let mut h = Harness::new("c33s", synthetic());
    h.setup();
    for (n, cause) in [Some(ProverCause::NotRunning), None]
        .into_iter()
        .enumerate()
    {
        let (request_id, output) = h.propose(1, &format!("call-{n}"));
        let job = job_for(&output, &request_id, ProofPurpose::Classification);
        let output = h.prove_with(no_report_done(&job, cause));
        assert!(output.halt.is_none());
        assert!(output.jobs.is_empty());
        assert_undecided(
            &mut h,
            &request_id,
            "no_prover",
            json!("prover_unavailable"),
        );
    }
    h.verify();
}

/// c33. PW 2.4: an Isabelle report whose verdict is undecided goes `undecided` with the cause its
/// prover stated, and with `check_not_decided` when it states none.
#[test]
fn an_undecided_verdict_reads_its_stated_cause() {
    let mut h = Harness::new("c7", isabelle());
    h.setup();
    let rows = [
        (
            Some(ProverCause::DeadlinePassed),
            "deadline_passed",
            "out_of_time",
        ),
        (
            Some(ProverCause::CheckLimitReached),
            "check_limit_reached",
            "out_of_time",
        ),
        (None, "check_not_decided", "failed"),
    ];
    for (n, (cause, stored, expected)) in rows.into_iter().enumerate() {
        let (request_id, output) = h.propose(1, &format!("call-{n}"));
        let job = job_for(&output, &request_id, ProofPurpose::Classification);
        let done = undecided_done(&job, cause);
        let proof = proof_id(&done);
        let output = h.prove_with(done);
        assert!(output.halt.is_none());
        assert_undecided(&mut h, &request_id, stored, json!(expected));
        assert!(proof_ids(&mut h, &request_id).contains(&json!(proof)));
    }
    h.verify();
}

/// c36. 5.7: a stored cause survives a restart, and a job proved again after it ends
/// with the cause the starting prover states; the restarted run reads starting with no problem.
#[test]
fn a_restart_keeps_the_cause_and_restates_starting() {
    let mut h = Harness::new("c14", isabelle_starting());
    h.setup();
    let (ended, output) = h.propose(1, "call-a");
    let job = job_for(&output, &ended, ProofPurpose::Classification);
    h.now = job.deadline_ms + 1;
    let _ = h.tick();
    assert_undecided(&mut h, &ended, "deadline_passed", json!("out_of_time"));

    let (proving, output) = h.propose(1, "call-b");
    let _ = job_for(&output, &proving, ProofPurpose::Classification);
    h.kill();
    h.advance(1_000);
    let output = h.start();
    assert!(output.halt.is_none());
    assert_eq!(
        h.snapshot()["health"],
        json!({"prover": "starting", "prover_problem": null, "store": "ready",
            "agent_channel": "not_listening", "agent_channel_problem": null})
    );
    assert_undecided(&mut h, &ended, "deadline_passed", json!("out_of_time"));
    let again = job_for(&output, &proving, ProofPurpose::Classification);
    let output = h.prove_with(no_report_done(&again, Some(ProverCause::Starting)));
    assert!(output.halt.is_none());
    assert_undecided(
        &mut h,
        &proving,
        "prover_starting",
        json!("prover_unavailable"),
    );
    h.verify();
}

/// c35. README section 7: `undecided_reason` is null unless the request is `undecided`, even when
/// a row correlated to it names a cause. Only a forged row can show this: the store refuses a
/// cause on any other move. The forged row breaks the chain, so the test ends without `verify`.
#[test]
fn the_state_check_in_read_details_matters() {
    let mut h = Harness::new("c15", isabelle());
    h.setup();
    let (blocked, output) = h.propose(1, "call-1");
    let job = job_for(&output, &blocked, ProofPurpose::Classification);
    let _ = h.prove(&job, Outcome::Block, &[]);
    assert_eq!(h.state(&blocked), "blocked");
    let (undecided, output) = h.propose(1, "call-2");
    let job = job_for(&output, &undecided, ProofPurpose::Classification);
    let _ = h.prove_with(undecided_done(&job, Some(ProverCause::DeadlinePassed)));
    assert_undecided(&mut h, &undecided, "deadline_passed", json!("out_of_time"));

    h.forge_ledger_row(&forged_move(&blocked, Some("check_not_decided")), &blocked);
    assert_eq!(
        h.stored_cause(&blocked).as_deref(),
        Some("check_not_decided")
    );
    let details = h.details(&blocked);
    assert_eq!(details["request"]["status"]["state"], json!("blocked"));
    assert_eq!(details["undecided_reason"], Value::Null);
    assert_eq!(reason(&mut h, &undecided), json!("out_of_time"));
}

/// c35. README section 7 and PR 28's choice 4: a request whose newest move to `undecided` has no cause,
/// as builds before the cause wrote it, reads null. The store no longer writes such a move, so
/// the row is forged; the forged row breaks the chain, so the test ends without `verify`.
#[test]
fn an_old_move_without_a_cause_reads_null() {
    let mut h = Harness::new("old-move", isabelle());
    h.setup();
    let (request_id, output) = h.propose(1, "call-1");
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let _ = h.prove_with(undecided_done(&job, None));
    assert_undecided(&mut h, &request_id, "check_not_decided", json!("failed"));

    h.forge_ledger_row(&forged_move(&request_id, None), &request_id);
    assert_eq!(h.stored_cause(&request_id), None);
    assert_eq!(h.state(&request_id), "undecided");
    assert_eq!(reason(&mut h, &request_id), Value::Null);
    // An unknown text, as a newer build might write, reads null as well.
    h.forge_ledger_row(
        &forged_move(&request_id, Some("a_cause_of_a_newer_build")),
        &request_id,
    );
    assert_eq!(reason(&mut h, &request_id), Value::Null);
}

/// c33. README section 7: a proof that decided but could not be acted on stores its cause, and the
/// details read null, since the contract has no value for it. Here a NeedApproval for the list
/// tool, which names no calendar and so has no scope a card can show.
#[test]
fn an_approval_with_no_scope_stores_its_cause_and_reads_null() {
    let mut h = Harness::new("no-scope", isabelle());
    h.setup();
    let (reply, output) = h.call(1, "call-1", "fabric_calendar_list", json!({}));
    let request_id = request_id_of(&reply);
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let output = h.prove(&job, Outcome::NeedApproval, &[]);
    assert!(output.halt.is_none());
    assert_undecided(&mut h, &request_id, "no_scope", Value::Null);
    h.verify();
}
