//! The duration a card proposes and the refusal of a longer one (contract schema version 2,
//! README section 10; D22, D40 and the tests c37 to c39).
//! Every card here is opened by the real `open_card` from a scripted proof result.

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::SESSION_IDLE_MS;
use fabric_broker_core::ports::{ProofDone, ProofJob};
use fabric_desktop_contracts::owner::{GrantEndReason, Mode, ProfileRows, ProofPurpose};
use serde_json::{Value, json};

const IDLE: u64 = SESSION_IDLE_MS as u64;

/// The message of the contract's `invalid_params` fixture for `decide_request`.
fn fixture_message() -> Value {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/desktop/v1/fixtures/messages/",
            "accept-owner-response-error-invalid-params.json"
        ))
        .expect("the fixture reads"),
    )
    .expect("the fixture is JSON");
    let error = &fixture["value"]["error"];
    assert_eq!(error["detail"]["pointer"], json!("/params/duration/kind"));
    error["message"].clone()
}

/// The duration the card of `request_id` proposes.
fn proposed(h: &mut Harness, request_id: &str) -> String {
    h.details(request_id)["card"]["proposed_duration"]["kind"]
        .as_str()
        .expect("the request has a card")
        .to_owned()
}

/// Asserts that `response` is the refusal of a duration longer than the card proposes.
fn assert_longer_refused(response: &Value) {
    let refusal = error(response);
    assert_eq!(refusal["code"], json!("invalid_params"));
    assert_eq!(refusal["detail"]["pointer"], json!("/params/duration/kind"));
    assert_eq!(refusal["message"], fixture_message());
    assert_eq!(refusal["retryable"], json!(false));
    assert_eq!(refusal["recovery"], json!("report_problem"));
}

/// The term kind of the live grant `grant_id` in the snapshot.
fn term_of(h: &mut Harness, grant_id: &str) -> Value {
    h.snapshot()["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .find(|g| g["grant_id"] == json!(grant_id))
        .unwrap_or_else(|| panic!("no live grant {grant_id}"))["term"]
        .clone()
}

/// The decide_request params of a pending request, as `Harness::decide` sends them.
fn decide_params(h: &mut Harness, request_id: &str, decision: &str, duration: Value) -> Value {
    let details = h.details(request_id);
    json!({
        "request_id": request_id,
        "request_digest": details["request"]["request_digest"],
        "expected_revision": details["request"]["revision"],
        "decision": decision,
        "selected": null,
        "duration": duration
    })
}

/// One agent call of `tool` that is asked of the owner: the request waits in
/// `pending_approval` with a card.
fn pending_call(h: &mut Harness, call_id: &str, tool: &str, params: Value) -> String {
    let (reply, output) = h.call(1, call_id, tool, params);
    let request_id = request_id_of(&reply);
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let _ = h.prove(&job, Outcome::NeedApproval, &[]);
    assert_eq!(h.state(&request_id), "pending_approval", "{tool}");
    request_id
}

/// Saves a custom profile with `rows`, acknowledging its expansion, and activates it.
fn activate_custom(h: &mut Harness, name: &str, rows: &Value) {
    let revision = h.snapshot()["profiles"]["revision"].clone();
    let save = |acknowledged: Value| {
        json!({"expected_revision": revision,
               "change": {"kind": "save_custom",
                          "profile": {"profile_id": null, "name": name, "rows": rows},
                          "acknowledged_expansion": acknowledged}})
    };
    let (response, _) = h.owner("save_profile", save(Value::Null));
    let answer = match response.get("error") {
        Some(refusal) => {
            assert_eq!(refusal["code"], json!("expansion_not_acknowledged"));
            h.ok(
                "save_profile",
                save(refusal["detail"]["expansion_digest"].clone()),
            )
        }
        None => result(&response),
    };
    let profile_id = answer["profiles"]["items"]
        .as_array()
        .expect("profiles")
        .iter()
        .find(|p| p["name"] == json!(name))
        .expect("the saved profile")["profile_id"]
        .as_str()
        .expect("a profile id")
        .to_owned();
    let _ = h.activate(&profile_id);
    assert_eq!(
        h.snapshot()["profiles"]["active_profile_id"],
        json!(profile_id)
    );
}

/// Careful's rows with `changes` applied.
fn careful_with(changes: &[(&str, Mode)]) -> Value {
    let mut rows = serde_json::to_value(ProfileRows::CAREFUL).expect("rows");
    for (row, mode) in changes {
        rows[*row] = serde_json::to_value(mode).expect("a mode");
    }
    rows
}

fn window() -> Value {
    json!({"start": "2026-11-27T06:00:00.000Z", "end": "2026-11-28T06:00:00.000Z"})
}

fn event() -> Value {
    json!({"event_id": "evt-fabric-0042", "revision": "3181161799000000"})
}

/// c38 (PR 28 and PR 24). A Careful create in an existing calendar proposes one_request, and
/// every longer duration is refused at /params/duration/kind with the contract's message; the
/// request and the store stay as they were, and one_request is then approved.
#[test]
fn careful_create_in_an_existing_calendar_proposes_one_request_and_refuses_longer() {
    let mut h = Harness::new("dur-i1", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    assert_eq!(proposed(&mut h, &request_id), "one_request");
    let revision = h.details(&request_id)["request"]["revision"].clone();

    for duration in ["until_revoked", "session"] {
        let (response, output) = h.decide(&request_id, "approve", json!({"kind": duration}));
        assert_longer_refused(&response);
        assert!(notices(&output).is_empty(), "a refusal sent a notice");
        assert!(output.jobs.is_empty(), "a refusal started a proof");
    }
    let details = h.details(&request_id);
    assert_eq!(
        details["request"]["status"]["state"],
        json!("pending_approval")
    );
    assert_eq!(details["request"]["revision"], revision);
    assert_eq!(details["approval"], Value::Null);
    assert_eq!(h.snapshot()["grants"], json!([]));

    let (grant, _) = h.approve(&request_id, "one_request");
    assert_eq!(term_of(&mut h, &grant)["kind"], json!("one_request"));
    assert_eq!(h.state(&request_id), "proving_fresh_permit");
    h.verify();
}

/// c37. A Careful create in Agent Drafts proposes until_revoked: until_revoked, a session bound
/// to the current live session and one_request are all accepted.
#[test]
fn careful_create_in_agent_drafts_proposes_until_revoked() {
    let mut h = Harness::new("dur-i2", isabelle());
    h.setup();
    let session = h.agent_view(AGENT)["session"]["session_id"].clone();
    let lasting = h.pending_lasting(1, "call-1");
    let bound = h.pending_lasting(1, "call-2");
    let single = h.pending_lasting(1, "call-3");
    for request_id in [&lasting, &bound, &single] {
        assert_eq!(proposed(&mut h, request_id), "until_revoked");
    }

    let (grant, _) = h.approve(&lasting, "until_revoked");
    assert_eq!(term_of(&mut h, &grant)["kind"], json!("until_revoked"));
    let (grant, job) = h.approve(&bound, "session");
    assert_eq!(
        term_of(&mut h, &grant),
        json!({"kind": "session", "session_id": session})
    );
    assert_eq!(
        job.facts.session.as_ref().map(|s| s.session_id.as_str()),
        session.as_str()
    );
    let (grant, _) = h.approve(&single, "one_request");
    assert_eq!(term_of(&mut h, &grant)["kind"], json!("one_request"));
    h.verify();
}

/// c37. The profile in force decides: under Flexible a create in an existing calendar proposes
/// until_revoked, under Careful one_request.
#[test]
fn flexible_create_in_an_existing_calendar_proposes_until_revoked() {
    let mut h = Harness::new("dur-i3", isabelle());
    h.setup();
    let careful = h.pending(1, "call-1");
    assert_eq!(proposed(&mut h, &careful), "one_request");
    let _ = h.activate("flexible");
    let flexible = h.pending(1, "call-2");
    assert_eq!(proposed(&mut h, &flexible), "until_revoked");
    let (grant, _) = h.approve(&flexible, "until_revoked");
    assert_eq!(term_of(&mut h, &grant)["kind"], json!("until_revoked"));
    h.verify();
}

/// c37 (D40). Through the real `open_card` and the store's calendar access: under Flexible a
/// create on a calendar the owner can only read, or see free/busy of, proposes one_request
/// (the theory's `writable` holds for owner and writer only), and the proof's facts mark it
/// read-only. The owner calendar of the same account still proposes until_revoked.
#[test]
fn a_create_on_a_calendar_without_write_access_proposes_one_request() {
    let mut h = Harness::new("dur-read-only", isabelle());
    let mut seeded = serde_json::to_value(account(ACCOUNT, &[CALENDAR])).expect("an account");
    for (calendar_id, access) in [("cal-shared", "reader"), ("cal-busy", "free_busy_reader")] {
        let calendar = json!({"calendar_id": calendar_id, "name": "Shared", "access": access,
                              "selected": true, "agent_drafts": false});
        seeded["calendars"]
            .as_array_mut()
            .expect("calendars")
            .push(calendar);
    }
    let _ = h.seed(&[AGENT], vec![record(&seeded)]);
    let _ = h.connect(1, AGENT);
    let _ = h.activate("flexible");

    let owned = h.pending_on(1, "call-1", ACCOUNT, CALENDAR);
    assert_eq!(proposed(&mut h, &owned), "until_revoked");
    for (call_id, calendar_id) in [("call-2", "cal-shared"), ("call-3", "cal-busy")] {
        let (request_id, output) = h.propose_on(1, call_id, ACCOUNT, calendar_id);
        let job = job_for(&output, &request_id, ProofPurpose::Classification);
        let fact = job
            .facts
            .calendars
            .iter()
            .find(|c| c.calendar.calendar_id.as_str() == calendar_id)
            .expect("the calendar is among the facts");
        assert!(!fact.writable, "{calendar_id}");
        let _ = h.prove(&job, Outcome::NeedApproval, &[]);
        assert_eq!(h.state(&request_id), "pending_approval", "{calendar_id}");
        assert_eq!(
            proposed(&mut h, &request_id),
            "one_request",
            "{calendar_id}"
        );
        let (response, _) = h.decide(&request_id, "approve", json!({"kind": "until_revoked"}));
        assert_longer_refused(&response);
    }
    h.verify();
}

/// c37. A custom profile in force, through the real `open_card`: create_in_existing_calendar
/// allow gives until_revoked on a plain calendar, create_in_agent_drafts ask gives one_request
/// on Agent Drafts.
#[test]
fn a_custom_profile_in_force_sets_the_ceiling() {
    let mut h = Harness::new("dur-custom", isabelle());
    h.setup();
    let rows = careful_with(&[
        ("create_in_agent_drafts", Mode::Ask),
        ("create_in_existing_calendar", Mode::Allow),
    ]);
    activate_custom(&mut h, "Existing calendars only", &rows);
    let plain = h.pending(1, "call-1");
    assert_eq!(proposed(&mut h, &plain), "until_revoked");
    let drafts = h.pending_lasting(1, "call-2");
    assert_eq!(proposed(&mut h, &drafts), "one_request");
    let (response, _) = h.decide(&drafts, "approve", json!({"kind": "until_revoked"}));
    assert_longer_refused(&response);
    h.verify();
}

/// c37. A run that cannot carry out a change proposes one_request on every card, an
/// Agent Drafts create included: with the synthetic classifier, and with Isabelle but no
/// provider.
#[test]
fn a_run_that_cannot_carry_out_a_change_proposes_one_request() {
    for (label, attachments) in [
        ("dur-synthetic", synthetic()),
        ("dur-no-provider", isabelle_without_provider()),
    ] {
        let mut h = Harness::new(label, attachments);
        h.setup();
        let _ = h.activate("flexible");
        let drafts = h.pending_lasting(1, "call-1");
        assert_eq!(proposed(&mut h, &drafts), "one_request", "{label}");
        let plain = h.pending(1, "call-2");
        assert_eq!(proposed(&mut h, &plain), "one_request", "{label}");
        let (response, _) = h.decide(&drafts, "approve", json!({"kind": "until_revoked"}));
        assert_longer_refused(&response);
        h.verify();
    }
}

/// c37. Reads and availability propose one_request under Careful and Flexible, a deliberate
/// under-proposal while no read finishes in this build; until_revoked is refused.
#[test]
fn reads_and_availability_propose_one_request() {
    let mut h = Harness::new("dur-i4", isabelle());
    h.setup();
    for profile in ["careful", "flexible"] {
        let _ = h.activate(profile);
        for calendar in [CALENDAR, DRAFTS] {
            let calls = [
                (
                    "fabric_calendar_availability",
                    json!({"account_id": ACCOUNT, "calendar_ids": [calendar],
                           "window": window(), "task": "Find a free hour."}),
                ),
                (
                    "fabric_calendar_read",
                    json!({"account_id": ACCOUNT, "calendar_ids": [calendar],
                           "window": window(), "fields": ["time", "title"],
                           "task": "See what is on Friday."}),
                ),
            ];
            for (tool, params) in calls {
                let call_id = format!("{profile}-{calendar}-{tool}");
                let request_id = pending_call(&mut h, &call_id, tool, params);
                assert_eq!(
                    proposed(&mut h, &request_id),
                    "one_request",
                    "{tool} on {calendar} under {profile}"
                );
                let (response, _) =
                    h.decide(&request_id, "approve", json!({"kind": "until_revoked"}));
                assert_longer_refused(&response);
                let _ = h.decide(&request_id, "deny", Value::Null);
            }
        }
    }
    h.verify();
}

/// c37. Updates, invitations, responses and deletes propose one_request on Agent Drafts and on
/// an existing calendar under Careful, Flexible and a custom profile that allows every row it
/// can.
#[test]
fn updates_invitations_responses_and_deletes_propose_one_request() {
    let mut h = Harness::new("dur-i5", isabelle());
    h.setup();
    let everything = careful_with(&[
        ("read_event_details", Mode::Allow),
        ("create_in_existing_calendar", Mode::Allow),
        ("edit_fabric_created_event", Mode::Allow),
        ("edit_preexisting_event", Mode::Allow),
        ("send_or_change_invitations", Mode::Allow),
        ("respond_to_invitation", Mode::Allow),
    ]);
    for profile in ["careful", "flexible", "custom"] {
        if profile == "custom" {
            activate_custom(&mut h, "Everything allowed", &everything);
        } else {
            let _ = h.activate(profile);
        }
        for calendar in [CALENDAR, DRAFTS] {
            // Past the minute the per-agent rate limit counts over.
            h.advance(61_000);
            let calls = [
                (
                    "fabric_calendar_propose_update",
                    json!({"account_id": ACCOUNT, "calendar_id": calendar, "event": event(),
                           "changes": {"title": "Design review", "start": null, "end": null,
                                       "description": null, "location": null},
                           "send_updates": "none", "task": "Rename the event."}),
                ),
                (
                    "fabric_calendar_propose_invitation",
                    json!({"account_id": ACCOUNT, "calendar_id": calendar, "event": event(),
                           "attendees": ["ally@example.net"], "send_updates": "all",
                           "task": "Invite the team."}),
                ),
                (
                    "fabric_calendar_propose_response",
                    json!({"account_id": ACCOUNT, "calendar_id": calendar,
                           "target": {"kind": "event", "event": event()},
                           "response": "accepted", "send_updates": "all",
                           "task": "Answer the invitation."}),
                ),
                (
                    "fabric_calendar_propose_delete",
                    json!({"account_id": ACCOUNT, "calendar_id": calendar, "event": event(),
                           "send_updates": "none", "task": "Remove the draft I made."}),
                ),
            ];
            for (tool, params) in calls {
                let call_id = format!("{profile}-{calendar}-{tool}");
                let request_id = pending_call(&mut h, &call_id, tool, params);
                assert_eq!(
                    proposed(&mut h, &request_id),
                    "one_request",
                    "{tool} on {calendar} under {profile}"
                );
                let _ = h.decide(&request_id, "deny", Value::Null);
            }
        }
    }
    h.verify();
}

/// c38. The ceiling is checked before the session: with the agent disconnected, a session on a
/// one_request card is refused at /params/duration/kind; on an until_revoked card at
/// /params/duration with `create_grant`'s message, while one_request and until_revoked are
/// accepted.
#[test]
fn the_ceiling_comes_before_the_session_check() {
    let mut h = Harness::new("dur-i6", isabelle());
    h.setup();
    let single = h.pending(1, "call-1");
    let lasting = h.pending_lasting(1, "call-2");
    let other = h.pending_lasting(1, "call-3");
    let _ = h.disconnect(1);
    assert_eq!(h.agent_view(AGENT)["session"], Value::Null);

    let (response, _) = h.decide(&single, "approve", json!({"kind": "session"}));
    assert_longer_refused(&response);

    let (response, _) = h.decide(&lasting, "approve", json!({"kind": "session"}));
    let refusal = error(&response);
    assert_eq!(refusal["code"], json!("invalid_params"));
    assert_eq!(refusal["detail"]["pointer"], json!("/params/duration"));
    assert_eq!(
        refusal["message"],
        json!(
            "The agent has no live session, so a session grant cannot be bound to one. \
             Approve it for one request or until revoked."
        )
    );
    assert_eq!(h.state(&lasting), "pending_approval");

    let _ = h.approve(&lasting, "until_revoked");
    let _ = h.approve(&other, "one_request");
    let _ = h.approve(&single, "one_request");
    h.verify();
}

/// c38. `selected` is checked before the ceiling.
#[test]
fn selected_is_checked_before_the_ceiling() {
    let mut h = Harness::new("dur-i7", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    let mut params = decide_params(
        &mut h,
        &request_id,
        "approve",
        json!({"kind": "until_revoked"}),
    );
    params["selected"] = h.details(&request_id)["card"]["scope"].clone();
    let refusal = error(&h.owner("decide_request", params).0);
    assert_eq!(refusal["code"], json!("invalid_params"));
    assert_eq!(refusal["detail"]["pointer"], json!("/params/selected"));
    h.verify();
}

/// c38. A denial carries no duration and is never checked against the ceiling.
#[test]
fn deny_is_not_checked() {
    let mut h = Harness::new("dur-i8", isabelle());
    h.setup();
    let single = h.pending(1, "call-1");
    let lasting = h.pending_lasting(1, "call-2");
    for request_id in [&single, &lasting] {
        let (response, _) = h.decide(request_id, "deny", Value::Null);
        assert_eq!(result(&response)["approval"]["decision"], json!("deny"));
        assert_eq!(h.state(request_id), "denied");
    }
    h.verify();
}

/// c38. The stored card is compared, never computed again: a one_request card stays one_request
/// after Flexible is activated, and refuses until_revoked.
#[test]
fn a_stored_one_request_card_refuses_longer_after_a_widening() {
    let mut h = Harness::new("dur-i9", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    let _ = h.activate("flexible");
    assert_eq!(proposed(&mut h, &request_id), "one_request");
    let (response, _) = h.decide(&request_id, "approve", json!({"kind": "until_revoked"}));
    assert_longer_refused(&response);
    assert_eq!(h.state(&request_id), "pending_approval");
    h.verify();
}

/// c38 (D20). A stale until_revoked ceiling ends asked again: the approval is accepted under
/// the stored ceiling, the fresh proof under the narrower profile asks again, the new card
/// proposes one_request and can be decided, the lasting grant stays live, and the request goes
/// on to a dispatch.
#[test]
fn a_stale_until_revoked_ceiling_is_accepted_and_asked_again() {
    let mut h = Harness::new("dur-i10", isabelle());
    h.setup();
    let _ = h.activate("flexible");
    let request_id = h.pending(1, "call-1");
    let card = h.details(&request_id)["card"].clone();
    assert_eq!(card["proposed_duration"]["kind"], json!("until_revoked"));
    let _ = h.activate("careful");
    assert_eq!(
        h.details(&request_id)["card"],
        card,
        "an activation rewrote a card"
    );

    let (lasting, fresh) = h.approve(&request_id, "until_revoked");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    let details = h.details(&request_id);
    assert_eq!(
        details["request"]["status"]["state"],
        json!("pending_approval")
    );
    assert_eq!(
        details["card"]["proposed_duration"]["kind"],
        json!("one_request")
    );
    assert_eq!(details["approval"], Value::Null);
    assert_eq!(term_of(&mut h, &lasting)["kind"], json!("until_revoked"));

    let (single, fresh) = h.approve(&request_id, "one_request");
    assert_eq!(relied(&fresh), vec![single.clone()]);
    let order = h.dispatch(&fresh, &[&single]);
    assert_eq!(order.request_id.as_str(), request_id);
    assert!(h.live_grants().contains(&lasting));
    h.verify();
}

/// c38 (D20). A stale until_revoked ceiling ends blocked: the row now blocks, the fresh proof
/// blocks, and the lasting grant stays live.
#[test]
fn a_stale_until_revoked_ceiling_can_end_blocked() {
    let mut h = Harness::new("dur-i10b", isabelle());
    h.setup();
    let _ = h.activate("flexible");
    let request_id = h.pending(1, "call-1");
    assert_eq!(proposed(&mut h, &request_id), "until_revoked");
    activate_custom(
        &mut h,
        "No existing calendars",
        &careful_with(&[("create_in_existing_calendar", Mode::Block)]),
    );
    let (lasting, fresh) = h.approve(&request_id, "until_revoked");
    let _ = h.prove(&fresh, Outcome::Block, &[]);
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("blocked"));
    assert_eq!(details["approval"]["grant_id"], json!(lasting));
    assert!(h.live_grants().contains(&lasting));
    h.verify();
}

/// A scripted Block of `job` whose failing rule and ruling name `rule`.
fn block_on(job: &ProofJob, rule: &str) -> ProofDone {
    let mut done = proof_done(job, Outcome::Block, &[]);
    let mut report = serde_json::to_value(done.report.as_ref().expect("a report")).expect("json");
    report["failing_rule"] = json!(rule);
    let proof_id = report["proof_id"].clone();
    done.report = Some(record(&report));
    done.ruling = Some(record(&json!({
        "rule": rule,
        "explanation": "The test prover says so.",
        "proof_id": proof_id
    })));
    done
}

/// c38 (D20). A stale until_revoked ceiling ends blocked when the agent was paused: the pause
/// leaves the card's ceiling alone, an until_revoked approval is accepted and its grant made,
/// the fresh proof blocks on `agent_not_paused` (a scripted Permit would be undecided,
/// `agent_paused`), and the lasting grant stays live. After a resume it serves the agent's next
/// create in that calendar.
#[test]
fn a_stale_until_revoked_ceiling_ends_blocked_when_the_agent_is_paused() {
    let mut h = Harness::new("dur-paused", isabelle());
    h.setup();
    let blocked = h.pending_lasting(1, "call-1");
    let undecided = h.pending_lasting(1, "call-2");
    assert_eq!(proposed(&mut h, &blocked), "until_revoked");
    let _ = h.pause(AGENT, true);
    assert_eq!(proposed(&mut h, &blocked), "until_revoked");

    let (lasting, fresh) = h.approve(&blocked, "until_revoked");
    assert!(fresh.facts.paused, "the fresh proof does not see the pause");
    let _ = h.prove_with(block_on(&fresh, "agent_not_paused"));
    let details = h.details(&blocked);
    assert_eq!(details["request"]["status"]["state"], json!("blocked"));
    assert_eq!(details["approval"]["grant_id"], json!(lasting));
    assert!(h.live_grants().contains(&lasting));

    let (other, fresh) = h.approve(&undecided, "until_revoked");
    let _ = h.prove(&fresh, Outcome::Permit, &[&other]);
    assert_eq!(h.state(&undecided), "undecided");
    assert_eq!(h.stored_cause(&undecided).as_deref(), Some("agent_paused"));
    assert!(h.live_grants().contains(&other));

    let _ = h.pause(AGENT, false);
    let (request_id, output) = h.propose_on(1, "call-3", ACCOUNT, DRAFTS);
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    assert!(offered_grants(&job).contains(&(lasting.clone(), false)));
    let output = h.prove(&job, Outcome::Permit, &[&lasting]);
    let fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    let _ = h.dispatch(&fresh, &[&lasting]);
    assert_eq!(h.state(&request_id), "dispatching");
    h.verify();
}

/// c38 (D20). A card keeps its stored until_revoked proposal across a restart, also into a
/// run that cannot carry out a change: the approval is accepted under the stored ceiling and
/// its lasting grant made, the fresh Permit ends undecided (`no_provider` with Isabelle and no
/// provider, `no_prover` with the synthetic classifier), and the lasting grant stays live.
#[test]
fn a_stored_until_revoked_card_decided_in_a_run_that_cannot_change_ends_undecided() {
    for (label, later, cause) in [
        (
            "dur-later-no-provider",
            isabelle_without_provider(),
            "no_provider",
        ),
        ("dur-later-synthetic", synthetic(), "no_prover"),
    ] {
        let mut h = Harness::new(label, isabelle());
        h.setup();
        let request_id = h.pending_lasting(1, "call-1");
        let card = h.details(&request_id)["card"].clone();
        assert_eq!(card["proposed_duration"]["kind"], json!("until_revoked"));

        h.kill();
        h.attachments = later;
        h.advance(1_000);
        let _ = h.start();
        let details = h.details(&request_id);
        assert_eq!(
            details["request"]["status"]["state"],
            json!("pending_approval"),
            "{label}"
        );
        assert_eq!(details["card"], card, "{label}: a restart rewrote the card");

        let (lasting, fresh) = h.approve(&request_id, "until_revoked");
        assert_eq!(term_of(&mut h, &lasting)["kind"], json!("until_revoked"));
        let output = h.prove(&fresh, Outcome::Permit, &[&lasting]);
        assert!(dispatches(&output).is_empty(), "{label}");
        assert_eq!(h.state(&request_id), "undecided", "{label}");
        assert_eq!(
            h.stored_cause(&request_id).as_deref(),
            Some(cause),
            "{label}"
        );
        assert!(h.live_grants().contains(&lasting), "{label}");
        h.verify();
    }
}

/// c38. A refused duration replays: the same command id and params get the same bytes and
/// write nothing; the same id with another duration is command_reused.
#[test]
fn a_refused_duration_replays() {
    let mut h = Harness::new("dur-i11", isabelle());
    h.setup();
    let request_id = h.pending(1, "call-1");
    let params = decide_params(
        &mut h,
        &request_id,
        "approve",
        json!({"kind": "until_revoked"}),
    );
    let first = h.send("cmd-1", "decide_request", params.clone()).1;
    let first_line = response_frame(&first, "cmd-1").to_line();
    assert_longer_refused(&response(&first, "cmd-1"));
    let head = h.ledger_head();

    let again = h.send("cmd-1", "decide_request", params.clone()).1;
    assert_eq!(response_frame(&again, "cmd-1").to_line(), first_line);
    assert_eq!(h.ledger_head(), head, "a replay wrote");

    let mut other = params;
    other["duration"] = json!({"kind": "one_request"});
    let (reused, _) = h.send("cmd-1", "decide_request", other);
    assert_eq!(error(&reused)["code"], json!("command_reused"));
    assert_eq!(h.state(&request_id), "pending_approval");
    h.verify();
}

/// c39 (6.1). The details carry the approval outside pending_approval: after an approval and
/// after a denial.
#[test]
fn details_carry_the_approval_outside_pending_approval() {
    let mut h = Harness::new("dur-i12", isabelle());
    h.setup();
    let approved = h.pending(1, "call-1");
    let (grant, _) = h.approve(&approved, "one_request");
    let details = h.details(&approved);
    assert_eq!(
        details["request"]["status"]["state"],
        json!("proving_fresh_permit")
    );
    assert_eq!(details["approval"]["grant_id"], json!(grant));
    let denied = h.pending(1, "call-2");
    let _ = h.decide(&denied, "deny", Value::Null);
    assert_eq!(h.details(&denied)["approval"]["decision"], json!("deny"));
    h.verify();
}

/// c37 (D40). Asked again after a lasting approval, the request proposes one_request
/// although its row still allows; the new card can be decided and the request goes on.
#[test]
fn a_reask_after_a_lasting_approval_proposes_one_request() {
    let mut h = Harness::new("dur-i13", isabelle());
    h.setup();
    let request_id = h.pending_lasting(1, "call-1");
    assert_eq!(proposed(&mut h, &request_id), "until_revoked");
    let (lasting, fresh) = h.approve(&request_id, "until_revoked");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert_eq!(h.state(&request_id), "pending_approval");
    assert_eq!(proposed(&mut h, &request_id), "one_request");
    let (response, _) = h.decide(&request_id, "approve", json!({"kind": "until_revoked"}));
    assert_longer_refused(&response);

    let (single, fresh) = h.approve(&request_id, "one_request");
    let _ = h.dispatch(&fresh, &[&single]);
    assert_eq!(h.state(&request_id), "dispatching");
    assert!(h.live_grants().contains(&lasting));
    h.verify();
}

/// c37 (D40), the three-card case: the guard is the stored lasting grant, not the latest
/// approval. Card 1 proposes until_revoked and is approved so; card 2 proposes one_request and
/// is approved so; card 3 still proposes one_request.
#[test]
fn a_request_with_a_lasting_grant_never_proposes_until_revoked_again() {
    let mut h = Harness::new("dur-three", isabelle());
    h.setup();
    let request_id = h.pending_lasting(1, "call-1");
    let mut kinds = vec![proposed(&mut h, &request_id)];
    for duration in ["until_revoked", "one_request"] {
        let (_, fresh) = h.approve(&request_id, duration);
        let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
        kinds.push(proposed(&mut h, &request_id));
    }
    assert_eq!(kinds, ["until_revoked", "one_request", "one_request"]);

    // A one_request approval alone adds no lasting grant: the next card proposes the table's
    // answer again.
    let other = h.pending_lasting(1, "call-2");
    let (_, fresh) = h.approve(&other, "one_request");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert_eq!(proposed(&mut h, &other), "until_revoked");
    h.verify();
}

/// c37 (D40). A session approval is a lasting one too: asked again after it, the request
/// proposes one_request, and until_revoked and session are refused.
#[test]
fn a_reask_after_a_session_approval_proposes_one_request() {
    let mut h = Harness::new("dur-reask-session", isabelle());
    h.setup();
    let request_id = h.pending_lasting(1, "call-1");
    assert_eq!(proposed(&mut h, &request_id), "until_revoked");
    let (_, fresh) = h.approve(&request_id, "session");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert_eq!(h.state(&request_id), "pending_approval");
    assert_eq!(proposed(&mut h, &request_id), "one_request");
    for duration in ["until_revoked", "session"] {
        let (response, _) = h.decide(&request_id, "approve", json!({"kind": duration}));
        assert_longer_refused(&response);
    }
    h.verify();
}

/// c37 (D40). A lasting grant that has ended still counts: asked again after its lasting grant
/// was revoked, the request proposes one_request.
#[test]
fn a_reask_after_the_lasting_grant_is_revoked_proposes_one_request() {
    let mut h = Harness::new("dur-reask-revoked", isabelle());
    h.setup();
    let request_id = h.pending_lasting(1, "call-1");
    let (lasting, fresh) = h.approve(&request_id, "until_revoked");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert_eq!(h.state(&request_id), "pending_approval");
    let _ = h.revoke(&lasting);
    assert!(!h.live_grants().contains(&lasting));
    let (_, fresh) = h.approve(&request_id, "one_request");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert_eq!(h.state(&request_id), "pending_approval");
    assert_eq!(proposed(&mut h, &request_id), "one_request");
    let (response, _) = h.decide(&request_id, "approve", json!({"kind": "until_revoked"}));
    assert_longer_refused(&response);
    h.verify();
}

/// c39 (6.1). After a re-asked card expires, the details' approval is null and the lasting grant is
/// still listed.
#[test]
fn an_expired_reask_shows_no_approval_and_keeps_the_lasting_grant() {
    let mut h = Harness::new("dur-expired", isabelle());
    h.setup();
    let request_id = h.pending_lasting(1, "call-1");
    let (lasting, fresh) = h.approve(&request_id, "until_revoked");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    let expires = h.details(&request_id)["card"]["expires_at"].clone();
    h.now = T0 + 24 * 3_600_000 + 1_000;
    assert!(timestamp(h.now).as_str() > expires.as_str());
    let _ = h.tick();
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("expired"));
    assert_eq!(details["approval"], Value::Null);
    assert!(h.live_grants().contains(&lasting));
    h.verify();
}

/// c39 (6.1). A request that went stale while asked again shows the approval the store keeps, the
/// earlier one: the contract has no "waiting" meaning for a stale request, and the approval it
/// shows names the grant that the disconnect ended.
#[test]
fn a_stale_reask_shows_the_stored_approval() {
    let mut h = Harness::new("dur-stale", isabelle());
    h.setup();
    let request_id = h.pending_lasting(1, "call-1");
    let (lasting, fresh) = h.approve(&request_id, "until_revoked");
    let _ = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert_eq!(h.details(&request_id)["approval"], Value::Null);
    let revision = h.snapshot()["accounts"][0]["revision"].clone();
    let _ = h.ok(
        "disconnect_google",
        json!({"account_id": ACCOUNT, "expected_revision": revision,
               "revoke_provider_token": false}),
    );
    let details = h.details(&request_id);
    assert_eq!(details["request"]["status"]["state"], json!("stale"));
    assert_eq!(details["approval"]["grant_id"], json!(lasting));
    assert_eq!(
        details["approval"]["duration"],
        json!({"kind": "until_revoked"})
    );
    h.kill();
    let ended = h
        .read(|r| r.grant(&id(&lasting)))
        .expect("the grant stays")
        .ended
        .map(|e| e.reason);
    assert_eq!(ended, Some(GrantEndReason::AccountDisconnected));
    h.verify();
}

/// c39 (5.5). The session boundary is the theory's: at the last activity plus the idle limit
/// minus 1 ms a session decision is accepted, the job's session fact is present and the grant is
/// not ended; at exactly the limit a session decision is refused at /params/duration, the
/// earlier session grant has the stored end session_idle_timeout, a one-request approval's job
/// carries no session fact, and a call on the old connection is not permitted.
#[test]
fn a_session_decision_follows_the_idle_limit_to_the_millisecond() {
    let mut h = Harness::new("dur-idle", isabelle());
    h.setup();
    let early = h.pending_lasting(1, "call-1");
    let late = h.pending_lasting(1, "call-2");
    let single = h.pending(1, "call-3");
    let last = h.now;
    let session_id = h.agent_view(AGENT)["session"]["session_id"].clone();

    h.now = last + IDLE - 1;
    let (grant, job) = h.approve(&early, "session");
    let fact = job.facts.session.as_ref().expect("the job's session fact");
    assert_eq!(
        fact.session_id.as_str(),
        session_id.as_str().expect("an id")
    );
    assert!(offered_grants(&job).contains(&(grant.clone(), false)));
    assert_eq!(
        term_of(&mut h, &grant),
        json!({"kind": "session", "session_id": session_id})
    );

    h.now = last + IDLE;
    let (response, _) = h.decide(&late, "approve", json!({"kind": "session"}));
    let refusal = error(&response);
    assert_eq!(refusal["code"], json!("invalid_params"));
    assert_eq!(refusal["detail"]["pointer"], json!("/params/duration"));
    assert_eq!(h.agent_view(AGENT)["session"], Value::Null);
    assert!(!h.live_grants().contains(&grant));

    let (_, job) = h.approve(&single, "one_request");
    assert!(
        job.facts.session.is_none(),
        "a session fact after the limit"
    );
    let (reply, _) = h.poll(1, "call-4", &single);
    assert_eq!(reply["error"]["code"], json!("not_permitted"));

    h.kill();
    let ended = h
        .read(|r| r.grant(&id(&grant)))
        .expect("the grant stays")
        .ended
        .map(|e| e.reason);
    assert_eq!(ended, Some(GrantEndReason::SessionIdleTimeout));
    h.verify();
}
