//! Revoke and pause against a proof, a handoff and a retry (design 5.1, 5.6, 5.7; section 9.3
//! c07-c10). Gate G2a row "Revoke/pause during proof, queued, before retry" and R08 (design 7.3).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::ports::AttemptReport;
use fabric_desktop_contracts::owner::ProofPurpose;
use serde_json::{Value, json};

/// c07. Design 5.1 and 5.6: a revoke is an ordinary input, answered at once while a proof runs;
/// the permit computed before it is discarded (fence changed), never handed off, and the
/// re-proof sees the relied grant as ended and blocks.
#[test]
fn revoke_while_a_proof_runs_is_acknowledged_first_and_the_permit_is_discarded() {
    let mut h = Harness::new("c07", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");

    let (response, output) = h.owner(
        "revoke_grant",
        json!({"grant_id": grant, "expected_revision": 1}),
    );
    assert_eq!(
        result(&response)["grant"]["ended"]["reason"],
        json!("revoked_by_owner"),
        "the revoke was not acknowledged while the proof ran"
    );
    assert!(dispatches(&output).is_empty());

    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(output.halt.is_none());
    assert!(
        dispatches(&output).is_empty(),
        "a discarded permit was handed off"
    );
    assert_eq!(h.state(&request_id), "proving_fresh_permit");
    let again = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(relied(&again), vec![grant.clone()]);
    assert!(offered_grants(&again).contains(&(grant.clone(), true)));

    let output = h.prove(&again, Outcome::Block, &[]);
    assert!(dispatches(&output).is_empty());
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("blocked"));
    assert_eq!(details["authorization"], Value::Null);
    h.verify();
}

/// c08. Design 5.6 and 5.7 (`queued -> proving_fresh_permit`): a pause committed before the
/// handoff refuses it: no `Job::Dispatch`, no consumed authorization, the request goes back to a
/// fresh proof whose facts say the agent is paused.
///
/// The setting of 9.3 c08 (the pause lands while the request sits in `queued`) has no public
/// entry: 5.1 runs the pump after every input, so an accepted permit is dispatched in the same
/// step. The pause therefore lands while the fresh proof runs; the permit then reaches the
/// broker with the pause already committed and is discarded on the fence, so no authorization
/// is ever issued. The `queued` setting and the dispatch check's void reasons (`expired`,
/// `fence_changed`, `grant_invalid`, `agent_paused`) are covered by the unit tests in
/// `service/tests.rs` (`..._while_queued_voids_the_authorization`, and c29's expiry).
#[test]
fn pause_while_queued_refuses_the_handoff() {
    let mut h = Harness::new("c08", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let answer = h.pause(AGENT, true);
    assert_eq!(answer["agents"][0]["paused"], json!(true));

    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(output.halt.is_none());
    assert!(
        dispatches(&output).is_empty(),
        "a paused agent's change was handed off"
    );
    assert_eq!(h.state(&request_id), "proving_fresh_permit");
    let again = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert!(again.facts.paused, "the new proof does not see the pause");

    h.kill();
    assert_eq!(
        h.read(|r| r.latest_authorization_of(&id(&request_id))),
        None,
        "a permit discarded on the fence issued an authorization"
    );
    assert!(
        !h.ledger()
            .iter()
            .any(|row| row.operation_type == "request.authorize"
                && row.correlation_id.as_deref() == Some(request_id.as_str())),
        "the ledger has an authorize row"
    );
    assert_eq!(h.attempt_rows(), 0);
    h.verify();
}

/// c09. Design 5.7 (`retry_wait -> proving_fresh_permit`) and 5.6: a revoke before a retry
/// refuses it: the retry starts a fresh proof whose facts carry the relied grant as ended, the
/// theory's Block ends the request `blocked`, and nothing is handed off.
#[test]
fn revoke_before_a_retry_refuses_the_retry() {
    let mut h = Harness::new("c09", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let order = h.dispatch(&fresh, &[&grant]);
    let _ = h.attempt(
        &order,
        AttemptReport::Transient {
            retry_after_ms: None,
        },
    );
    assert_eq!(h.state(&request_id), "retry_wait");

    let _ = h.revoke(&grant);
    h.advance(2_000);
    let output = h.tick();
    assert!(dispatches(&output).is_empty());
    assert_eq!(h.state(&request_id), "proving_fresh_permit");
    let job = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(relied(&job), vec![grant.clone()]);
    assert!(offered_grants(&job).contains(&(grant.clone(), true)));

    let output = h.prove(&job, Outcome::Block, &[]);
    assert!(dispatches(&output).is_empty());
    assert_eq!(h.state(&request_id), "blocked");
    assert_eq!(
        h.details(&request_id)["attempts"].as_array().map(Vec::len),
        Some(1)
    );
    h.verify();
}

/// c10. Design 6.1 `revoke_grant`: a revoke after the handoff is committed and answered, and the
/// call already handed off stays `dispatching` with its open attempt visible.
#[test]
fn a_revoke_after_handoff_leaves_the_attempt_visible() {
    let mut h = Harness::new("c10", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let order = h.dispatch(&fresh, &[&grant]);

    let answer = h.revoke(&grant);
    assert_eq!(
        answer["grant"]["ended"]["reason"],
        json!("revoked_by_owner")
    );

    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("dispatching"));
    let attempts = details["attempts"].as_array().expect("attempts").clone();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["number"], json!(order.attempt));
    assert_eq!(attempts[0]["finished_at"], Value::Null);
    assert_eq!(attempts[0]["outcome"], Value::Null);
    assert_eq!(
        details["authorization"]["authorization_id"],
        json!(order.authorization.authorization_id)
    );
    assert_ne!(details["authorization"]["consumed_at"], Value::Null);
    h.verify();
}
