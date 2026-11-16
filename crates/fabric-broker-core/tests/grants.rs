//! Grant terms, approvals and account namespacing (design 5.5, 5.6, 5.8, 6.1; section 9.3
//! c01-c04, c11, c21, c23). Gate G2a rows "Expire grants; restart" and "Approve, then alter"
//! (design 7.3).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::ports::ProofJob;
use fabric_desktop_contracts::owner::{Duration, GrantEndReason, GrantTerm, ProofPurpose, Scope};
use fabric_desktop_contracts::scalar::Digest;
use fabric_desktop_contracts::{Decision, approval_digest};
use fabric_storage::{Mutation, SessionEndReason};
use serde_json::{Value, json};

fn end_reason(h: &mut Harness, grant_id: &str) -> Option<GrantEndReason> {
    h.read(|r| r.grant(&id(grant_id)))
        .unwrap_or_else(|| panic!("no grant {grant_id}"))
        .ended
        .map(|e| e.reason)
}

fn session_of(h: &mut Harness, agent_id: &str) -> String {
    h.agent_view(agent_id)["session"]["session_id"]
        .as_str()
        .expect("the agent has a live session")
        .to_owned()
}

/// c01. Design 5.5 and 5.8: a session grant is bound to a session of the current run, so the
/// recovery of the next start ends it with `broker_restart`; an until-revoked grant is
/// untouched.
#[test]
fn restart_ends_session_grants_with_broker_restart() {
    let mut h = Harness::new("c01", isabelle());
    h.setup();
    let (_, session_grant, _) = h.approved(1, "call-1", "session");
    let (_, kept, _) = h.approved(1, "call-2", "until_revoked");
    let session_id = session_of(&mut h, AGENT);

    h.kill();
    // The dead run left the grant without an end; only the next run's recovery records one.
    assert_eq!(end_reason(&mut h, &session_grant), None);

    h.advance(1_000);
    let _ = h.start();
    let live = h.live_grants();
    assert!(
        !live.contains(&session_grant),
        "a session grant outlived its run"
    );
    assert!(
        live.contains(&kept),
        "an until-revoked grant ended at a restart"
    );
    assert_eq!(h.agent_view(AGENT)["session"], Value::Null);

    h.kill();
    assert_eq!(
        end_reason(&mut h, &session_grant),
        Some(GrantEndReason::BrokerRestart)
    );
    let session = h
        .read(|r| r.session(&id(&session_id)))
        .expect("the session row stays");
    assert_eq!(
        session.ended.map(|e| e.1),
        Some(SessionEndReason::BrokerRestart)
    );
    h.verify();
}

/// c02. Design 5.8: quit ends every live session and its session grants with `app_quit`, and
/// records a clean end of the run.
#[test]
fn quit_ends_session_grants_with_app_quit() {
    let mut h = Harness::new("c02", isabelle());
    h.setup();
    let (_, session_grant, _) = h.approved(1, "call-1", "session");
    let (_, kept, _) = h.approved(1, "call-2", "until_revoked");
    let session_id = session_of(&mut h, AGENT);

    let output = h.quit();
    assert!(output.halt.is_none());
    assert!(output.jobs.is_empty(), "no job begins after quit");

    assert_eq!(
        end_reason(&mut h, &session_grant),
        Some(GrantEndReason::AppQuit)
    );
    assert_eq!(end_reason(&mut h, &kept), None);
    let session = h
        .read(|r| r.session(&id(&session_id)))
        .expect("the session row stays");
    assert_eq!(session.ended.map(|e| e.1), Some(SessionEndReason::AppQuit));
    let clean: i64 = h
        .raw()
        .query_row(
            "SELECT clean FROM runs ORDER BY run_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("the run is recorded");
    assert_eq!(clean, 1, "quit records a clean end of the run");
    h.verify();
}

/// c03. Design 5.5 and 5.8: an until-revoked grant survives restarts; a revoked grant stays
/// revoked, with its first end unchanged.
#[test]
fn until_revoked_survives_restart_and_revoked_stays_revoked() {
    let mut h = Harness::new("c03", isabelle());
    h.setup();
    let (_, kept, _) = h.approved(1, "call-1", "until_revoked");
    let (_, revoked, _) = h.approved(1, "call-2", "until_revoked");
    let answer = h.revoke(&revoked);
    assert_eq!(
        answer["grant"]["ended"]["reason"],
        json!("revoked_by_owner")
    );
    let ended = answer["grant"]["ended"].clone();

    h.advance(1_000);
    let _ = h.restart();
    h.advance(1_000);
    let _ = h.restart();
    let live = h.live_grants();
    assert!(live.contains(&kept));
    assert!(!live.contains(&revoked));

    h.kill();
    assert_eq!(end_reason(&mut h, &kept), None);
    let grant = h
        .read(|r| r.grant(&id(&revoked)))
        .expect("the revoked grant stays");
    assert_eq!(
        serde_json::to_value(&grant.ended).expect("an end serializes"),
        ended,
        "a restart changed a revoked grant's end"
    );
    h.verify();
}

/// c04. Design 5.5 and D18: a one-request grant is offered only to its origin request, stays
/// live while its change is dispatched or applied, and ends `consumed` when its request ends.
#[test]
fn a_one_request_grant_ends_consumed_with_its_request_and_serves_no_other() {
    let mut h = Harness::new("c04", isabelle());
    h.setup();
    let (origin, grant, fresh) = h.approved(1, "call-1", "one_request");

    // Another request of the same agent is not offered the grant and cannot use it.
    let (other, output) = h.propose(1, "call-2");
    let job = job_for(&output, &other, ProofPurpose::Classification);
    assert!(
        offered_grants(&job).iter().all(|(g, _)| *g != grant),
        "a one-request grant was offered to another request"
    );
    let output = h.prove(&job, Outcome::Permit, &[&grant]);
    assert!(output.halt.is_none());
    assert!(dispatches(&output).is_empty());
    assert_eq!(h.state(&other), "undecided");

    // The origin request uses it; the grant stays live while the change is handed off.
    let order = h.dispatch(&fresh, &[&grant]);
    assert_eq!(h.state(&origin), "dispatching");
    assert!(h.live_grants().contains(&grant));

    // The origin request ends: the grant ends consumed with it.
    let output = h.attempt(&order, rejected());
    assert!(output.halt.is_none());
    let details = h.details(&origin);
    assert_eq!(
        details["request"]["status"]["state"],
        json!("terminal_failure")
    );
    assert!(!h.live_grants().contains(&grant));

    // A second one-request grant whose change was applied stays live (D18).
    let (applied, applied_grant, fresh) = h.approved(1, "call-3", "one_request");
    let order = h.dispatch(&fresh, &[&applied_grant]);
    let _ = h.attempt(
        &order,
        fabric_broker_core::ports::AttemptReport::Applied { event: None },
    );
    assert_eq!(h.state(&applied), "applied");
    assert!(h.live_grants().contains(&applied_grant));

    h.kill();
    let ended = h
        .read(|r| r.grant(&id(&grant)))
        .expect("the grant stays")
        .ended
        .expect("the grant ended");
    assert_eq!(ended.reason, GrantEndReason::Consumed);
    assert_eq!(
        serde_json::to_value(&ended.at).expect("a timestamp serializes"),
        details["request"]["updated_at"],
        "the grant did not end in the commit that ended its request"
    );
    h.verify();
}

/// c11. Design 6.1 `decide_request` and 5.5: the approval records the digest of exactly the
/// decision taken, the grant is bound to the request's action, and the dispatch order names that
/// one action under an authorization for that request and grant.
#[test]
fn an_approval_binds_the_digest_and_the_order_names_one_action() {
    let mut h = Harness::new("c11", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    let details = h.details(&request_id);
    let request_digest = details["request"]["request_digest"].clone();
    let card_scope = details["card"]["scope"].clone();

    let (response, output) = h.decide(&request_id, "approve", json!({"kind": "one_request"}));
    let approval = result(&response)["approval"].clone();
    assert_eq!(approval["decision"], json!("approve"));
    assert_eq!(approval["scope"], card_scope);
    assert_eq!(approval["effective_request_digest"], request_digest);
    let scope: Scope = record(&card_scope);
    let duration: Duration = record(&json!({"kind": "one_request"}));
    let digest: Digest = serde_json::from_value(request_digest).expect("a digest");
    let expected = approval_digest(
        &id(&request_id),
        &digest,
        &Decision::Approve { scope, duration },
    );
    assert_eq!(
        approval["approval_digest"],
        serde_json::to_value(&expected).expect("a digest serializes")
    );

    let action = create_action(create_params(ACCOUNT, CALENDAR, "call-1"));
    let grant_id = approval["grant_id"]
        .as_str()
        .expect("a grant id")
        .to_owned();
    let grant = h.snapshot()["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .find(|g| g["grant_id"] == json!(grant_id))
        .cloned()
        .expect("the grant is live");
    assert_eq!(
        grant["term"],
        json!({"kind": "one_request", "action_digest": action.digest()})
    );
    assert_eq!(grant["origin"]["request_id"], json!(request_id));
    assert_eq!(
        grant["origin"]["approval_digest"],
        approval["approval_digest"]
    );

    let job = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    assert_eq!(job.action_digest, action.digest());
    let order = h.dispatch(&job, &[&grant_id]);
    assert_eq!(order.action, action, "the order names another action");
    assert_eq!(order.authorization.action_digest, action.digest());
    assert_eq!(order.authorization.request_id.as_str(), request_id);
    assert_eq!(order.authorization.grant_id.as_str(), grant_id);
    assert_eq!(order.attempt, 1);
    h.verify();
}

/// c11. Design 5.5 and D22: a session grant binds the agent's current live session, which may
/// be later than the request's; with no live session the duration is refused.
#[test]
fn a_session_grant_binds_the_current_live_session() {
    let mut h = Harness::new("c11b", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    let first = session_of(&mut h, AGENT);
    let _ = h.connect(2, AGENT);
    let current = session_of(&mut h, AGENT);
    assert_ne!(first, current);

    let (grant_id, _) = h.approve(&request_id, "session");
    let grant = h.snapshot()["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .find(|g| g["grant_id"] == json!(grant_id))
        .cloned()
        .expect("the grant is live");
    let term: GrantTerm = serde_json::from_value(grant["term"].clone()).expect("a term");
    assert_eq!(
        term,
        GrantTerm::Session {
            session_id: id(&current)
        }
    );

    let second = h.pending(2, "call-2");
    let _ = h.disconnect(2);
    let (response, _) = h.decide(&second, "approve", json!({"kind": "session"}));
    let refusal = error(&response);
    assert_eq!(refusal["code"], json!("invalid_params"));
    assert_eq!(refusal["detail"]["pointer"], json!("/params/duration"));
    assert_eq!(h.state(&second), "pending_approval");
    h.verify();
}

/// The connected account ids of a proof's facts, sorted.
fn connected_of(job: &ProofJob) -> Vec<String> {
    let mut connected = strings(&job.facts.connected);
    connected.sort();
    connected
}

/// The selected calendars of a proof's facts as `(account, calendar)` pairs, sorted.
fn calendars_of(job: &ProofJob) -> Vec<(String, String)> {
    let mut calendars: Vec<(String, String)> = job
        .facts
        .calendars
        .iter()
        .map(|c| {
            (
                c.calendar.account_id.as_str().to_owned(),
                c.calendar.calendar_id.as_str().to_owned(),
            )
        })
        .collect();
    calendars.sort();
    calendars
}

/// c21. Design 3.5 (per-account namespacing), 10.3 and 6.1 `disconnect_google`: a proof sees
/// every connected account and every selected calendar, each calendar named with its account
/// (11.3 W2: no filter by the action's account), and disconnecting one account
/// ends that account's grants and stales its pending requests only, even where the other
/// account has the same calendar id. After the disconnect, a proof no longer sees that account.
#[test]
fn grants_and_calendars_are_namespaced_by_account() {
    let mut h = Harness::new("c21", isabelle());
    let _ = h.seed(
        &[AGENT],
        vec![
            account("acct-a", &[CALENDAR]),
            account("acct-b", &[CALENDAR]),
        ],
    );
    let _ = h.connect(1, AGENT);

    let (on_b, output) = h.propose_on(1, "call-b1", "acct-b", CALENDAR);
    let job = job_for(&output, &on_b, ProofPurpose::Classification);
    assert_eq!(
        connected_of(&job),
        vec!["acct-a".to_owned(), "acct-b".to_owned()]
    );
    assert_eq!(
        calendars_of(&job),
        vec![
            ("acct-a".to_owned(), CALENDAR.to_owned()),
            ("acct-b".to_owned(), CALENDAR.to_owned()),
        ],
        "the same calendar id of two accounts must be two calendars, each named with its account"
    );
    let _ = h.prove(&job, Outcome::NeedApproval, &[]);

    let (_, grant_a, _) = h.approved_on(1, "call-a1", "acct-a", CALENDAR, "until_revoked");
    let (_, grant_b, _) = h.approved_on(1, "call-b2", "acct-b", CALENDAR, "until_revoked");
    let pending_a = h.pending_on(1, "call-a2", "acct-a", CALENDAR);

    let revision = h.snapshot()["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|a| a["account_id"] == json!("acct-a"))
        .expect("account A")["revision"]
        .clone();
    let answer = h.ok(
        "disconnect_google",
        json!({"account_id": "acct-a", "expected_revision": revision, "revoke_provider_token": false}),
    );
    assert_eq!(answer["ended_grant_ids"], json!([grant_a]));
    assert_eq!(answer["account"]["state"], json!("disconnected"));
    assert_eq!(h.state(&pending_a), "stale");
    assert_eq!(h.state(&on_b), "pending_approval");
    let live = h.live_grants();
    assert!(live.contains(&grant_b));
    assert!(!live.contains(&grant_a));

    let (after, output) = h.propose_on(1, "call-b3", "acct-b", CALENDAR);
    let job = job_for(&output, &after, ProofPurpose::Classification);
    assert_eq!(connected_of(&job), vec!["acct-b".to_owned()]);
    assert_eq!(
        calendars_of(&job),
        vec![("acct-b".to_owned(), CALENDAR.to_owned())],
        "a proof after the disconnect saw a calendar of the disconnected account"
    );
    assert!(
        job.facts
            .grants
            .iter()
            .all(|g| g.grant.grant_id.as_str() != grant_a),
        "a proof after the disconnect was offered the disconnected account's grant"
    );

    h.kill();
    assert_eq!(
        end_reason(&mut h, &grant_a),
        Some(GrantEndReason::AccountDisconnected)
    );
    assert_eq!(end_reason(&mut h, &grant_b), None);
    h.verify();
}

/// A live until-revoked grant of `AGENT` whose scope names `recipient_sets`.
fn grant_naming(
    grant_id: &str,
    origin: &str,
    recipient_sets: &[&str],
    epoch: u64,
    at: u64,
) -> Value {
    let recipients = if recipient_sets.is_empty() {
        Value::Null
    } else {
        json!({"kind": "recipient_sets", "recipient_set_ids": recipient_sets})
    };
    json!({
        "grant_id": grant_id,
        "revision": 1,
        "agent_id": AGENT,
        "purpose": "Invite the team.",
        "tools": ["fabric_calendar_propose_invitation"],
        "scope": {
            "calendars": [{"account_id": ACCOUNT, "calendar_id": CALENDAR}],
            "window": null,
            "fields": null,
            "recipients": recipients,
            "send_updates": "all"
        },
        "term": {"kind": "until_revoked", "expires_at": null},
        "origin": {"request_id": origin, "approval_digest": format!("sha256:{}", "c".repeat(64))},
        "policy": {"profile_id": "careful", "version": 1},
        "revocation_epoch": epoch,
        "created_at": timestamp(at),
        "ended": null
    })
}

/// Saves `set` with `members`; returns the new version.
fn save(h: &mut Harness, set: &Value, members: &[&str]) -> Value {
    h.ok(
        "save_group",
        json!({
            "expected_revision": set["revision"],
            "change": {
                "kind": "save",
                "recipient_set_id": set["recipient_set_id"],
                "definition": {"name": set["definition"]["name"], "members": members, "domains": []}
            }
        }),
    )["recipient_set"]
        .clone()
}

/// c23. Design 5.6, D19 and 6.1 `save_group`: narrowing a recipient set ends no grant; adding a
/// member or deleting the set ends the live grants whose scope names it, with
/// `authority_reduced`; other grants stay. The grants that name a set are written to the store
/// directly, because W04 approves only the card's scope (D22, `selected` is refused).
#[test]
fn widening_or_deleting_a_recipient_set_ends_the_grants_that_name_it() {
    let mut h = Harness::new("c23", isabelle());
    h.setup();
    let team = h.create_group("Team", &["ally@example.net", "sam@example.org"]);
    let family = h.create_group("Family", &["kim@example.net"]);
    let team_id = team["recipient_set_id"].as_str().expect("an id").to_owned();
    let family_id = family["recipient_set_id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let origin = h.pending(1, "call-1");
    let _ = h.decide(&origin, "deny", Value::Null);

    let _ = h.quit();
    let epoch = h.read(|r| r.counters()).revocation_epoch;
    let at = h.now;
    let mutations = [
        grant_naming("grant-team", &origin, &[&team_id], epoch, at),
        grant_naming("grant-family", &origin, &[&family_id], epoch, at),
        grant_naming("grant-plain", &origin, &[], epoch, at),
    ]
    .iter()
    .map(|g| Mutation::GrantInserted { grant: record(g) })
    .collect();
    h.write(mutations);
    h.advance(1_000);
    let _ = h.start();

    // Narrowing applies at once and ends nothing.
    let team = save(&mut h, &team, &["ally@example.net"]);
    assert!(h.live_grants().contains(&"grant-team".to_owned()));

    // Adding a member ends the grant naming the set.
    let _ = save(&mut h, &team, &["ally@example.net", "pat@example.net"]);
    let live = h.live_grants();
    assert!(!live.contains(&"grant-team".to_owned()));
    assert!(live.contains(&"grant-family".to_owned()));
    assert!(live.contains(&"grant-plain".to_owned()));

    // Deleting a set ends the grant naming it.
    let answer = h.ok(
        "save_group",
        json!({
            "expected_revision": family["revision"],
            "change": {"kind": "delete", "recipient_set_id": family_id}
        }),
    );
    assert_eq!(answer["recipient_set"], Value::Null);
    let live = h.live_grants();
    assert!(!live.contains(&"grant-family".to_owned()));
    assert!(live.contains(&"grant-plain".to_owned()));

    h.kill();
    for grant in ["grant-team", "grant-family"] {
        assert_eq!(
            end_reason(&mut h, grant),
            Some(GrantEndReason::AuthorityReduced),
            "{grant}"
        );
    }
    assert_eq!(end_reason(&mut h, "grant-plain"), None);
    h.verify();
}
