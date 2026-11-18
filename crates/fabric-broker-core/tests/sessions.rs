//! Sessions, agent activity and the agent entry (design 5.4, 5.5, D25, D35; section 9.3 c06, c22,
//! c30). Gate G2a rows "Expire grants" and "Unregistered agent (store half)" (design 7.3).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::SESSION_IDLE_MS;
use fabric_broker_core::ports::ConnectionId;
use fabric_desktop_contracts::owner::GrantEndReason;
use fabric_storage::SessionEndReason;
use serde_json::{Value, json};

const IDLE: u64 = SESSION_IDLE_MS as u64;

fn session_of(h: &mut Harness, agent_id: &str) -> Value {
    h.agent_view(agent_id)["session"].clone()
}

/// c06. Design 5.5: a session idle for 1 800 000 ms after its last call ends with
/// `idle_timeout`, its session grants end `session_idle_timeout`, and the agent revision rises.
/// It is live until 1 ms before the limit and ends at the limit itself, as the theory's
/// `session_live` reads it (c39).
#[test]
fn an_idle_session_ends_at_1800_s_with_session_idle_timeout() {
    let mut h = Harness::new("c06a", isabelle());
    h.setup();
    let (_, grant, _) = h.approved(1, "call-1", "session");
    let last_call = T0;
    let session_id = session_of(&mut h, AGENT)["session_id"]
        .as_str()
        .expect("a live session")
        .to_owned();
    let revision = h.agent_view(AGENT)["revision"]
        .as_u64()
        .expect("a revision");

    h.now = last_call + IDLE - 1;
    let output = h.tick();
    assert!(output.halt.is_none());
    assert_ne!(
        session_of(&mut h, AGENT),
        Value::Null,
        "the session ended early"
    );
    assert!(h.live_grants().contains(&grant));

    h.now = last_call + IDLE;
    let output = h.tick();
    assert!(output.halt.is_none());
    assert_eq!(
        session_of(&mut h, AGENT),
        Value::Null,
        "the idle session lives on"
    );
    assert!(!h.live_grants().contains(&grant));
    assert_eq!(h.agent_view(AGENT)["revision"], json!(revision + 1));

    h.kill();
    let session = h
        .read(|r| r.session(&id(&session_id)))
        .expect("the session row stays");
    assert_eq!(
        session.ended.map(|e| e.1),
        Some(SessionEndReason::IdleTimeout)
    );
    let ended = h
        .read(|r| r.grant(&id(&grant)))
        .expect("the grant stays")
        .ended
        .map(|e| e.reason);
    assert_eq!(ended, Some(GrantEndReason::SessionIdleTimeout));
    h.verify();
}

/// c06. Design 5.5, D35 and R20: a status poll is activity, so it moves the idle deadline.
#[test]
fn a_status_poll_keeps_a_session_alive() {
    let mut h = Harness::new("c06b", no_prover());
    h.setup();
    let (request_id, _) = h.propose(1, "call-1");
    h.advance(1_000_000);
    let (reply, _) = h.poll(1, "call-2", &request_id);
    assert_eq!(reply["result"]["request"]["request_id"], json!(request_id));
    let polled = h.now;

    h.now = T0 + IDLE + 1;
    let _ = h.tick();
    let session = session_of(&mut h, AGENT);
    assert_ne!(session, Value::Null, "a polling agent lost its session");
    assert_eq!(session["last_active_at"], timestamp(polled));

    h.now = polled + IDLE;
    let _ = h.tick();
    assert_eq!(session_of(&mut h, AGENT), Value::Null);
    h.verify();
}

/// c06. Design 5.5 and D35: a session's `last_active_at` starts at its connect, so a session
/// with no call has a value and idles 1 800 000 ms after its connect; `last_tool_call_at` stays
/// null until the agent's first call.
#[test]
fn a_session_with_no_call_has_last_active_at_its_start_and_idles_1800_s_after_connect() {
    let mut h = Harness::new("c06c", no_prover());
    h.setup();
    let agent = h.agent_view(AGENT);
    assert_eq!(agent["session"]["started_at"], timestamp(T0));
    assert_eq!(agent["session"]["last_active_at"], timestamp(T0));
    assert_eq!(agent["last_tool_call_at"], Value::Null);

    h.now = T0 + IDLE - 1;
    let _ = h.tick();
    assert_ne!(session_of(&mut h, AGENT), Value::Null);

    h.now = T0 + IDLE;
    let _ = h.tick();
    assert_eq!(session_of(&mut h, AGENT), Value::Null);
    h.verify();
}

/// c22. Design 5.5 and section 4 (access audit): an agent that is not enrolled gets its
/// connection closed and an audit row; no session, no request and no ledger row exist for it.
#[test]
fn an_unknown_agent_gets_no_session_and_no_request() {
    let mut h = Harness::new("c22", isabelle());
    h.setup();
    let head = h.ledger_head();
    let audited = h.audit().len();

    let output = h.connect(9, "agent-unknown");
    assert!(output.halt.is_none());
    assert!(output.close.contains(&ConnectionId(9)));
    assert!(
        h.audit().len() > audited,
        "the connection attempt was not audited"
    );

    let output = h.send_call(
        9,
        "call-x",
        CREATE,
        create_params(ACCOUNT, CALENDAR, "intruder"),
    );
    assert!(output.halt.is_none());
    assert!(output.jobs.is_empty());
    assert!(
        agent_replies(&output, 9)
            .iter()
            .all(|r| r.get("error").is_some()),
        "an unknown agent got a result"
    );
    assert_eq!(
        h.ledger_head(),
        head,
        "an unknown agent wrote to the ledger"
    );
    let snapshot = h.snapshot();
    assert_eq!(snapshot["requests"], json!([]));
    assert_eq!(snapshot["agents"].as_array().expect("agents").len(), 1);

    h.kill();
    assert_eq!(h.read(|r| r.agent(&id("agent-unknown"))), None);
    let sessions = h.read(|r| r.live_sessions());
    assert!(
        sessions.iter().all(|s| s.agent_id.as_str() == AGENT),
        "an unknown agent got a session"
    );
    h.verify();
}

/// c30. Design D25 and section 4 (idempotency): a repeated call returns its request and writes
/// nothing, a status poll writes nothing, and neither halts; the same call id with another
/// action is `invalid_params`.
#[test]
fn repeated_calls_and_status_polls_write_no_ledger_row() {
    let mut h = Harness::new("c30a", isabelle());
    h.setup();
    let (request_id, _) = h.propose(1, "call-1");
    let head = h.ledger_head();

    let (reply, output) = h.call(
        1,
        "call-1",
        CREATE,
        create_params(ACCOUNT, CALENDAR, "call-1"),
    );
    assert!(output.halt.is_none());
    assert_eq!(request_id_of(&reply), request_id);
    let (reply, output) = h.poll(1, "call-2", &request_id);
    assert!(output.halt.is_none());
    assert_eq!(request_id_of(&reply), request_id);
    assert_eq!(
        h.ledger_head(),
        head,
        "a repeat or a poll wrote a ledger row"
    );

    let (reply, output) = h.call(
        1,
        "call-1",
        CREATE,
        create_params(ACCOUNT, CALENDAR, "changed"),
    );
    assert!(output.halt.is_none());
    assert_eq!(error(&reply)["code"], json!("invalid_params"));
    assert_eq!(h.ledger_head(), head);
    h.verify();
}

/// c30. Design 5.4 and D35: activity is kept in memory: calls and polls change the agent's
/// `last_tool_call_at` and its session's `last_active_at` but never its revision and send no
/// agent notice, so a pause at the revision the page holds succeeds.
#[test]
fn activity_never_raises_the_agent_revision() {
    let mut h = Harness::new("c30b", isabelle());
    h.setup();
    let _ = h.subscribe();
    let revision = h.agent_view(AGENT)["revision"].clone();

    let (request_id, output) = h.propose(1, "call-1");
    assert!(notices_of(&output, "agent").is_empty());
    h.advance(1_000);
    let (_, output) = h.poll(1, "call-2", &request_id);
    assert!(notices_of(&output, "agent").is_empty());

    let agent = h.agent_view(AGENT);
    assert_eq!(agent["revision"], revision);
    assert_eq!(agent["last_tool_call_at"], timestamp(h.now));
    assert_eq!(agent["session"]["last_active_at"], timestamp(h.now));

    let answer = h.ok(
        "pause_agent",
        json!({
            "target": {"kind": "agent", "agent_id": AGENT, "expected_revision": revision},
            "paused": true
        }),
    );
    assert_eq!(answer["agents"][0]["paused"], json!(true));
    h.verify();
}

/// c30. Design 5.4 and 5.5: a connect that replaces a live session is one commit: the old
/// session ends `Replaced` and its session grants `session_disconnected`, and the agent revision
/// rises by exactly one with one agent notice.
#[test]
fn a_connect_that_replaces_a_live_session_raises_the_agent_revision_once() {
    let mut h = Harness::new("c30c", isabelle());
    h.setup();
    let (_, grant, _) = h.approved(1, "call-1", "session");
    let _ = h.subscribe();
    let agent = h.agent_view(AGENT);
    let revision = agent["revision"].as_u64().expect("a revision");
    let old_session = agent["session"]["session_id"]
        .as_str()
        .expect("a live session")
        .to_owned();
    let head = h.ledger_head();

    let output = h.connect(2, AGENT);
    assert!(output.halt.is_none());
    let agent_notices = notices_of(&output, "agent");
    assert_eq!(agent_notices.len(), 1, "expected one agent notice");
    assert_eq!(
        agent_notices[0]["change"]["agent"]["revision"],
        json!(revision + 1)
    );
    let grant_notices = notices_of(&output, "grant");
    assert!(grant_notices.iter().any(|n| {
        n["change"]["grant"]["grant_id"] == json!(grant)
            && n["change"]["grant"]["ended"]["reason"] == json!("session_disconnected")
    }));
    assert_eq!(
        h.ledger_head(),
        head + 1,
        "the connect took more than one commit"
    );

    let agent = h.agent_view(AGENT);
    assert_eq!(agent["revision"], json!(revision + 1));
    assert_ne!(agent["session"]["session_id"], json!(old_session));

    h.kill();
    let session = h
        .read(|r| r.session(&id(&old_session)))
        .expect("the old session row stays");
    assert_eq!(session.ended.map(|e| e.1), Some(SessionEndReason::Replaced));
    let ended = h
        .read(|r| r.grant(&id(&grant)))
        .expect("the grant stays")
        .ended
        .map(|e| e.reason);
    assert_eq!(ended, Some(GrantEndReason::SessionDisconnected));
    h.verify();
}
