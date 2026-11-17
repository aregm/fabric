//! Audit before effect, halts, kills and the unavailable mode (design 3.4, 5.2, 5.8, 7.1, 7.2,
//! D7, D9; section 9.3 c12-c15, c25, c27). Gate G2a rows "Fail audit persistence or lock the
//! database", "Accepted insert, response lost, broker killed", "Expire grants; restart (also a
//! reopen)", DEF-4 and R12 (design 7.3).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::REOPEN_INTERVAL_MS;
use fabric_broker_core::ports::{AttemptReport, ConnectionId};
use fabric_desktop_contracts::RequestState;
use fabric_desktop_contracts::owner::{AttemptOutcome, GrantEndReason, ProofPurpose};
use fabric_storage::{AuthorizationState, SCHEMA_VERSION, Unavailable};
use serde_json::{Value, json};

fn hello() -> Value {
    json!({"client": {"name": "Fabric Calendar", "build": "test"}, "schema_version": 2})
}

fn store_unavailable(response: &Value) -> Value {
    let refusal = error(response);
    assert_eq!(refusal["code"], json!("store_unavailable"), "{response}");
    assert_eq!(refusal["detail"]["reason"], json!("locked"), "{response}");
    refusal
}

/// c12, for the two writes of a handoff. Design 5.2, 5.7 and 7.1: when the `request.authorize`
/// or the `request.dispatch` commit fails, the step halts, its output carries no job, and the
/// next start hands nothing off from it.
fn a_failed_handoff_write_hands_off_nothing(operation_type: &str) {
    let mut h = Harness::new("c12-handoff", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    h.inject_fault(operation_type);

    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(
        output.halt.is_some(),
        "{operation_type}: a failed write did not halt"
    );
    assert!(
        output.jobs.is_empty(),
        "{operation_type}: a halted step handed out a job"
    );

    h.kill();
    h.clear_faults();
    h.advance(1_000);
    let output = h.start();
    assert!(
        dispatches(&output).is_empty(),
        "{operation_type}: a restart handed off"
    );
    assert_eq!(h.state(&request_id), "proving_fresh_permit");

    h.kill();
    let authorization = h.read(|r| r.latest_authorization_of(&id(&request_id)));
    match operation_type {
        "request.authorize" => assert_eq!(authorization, None),
        _ => assert_eq!(
            authorization.map(|a| a.state),
            Some(AuthorizationState::Voided)
        ),
    }
    assert_eq!(h.attempt_rows(), 0);
    h.verify();
}

/// c12. Design 5.2, 5.3, 7.1 and 7.2: a failed write of a handoff, an agent call, a revoke or a
/// decision halts the step, answers the frame in hand with `internal` (agent) or
/// `store_unavailable` (owner), hands out no job and leaves no trace in the store.
#[test]
fn a_failed_authorize_or_dispatch_write_hands_off_nothing_and_halts() {
    a_failed_handoff_write_hands_off_nothing("request.authorize");
    a_failed_handoff_write_hands_off_nothing("request.dispatch");

    // agent.call
    let mut h = Harness::new("c12-call", isabelle());
    h.setup();
    h.inject_fault("agent.call");
    let (reply, output) = h.call(
        1,
        "call-1",
        CREATE,
        create_params(ACCOUNT, CALENDAR, "call-1"),
    );
    assert_eq!(error(&reply)["code"], json!("internal"));
    assert!(output.halt.is_some());
    assert!(output.jobs.is_empty());
    h.kill();
    h.clear_faults();
    assert_eq!(
        h.read(|r| r.request_by_call(&id(AGENT), &id("call-1"))),
        None
    );
    h.verify();

    // owner.revoke_grant
    let mut h = Harness::new("c12-revoke", isabelle());
    h.setup();
    let (_, grant, _) = h.approved(1, "call-1", "until_revoked");
    h.inject_fault("owner.revoke_grant");
    let (response, output) = h.owner(
        "revoke_grant",
        json!({"grant_id": grant, "expected_revision": 1}),
    );
    let _ = store_unavailable(&response);
    assert!(output.halt.is_some());
    assert!(output.jobs.is_empty());
    h.kill();
    h.clear_faults();
    let ended = h.read(|r| r.grant(&id(&grant))).expect("the grant").ended;
    assert_eq!(ended, None, "a failed revoke ended the grant");
    h.verify();

    // owner.decide_request
    let mut h = Harness::new("c12-decide", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    h.inject_fault("owner.decide_request");
    let (response, output) = h.decide(&request_id, "approve", json!({"kind": "one_request"}));
    let _ = store_unavailable(&response);
    assert!(output.halt.is_some());
    assert!(output.jobs.is_empty());
    h.kill();
    h.clear_faults();
    let request = h
        .read(|r| r.request(&id(&request_id)))
        .expect("the request");
    assert_eq!(
        request.row.request.status.state(),
        RequestState::PendingApproval
    );
    assert_eq!(request.approval, None);
    assert!(h.read(|r| r.grants_of_agent(&id(AGENT))).is_empty());
    h.verify();
}

/// c13. Design 5.2 and 7.2: a database held by another writer makes the broker's write fail
/// after its busy timeout; the command is answered `store_unavailable` and the step halts.
#[test]
fn a_busy_store_answers_store_unavailable_and_halts() {
    let mut h = Harness::new("c13", isabelle());
    h.setup();
    let holder = h.raw();
    holder
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("hold the write lock");

    let (response, output) = h.owner(
        "save_group",
        json!({
            "expected_revision": 0,
            "change": {
                "kind": "save",
                "recipient_set_id": null,
                "definition": {"name": "Team", "members": [], "domains": []}
            }
        }),
    );
    let _ = store_unavailable(&response);
    assert!(output.halt.is_some());
    assert!(output.jobs.is_empty());

    holder.execute_batch("ROLLBACK;").expect("release the lock");
    drop(holder);
    h.kill();
    assert!(h.read(|r| r.recipient_sets()).is_empty());
    h.verify();
}

/// c14. Design 5.2 and D7: a halted step's jobs are dropped even when one of its commits
/// succeeded, and a dispatch committed before the halt becomes `outcome_unknown` at the next
/// start, its open attempt finished `outcome_unknown`.
#[test]
fn a_halted_output_carries_no_job_and_a_committed_dispatch_restarts_as_outcome_unknown() {
    let mut h = Harness::new("c14", isabelle());
    h.setup();
    let (handed_off, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let order = h.dispatch(&fresh, &[&grant]);
    let waiting = h.pending(1, "call-2");

    // The decision commits; the move it makes due, which would start a proof, fails.
    h.inject_when(
        "after_decision",
        &format!(
            "NEW.correlation_id = '{waiting}' AND NEW.operation_type <> 'owner.decide_request'"
        ),
    );
    let (_, output) = h.decide(&waiting, "approve", json!({"kind": "one_request"}));
    assert!(
        output.halt.is_some(),
        "the failed follow-up write did not halt"
    );
    assert!(output.jobs.is_empty(), "a halted output carried a job");

    h.kill();
    h.clear_faults();
    h.advance(1_000);
    let output = h.start();
    assert!(dispatches(&output).is_empty());
    assert_eq!(h.state(&handed_off), "outcome_unknown");

    h.kill();
    let effect = h
        .read(|r| r.effect_of(&id(&handed_off)))
        .expect("the effect stays");
    assert_eq!(effect.effect_id, order.effect_id);
    let attempts = h.read(|r| r.attempts(&effect.effect_id));
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].attempt.outcome,
        Some(AttemptOutcome::OutcomeUnknown)
    );
    assert!(attempts[0].attempt.finished_at.is_some());
    h.verify();
}

/// c14. Design 5.2 and D7, deterministically: in one step a due retry commits its move and
/// queues its fresh proof, then the owner write of the same step fails. The halted output
/// carries no job, although the commit that queued it succeeded.
#[test]
fn a_halted_step_drops_the_job_of_an_earlier_commit_in_the_same_step() {
    let mut h = Harness::new("c14b", isabelle());
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

    h.inject_when("owner_writes", "NEW.operation_type LIKE 'owner.%'");
    h.advance(2_000);
    let head = h.ledger_head();
    let (_, output) = h.owner(
        "save_group",
        json!({
            "expected_revision": 0,
            "change": {"kind": "save", "recipient_set_id": null,
                       "definition": {"name": "Team", "members": [], "domains": []}}
        }),
    );
    assert!(output.halt.is_some(), "the failed owner write did not halt");
    assert!(output.jobs.is_empty(), "a halted output carried a job");
    let queued = h.ledger().into_iter().any(|row| {
        row.seq > head
            && row.operation_type == "request.advance"
            && row.correlation_id.as_deref() == Some(request_id.as_str())
    });
    assert!(queued, "the retry's move did not commit before the fault");

    h.kill();
    h.clear_faults();
    h.verify();
}

/// c15. Design 5.8 and 3.5 (`effects`): a kill after the dispatch intent leaves the request
/// `outcome_unknown` at the next start, with the provider event id reserved before the first
/// call, and nothing is handed off again.
#[test]
fn a_kill_after_dispatch_intent_restarts_as_outcome_unknown_with_the_same_provider_event_id() {
    let mut h = Harness::new("c15", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "until_revoked");
    let order = h.dispatch(&fresh, &[&grant]);
    let reserved = order
        .provider_event_id
        .clone()
        .expect("a create carries its provider event id");

    h.kill();
    h.advance(1_000);
    let output = h.start();
    assert!(
        dispatches(&output).is_empty(),
        "a restart handed the change off again"
    );
    assert_eq!(h.state(&request_id), "outcome_unknown");

    h.kill();
    let effect = h
        .read(|r| r.effect_of(&id(&request_id)))
        .expect("the effect stays");
    assert_eq!(effect.provider_event_id, Some(reserved.clone()));
    let open = h.read(|r| r.open_effects());
    assert!(open.iter().any(|e| e.event_id == reserved));
    let attempts = h.read(|r| r.attempts(&effect.effect_id));
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].attempt.outcome,
        Some(AttemptOutcome::OutcomeUnknown)
    );
    h.verify();
}

/// c25. Design 3.4 and D8: a store that cannot be used keeps the broker up; `hello` answers, and
/// every other method answers `store_unavailable` with the reason's message; an agent
/// connection is closed and an agent call answered `internal` (the code's rule; 3.4 names only
/// the owner methods); a final reason is not retried.
#[test]
fn an_unavailable_store_answers_hello_and_refuses_everything_else() {
    let mut h = Harness::new("c25", no_prover());
    let _ = h.quit();
    h.raw()
        .execute_batch("PRAGMA user_version = 2;")
        .expect("mark the store as written by a newer build");

    h.advance(1_000);
    let output = h.start();
    assert!(output.halt.is_none());
    assert!(!h.broker().store_ready());

    let answer = h.ok("hello", hello());
    assert_eq!(answer["schema_version"], json!(2));

    let message = Unavailable::NewerSchema {
        found: 2,
        supported: SCHEMA_VERSION,
    }
    .message();
    let requests = [
        ("get_snapshot", json!({})),
        ("subscribe", json!({"cursor": 0})),
        (
            "save_profile",
            json!({"expected_revision": 0, "change": {"kind": "activate", "profile_id": "careful"}}),
        ),
        (
            "save_preferences",
            json!({"expected_revision": 0, "settings": {"zone": null, "workday": null,
                "protected_times": [], "uninterrupted": null, "buffer": null, "warning_calendars": []}}),
        ),
        (
            "save_group",
            json!({"expected_revision": 0, "change": {"kind": "save", "recipient_set_id": null,
                "definition": {"name": "Team", "members": [], "domains": []}}}),
        ),
        (
            "decide_request",
            json!({"request_id": "request-1", "request_digest": format!("sha256:{}", "d".repeat(64)),
                "expected_revision": 1, "decision": "deny", "selected": null, "duration": null}),
        ),
        (
            "pause_agent",
            json!({"target": {"kind": "all_agents"}, "paused": true}),
        ),
        (
            "revoke_grant",
            json!({"grant_id": "grant-1", "expected_revision": 1}),
        ),
        (
            "get_request_details",
            json!({"request_id": "request-1", "redaction": "redacted"}),
        ),
        ("get_proof", json!({"proof_id": "proof-1"})),
        (
            "disconnect_google",
            json!({"account_id": "acct-1", "expected_revision": 1, "revoke_provider_token": false}),
        ),
    ];
    for (method, params) in &requests {
        let (response, output) = h.owner(method, params.clone());
        assert!(output.halt.is_none(), "{method} halted");
        let refusal = store_unavailable(&response);
        assert_eq!(refusal["message"], json!(message), "{method}");
    }

    // The agent channel: a connection is closed at once, and a call on one is answered
    // `internal`; nothing is stored.
    let head = h.ledger_head();
    let output = h.connect(1, AGENT);
    assert!(output.halt.is_none());
    assert_eq!(output.close, vec![ConnectionId(1)]);
    assert!(output.agent.is_empty());
    assert!(output.owner.is_empty());
    let (reply, output) = h.call(
        1,
        "call-1",
        CREATE,
        create_params(ACCOUNT, CALENDAR, "call-1"),
    );
    assert!(output.halt.is_none());
    assert_eq!(output.agent.len(), 1);
    assert!(output.jobs.is_empty());
    assert_eq!(error(&reply)["code"], json!("internal"), "{reply}");
    assert_eq!(
        h.ledger_head(),
        head,
        "the unavailable mode stored something"
    );

    h.advance(REOPEN_INTERVAL_MS + 1_000);
    let (response, _) = h.owner("get_snapshot", json!({}));
    let _ = store_unavailable(&response);
    assert!(
        !h.broker().store_ready(),
        "a final reason was retried into use"
    );

    h.kill();
    h.raw()
        .execute_batch("PRAGMA user_version = 1;")
        .expect("restore the schema version");
    h.verify();
}

/// c27. Design D9, D30, 3.4 and 5.8: a store in use at start leaves the broker unavailable; once
/// the lock is free, an owner frame at least 5 s after the last attempt reopens it through the
/// full recovery (the dead run's session grant ends `broker_restart`, its issued authorization
/// is voided, `queued -> proving_fresh_permit`), and only then is the frame answered.
#[test]
fn a_reopen_after_in_use_runs_recovery() {
    let mut h = Harness::new("c27", isabelle());
    h.setup();
    let (request_id, grant, fresh) = h.approved(1, "call-1", "session");
    // Leave a queued request with an issued authorization: the dispatch commit fails.
    h.inject_fault("request.dispatch");
    let output = h.prove(&fresh, Outcome::Permit, &[&grant]);
    assert!(output.halt.is_some());
    h.kill();
    h.clear_faults();

    let held = h.open_store();
    h.advance(1_000);
    let started = h.now;
    let output = h.start();
    assert!(output.jobs.is_empty());
    assert!(!h.broker().store_ready());
    let _ = h.ok("hello", hello());
    let (response, _) = h.owner("get_snapshot", json!({}));
    let _ = store_unavailable(&response);

    held.close().expect("release the store");
    h.now = started + 1_000;
    let (response, _) = h.owner("get_snapshot", json!({}));
    let _ = store_unavailable(&response);
    assert!(!h.broker().store_ready(), "the store reopened within 5 s");

    h.now = started + REOPEN_INTERVAL_MS + 1;
    let (response, output) = h.owner("get_snapshot", json!({}));
    let snapshot = result(&response);
    assert!(h.broker().store_ready());
    let live: Vec<Value> = snapshot["grants"].as_array().expect("grants").clone();
    assert!(
        live.iter().all(|g| g["grant_id"] != json!(grant)),
        "the frame was answered before the recovery ended the dead run's session grant"
    );
    assert!(dispatches(&output).is_empty());
    let _ = job_for(&output, &request_id, ProofPurpose::FreshPermit);

    h.kill();
    let ended = h
        .read(|r| r.grant(&id(&grant)))
        .expect("the grant")
        .ended
        .map(|e| e.reason);
    assert_eq!(ended, Some(GrantEndReason::BrokerRestart));
    let authorization = h
        .read(|r| r.latest_authorization_of(&id(&request_id)))
        .expect("the authorization stays");
    assert_eq!(authorization.state, AuthorizationState::Voided);
    assert_eq!(
        h.void_reason(authorization.authorization.authorization_id.as_str())
            .as_deref(),
        Some("broker_restart")
    );
    let request = h
        .read(|r| r.request(&id(&request_id)))
        .expect("the request");
    assert_eq!(
        request.row.request.status.state(),
        RequestState::ProvingFreshPermit
    );
    h.verify();
}

/// c27b. Design 3.4 and D9: a store file that cannot be read at start is a passing
/// I/O condition, not an integrity failure: the broker is unavailable with the `Io` message
/// (which never tells the owner to move the store aside), and once the file can be read an owner
/// frame at least 5 s after the last attempt reopens it, as for a store in use.
#[cfg(unix)]
#[test]
fn a_reopen_after_a_passing_io_failure_opens_the_store() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut h = Harness::new("c27b", no_prover());
    h.setup();
    let before = h.snapshot();
    let _ = h.quit();
    let database = h.data_dir.join(fabric_storage::DATABASE_FILE);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o000))
        .expect("make the store file unreadable");
    if std::fs::File::open(&database).is_ok() {
        // A superuser reads any file: the condition cannot be made here.
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))
            .expect("restore the store file");
        return;
    }

    h.advance(1_000);
    let started = h.now;
    let output = h.start();
    assert!(output.halt.is_none());
    assert!(!h.broker().store_ready());
    let (response, _) = h.owner("get_snapshot", json!({}));
    let refusal = store_unavailable(&response);
    assert_eq!(refusal["message"], json!(Unavailable::Io.message()));

    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))
        .expect("restore the store file");
    h.now = started + 1_000;
    let (response, _) = h.owner("get_snapshot", json!({}));
    let _ = store_unavailable(&response);
    assert!(!h.broker().store_ready(), "the store reopened within 5 s");

    h.now = started + REOPEN_INTERVAL_MS + 1;
    let (response, _) = h.owner("get_snapshot", json!({}));
    let snapshot = result(&response);
    assert!(h.broker().store_ready());
    let agent_ids = |snapshot: &Value| -> Vec<Value> {
        snapshot["agents"]
            .as_array()
            .expect("agents")
            .iter()
            .map(|a| a["agent_id"].clone())
            .collect()
    };
    assert_eq!(agent_ids(&snapshot), agent_ids(&before));
    assert!(!agent_ids(&before).is_empty());
    h.kill();
    h.verify();
}
