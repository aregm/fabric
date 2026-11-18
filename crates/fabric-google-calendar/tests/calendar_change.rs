//! Changing events (6.6 to 6.8) and the reconciliation of an unknown outcome (section 7):
//! g28 to g36, with the read after a delete (18.7 point 1) and the member that differs in a
//! yes with another content (18.7 point 2).

// The loopback endpoints and `ManualClock` exist only in debug builds (PD2).
#![cfg(debug_assertions)]

mod calendar_common;

use calendar_common::{
    Person, World, ZONE, at, create, delete, event_ref, invitation, no_changes, occurrence, put,
    reserved, response, response_to, timed, update,
};
use fabric_desktop_contracts::agent::{Action, ProposedResponse};
use fabric_desktop_contracts::common::{FailureCode, SendUpdates};
use fabric_desktop_contracts::scalar::{EventDescription, EventLocation, EventTitle};
use fabric_google_calendar::Clock;
use fabric_google_calendar::calendar::{CalendarTarget, ChangeOrder};
use fabric_google_calendar::outcome::{
    ChangeOutcome, ChangeReport, Differs, Notification, ReadFailure, Sentence,
};
use fabric_google_calendar::reconcile::{ChangeTool, Effect, ReadBack, Verdict, decide};
use fabric_google_fake::{CalendarEntry, DeletedEventGet, Fault, FaultAction, LoggedRequest};
use serde_json::{Value, json};

fn run(
    world: &mut World,
    who: &Person,
    target: &CalendarTarget,
    action: Action,
    id: Option<String>,
) -> ChangeReport {
    let order = ChangeOrder::new(action, id, 1);
    let deadline = world.deadline();
    world.as_account(who, |g| g.change(&who.id, target, &order, deadline))
}

fn rejected(report: &ChangeReport) -> (FailureCode, &str) {
    let ChangeOutcome::Rejected { code, explanation } = &report.outcome else {
        panic!("rejected, not {report:?}");
    };
    (*code, explanation.as_str())
}

fn changes(world: &World) -> Vec<LoggedRequest> {
    world.fake.log().calendar_changes().entries().to_vec()
}

fn stored(world: &World, target: &CalendarTarget, id: &str) -> Value {
    world
        .fake
        .event(target.provider_id.as_str(), id)
        .expect("stored")
}

fn reconcile(
    world: &mut World,
    who: &Person,
    target: &CalendarTarget,
    effect: &Effect,
) -> ReadBack {
    let deadline = world.deadline();
    world
        .as_account(who, |g| g.reconcile_read(&who.id, target, effect, deadline))
        .expect("a read-back")
}

/// g28: the request is exactly 6.6; Applied carries id, revision and `confirmed`; an order
/// without a valid reserved id makes no request; 409 is Rejected; a 200 with another id is
/// NoAnswer.
#[test]
fn a_create_sends_exactly_6_6_and_is_applied_with_id_revision_and_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let id = reserved(1);
    let report = run(&mut world, &alice, &p, create(&p), Some(id.clone()));
    let ChangeOutcome::Applied {
        event,
        confirmed,
        differs,
        notification,
    } = &report.outcome
    else {
        panic!("applied, not {report:?}");
    };
    assert_eq!(*differs, None);
    let event = event.as_ref().expect("an event");
    assert_eq!(event.event_id.as_str(), id);
    assert_eq!(
        Some(event.revision.as_str().to_owned()),
        world
            .fake
            .with_state(|s| s.etag(p.provider_id.as_str(), &id))
    );
    assert!(*confirmed);
    assert_eq!(*notification, Notification::NotRequested);
    assert_eq!(report.side, None);
    let sent = changes(&world);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].segments,
        ["calendar", "v3", "calendars", "alice@example.com", "events"]
    );
    assert_eq!(
        sent[0].query,
        [("sendUpdates".to_owned(), "none".to_owned())]
    );
    assert_eq!(sent[0].if_match, None);
    assert_eq!(
        sent[0].body_json(),
        Some(json!({
            "id": id, "summary": "Planning",
            "start": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": ZONE},
            "end": {"dateTime": "2026-11-20T11:00:00+01:00", "timeZone": ZONE},
            "description": "Agenda", "location": "Room 1",
        }))
    );

    // No valid reserved id: no request.
    let before = world.fake.log().len();
    for bad in [
        None,
        Some(String::new()),
        Some("ABCDEFGHIJ".to_owned()),
        Some("abcd".to_owned()),
        Some("abc_defgh".to_owned()),
    ] {
        let report = run(&mut world, &alice, &p, create(&p), bad);
        assert_eq!(
            rejected(&report),
            (
                FailureCode::InvalidArguments,
                Sentence::ReservedIdForm.text()
            )
        );
    }
    assert_eq!(world.fake.log().len(), before);

    // The same reserved id again: 409, Rejected, and the event is not touched.
    let report = run(&mut world, &alice, &p, create(&p), Some(id.clone()));
    assert_eq!(
        rejected(&report),
        (FailureCode::ProviderRejected, Sentence::IdTaken.text())
    );
    assert_eq!(world.fake.events(p.provider_id.as_str()).len(), 1);

    // A 200 with another id is NoAnswer.
    world.fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars/",
        FaultAction::Json(
            200,
            json!({"id": "someotherid", "etag": "\"5\"", "summary": "Planning"}),
        ),
    ));
    let report = run(&mut world, &alice, &p, create(&p), Some(reserved(2)));
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    // A 200 whose body is not an event is NoAnswer too.
    world.fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars/",
        FaultAction::Json(200, json!(["x"])),
    ));
    let report = run(&mut world, &alice, &p, create(&p), Some(reserved(3)));
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    world.check_log();
}

/// g29: the answer lost after the fake applied the insert: NoAnswer, no second POST;
/// reconciliation finds the event and says Applied; the fake holds one event.
#[test]
fn a_create_whose_answer_is_lost_is_no_answer_and_reconciliation_finds_it_applied() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    world.fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars/",
        FaultAction::ApplyThenClose,
    ));
    let id = reserved(7);
    let report = run(&mut world, &alice, &p, create(&p), Some(id.clone()));
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    assert_eq!(changes(&world).len(), 1, "no second POST");
    let effect = Effect::new(create(&p), Some(id.clone()));
    let back = reconcile(&mut world, &alice, &p, &effect);
    let ReadBack::Found { revision, matches } = &back else {
        panic!("found, not {back:?}");
    };
    assert!(*matches);
    assert_eq!(
        decide(
            ChangeTool::Create,
            None,
            false,
            None,
            &back,
            world.clock.now_ms()
        ),
        Verdict::Applied {
            revision: Some(revision.clone()),
            confirmed: true,
            notification: Notification::NotRequested
        }
    );
    assert_eq!(world.fake.events(p.provider_id.as_str()).len(), 1);
    assert_eq!(changes(&world).len(), 1);
    world.check_log();
}

/// g30: a 503 on a create is NoAnswer, not Transient; with an insert that becomes visible only
/// later, the first reconcile read says read again and the second finds it.
#[test]
fn a_create_answered_503_is_no_answer_and_a_late_visible_insert_is_found_on_the_second_read() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    world.fake.arm(
        Fault::on(
            "POST",
            "/calendar/v3/calendars/",
            FaultAction::apply_then_503(),
        )
        .and(FaultAction::HideInsertForReads(1)),
    );
    let id = reserved(8);
    let report = run(&mut world, &alice, &p, create(&p), Some(id.clone()));
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    let effect = Effect::new(create(&p), Some(id));
    let first = reconcile(&mut world, &alice, &p, &effect);
    assert_eq!(first, ReadBack::NotFound);
    let t = world.clock.now_ms();
    assert_eq!(
        decide(ChangeTool::Create, None, false, None, &first, t),
        Verdict::ReadAgain {
            not_before_ms: t + 60_000
        }
    );
    world.clock.advance(60_000);
    let second = reconcile(&mut world, &alice, &p, &effect);
    assert!(matches!(second, ReadBack::Found { matches: true, .. }));
    assert!(matches!(
        decide(
            ChangeTool::Create,
            None,
            false,
            Some(t),
            &second,
            world.clock.now_ms()
        ),
        Verdict::Applied {
            confirmed: true,
            ..
        }
    ));
    assert_eq!(changes(&world).len(), 1);
    world.check_log();
}

fn meeting(world: &World, target: &CalendarTarget) -> (String, String) {
    put(
        world,
        target,
        json!({
            "id": "meeting1", "summary": "Weekly", "location": "Room 2", "description": "Notes",
            "start": {"dateTime": "2026-11-20T09:00:00+01:00", "timeZone": ZONE},
            "end": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": ZONE},
            "colorId": "5", "transparency": "opaque",
            "attendees": [{"email": "ann@example.com", "responseStatus": "accepted"}],
        }),
    )
}

/// g31: PATCH with `If-Match`, only the set members, `sendUpdates` as approved; untouched
/// members are unchanged at the fake.
#[test]
fn an_update_patches_only_the_set_members_under_if_match() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.title = Some(EventTitle::new("Weekly sync").expect("valid"));
    c.start = Some(at("2026-11-20T09:30:00+01:00", ZONE));
    c.location = Some(EventLocation::new("").expect("valid"));
    let report = run(
        &mut world,
        &alice,
        &p,
        update(&p, event_ref(&id, &etag), c, SendUpdates::ExternalOnly),
        None,
    );
    let ChangeOutcome::Applied {
        event,
        confirmed,
        differs,
        notification,
    } = &report.outcome
    else {
        panic!("applied, not {report:?}");
    };
    assert!(*confirmed);
    assert_eq!(*differs, None);
    assert_eq!(*notification, Notification::Sent);
    let event = event.as_ref().expect("an event");
    assert_eq!(event.event_id.as_str(), id);
    assert_ne!(event.revision.as_str(), etag);
    let sent = changes(&world);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].segments[5], id);
    assert_eq!(sent[0].if_match.as_deref(), Some(etag.as_str()));
    assert_eq!(sent[0].query_value("sendUpdates"), Some("externalOnly"));
    assert_eq!(
        sent[0].body_json(),
        Some(json!({"summary": "Weekly sync", "location": "",
            "start": {"dateTime": "2026-11-20T09:30:00+01:00", "timeZone": ZONE, "date": null}}))
    );
    let after = stored(&world, &p, &id);
    assert_eq!(after["description"], json!("Notes"));
    assert_eq!(after["colorId"], json!("5"));
    assert_eq!(after["end"]["dateTime"], json!("2026-11-20T10:00:00+01:00"));
    assert_eq!(
        after["attendees"],
        json!([{"email": "ann@example.com", "responseStatus": "accepted"}])
    );
    // `sendUpdates=none` is written and reported as not requested.
    let etag = after["etag"].as_str().expect("etag").to_owned();
    let mut c = no_changes();
    c.description =
        Some(fabric_desktop_contracts::scalar::EventDescription::new("New").expect("valid"));
    let report = run(
        &mut world,
        &alice,
        &p,
        update(&p, event_ref(&id, &etag), c, SendUpdates::None),
        None,
    );
    assert!(matches!(
        report.outcome,
        ChangeOutcome::Applied {
            notification: Notification::NotRequested,
            confirmed: true,
            ..
        }
    ));
    assert_eq!(changes(&world)[1].query_value("sendUpdates"), Some("none"));
    world.check_log();
}

/// g32: against a changed event 412 is `revision_conflict` and the fake's event keeps the other
/// person's change only; 404 and 410 are `revision_conflict`; 409 is Transient.
#[test]
fn an_update_of_a_changed_or_gone_event_is_a_revision_conflict_and_409_is_transient() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.title = Some(EventTitle::new("Mine").expect("valid"));
    let action = update(&p, event_ref(&id, &etag), c, SendUpdates::All);
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::change_event(p.provider_id.as_str(), &id, json!({"summary": "Theirs"})),
    ));
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(
        rejected(&report),
        (FailureCode::RevisionConflict, Sentence::EventChanged.text())
    );
    assert_eq!(stored(&world, &p, &id)["summary"], json!("Theirs"));
    assert_eq!(world.fake.log().last().expect("one").answered, Some(412));

    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::error(409, "conflict"),
    ));
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(
        report.outcome,
        ChangeOutcome::Transient {
            retry_after_ms: None
        }
    );

    world
        .fake
        .with_state(|s| s.cancel_event(p.provider_id.as_str(), &id));
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(
        rejected(&report),
        (FailureCode::RevisionConflict, Sentence::EventGone.text())
    );
    assert_eq!(world.fake.log().last().expect("one").answered, Some(410));
    world
        .fake
        .with_state(|s| s.remove_event(p.provider_id.as_str(), &id));
    let report = run(&mut world, &alice, &p, action, None);
    assert_eq!(
        rejected(&report),
        (FailureCode::RevisionConflict, Sentence::EventGone.text())
    );
    assert_eq!(world.fake.log().last().expect("one").answered, Some(404));
    world.check_log();
}

/// g33: a 503 after the fake applied the update: NoAnswer; reconciliation says Applied with
/// `notification: Unknown`; one change at the fake.
#[test]
fn an_update_answered_503_after_it_was_applied_reconciles_as_applied_with_unknown_notification() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.title = Some(EventTitle::new("Moved").expect("valid"));
    let action = update(&p, event_ref(&id, &etag), c, SendUpdates::All);
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::apply_then_503(),
    ));
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    let back = reconcile(&mut world, &alice, &p, &Effect::new(action.clone(), None));
    let effect = Effect::new(action, None);
    let verdict = decide(
        ChangeTool::Update,
        effect.approved_revision(),
        effect.may_notify(),
        None,
        &back,
        world.clock.now_ms(),
    );
    assert!(
        matches!(
            verdict,
            Verdict::Applied {
                notification: Notification::Unknown,
                confirmed: true,
                ..
            }
        ),
        "{verdict:?}"
    );
    assert_eq!(changes(&world).len(), 1);
    let after = stored(&world, &p, &id);
    assert_eq!(after["summary"], json!("Moved"));
    let etag_now: u64 = after["etag"]
        .as_str()
        .expect("etag")
        .trim_matches('"')
        .parse()
        .expect("a number");
    let etag_then: u64 = etag.trim_matches('"').parse().expect("a number");
    assert_eq!(etag_now, etag_then + 1, "exactly one change");
    world.check_log();
}

fn invited_event(world: &World, target: &CalendarTarget, own: &str) -> (String, String) {
    put(
        world,
        target,
        json!({
            "id": "inv1", "summary": "Review",
            "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"},
            "organizer": {"email": "carol@example.com"},
            "attendees": [
                {"email": own, "self": true, "responseStatus": "needsAction"},
                {"email": "carol@example.com", "organizer": true, "responseStatus": "accepted"},
                {"email": "Ann@Example.com", "responseStatus": "declined", "comment": "away"},
                {"email": "bob@example.com", "responseStatus": "accepted"},
                {"email": "room@resource.example.com", "resource": true, "responseStatus": "accepted"},
            ],
        }),
    )
}

/// g34: the list after the change is exactly the approved one; kept attendees keep their
/// answers; the own entry stays; a changed revision or a cut list at the read makes no PATCH.
#[test]
fn an_invitation_change_sends_exactly_the_approved_list_and_keeps_the_own_entry() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = invited_event(&world, &p, "alice@example.com");
    let action = invitation(
        &p,
        event_ref(&id, &etag),
        &["dan@example.com", "Ann@example.com"],
        SendUpdates::All,
    );
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert!(
        matches!(
            report.outcome,
            ChangeOutcome::Applied {
                confirmed: true,
                notification: Notification::Sent,
                ..
            }
        ),
        "{report:?}"
    );
    let log = world.fake.log();
    assert_eq!(log.len(), 2);
    assert_eq!(log.entries()[0].method, "GET");
    let patch = &changes(&world)[0];
    assert_eq!(patch.if_match.as_deref(), Some(etag.as_str()));
    assert_eq!(patch.query_value("sendUpdates"), Some("all"));
    assert_eq!(
        patch.body_json(),
        Some(json!({"attendees": [
            {"email": "dan@example.com"},
            {"email": "Ann@Example.com", "responseStatus": "declined", "comment": "away"},
            {"email": "alice@example.com", "self": true, "responseStatus": "needsAction"},
        ]}))
    );
    let after = stored(&world, &p, &id);
    let addresses: Vec<&str> = after["attendees"]
        .as_array()
        .expect("list")
        .iter()
        .map(|a| a["email"].as_str().expect("address"))
        .collect();
    assert_eq!(
        addresses,
        ["dan@example.com", "Ann@Example.com", "alice@example.com"]
    );
    assert_eq!(after["attendees"][1]["responseStatus"], json!("declined"));

    // A changed revision at the read: no PATCH.
    let before = changes(&world).len();
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(
        rejected(&report),
        (FailureCode::RevisionConflict, Sentence::EventChanged.text())
    );
    // A cut list at the read: no PATCH.
    let etag_now = stored(&world, &p, &id)["etag"]
        .as_str()
        .expect("etag")
        .to_owned();
    let action = invitation(
        &p,
        event_ref(&id, &etag_now),
        &["dan@example.com"],
        SendUpdates::None,
    );
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::CutAttendees,
    ));
    let report = run(&mut world, &alice, &p, action, None);
    assert_eq!(
        rejected(&report),
        (FailureCode::InvalidArguments, Sentence::AttendeesCut.text())
    );
    assert_eq!(changes(&world).len(), before, "no PATCH");
    world.check_log();
}

/// g34, adversarial lists at the fake: two entries for one address, an address whose domain
/// differs in case, the own entry that is also the organizer, an entry without `email`, and
/// no `attendees` member at all. Each PATCH body is exactly what 6.7 says; afterwards nobody
/// outside the approved list (and the own entry) is an attendee, everyone approved is one
/// exactly once, and everyone who was an attendee keeps their answer.
#[test]
fn an_invitation_change_against_adversarial_attendee_lists_keeps_exactly_the_approved_people() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let own = json!({"email": "alice@example.com", "self": true, "responseStatus": "accepted"});
    let cases: Vec<(&str, Option<Value>, Vec<&str>, Value)> = vec![
        (
            "one address twice",
            Some(json!([
                own.clone(),
                {"email": "ann@example.com", "responseStatus": "accepted"},
                {"email": "ann@example.com", "responseStatus": "declined"},
                {"email": "bob@example.com", "responseStatus": "tentative"},
            ])),
            vec!["ann@example.com", "dan@example.com"],
            json!([
                {"email": "ann@example.com", "responseStatus": "accepted"},
                {"email": "dan@example.com"},
                own.clone(),
            ]),
        ),
        (
            "the domain in another case",
            Some(json!([
                {"email": "Ann@EXAMPLE.com", "responseStatus": "declined", "comment": "away"},
                own.clone(),
            ])),
            vec!["Ann@example.com"],
            json!([
                {"email": "Ann@EXAMPLE.com", "responseStatus": "declined", "comment": "away"},
                own.clone(),
            ]),
        ),
        (
            "the own entry is the organizer",
            Some(json!([
                {"email": "alice@example.com", "self": true, "organizer": true, "responseStatus": "accepted"},
                {"email": "bob@example.com", "responseStatus": "accepted"},
            ])),
            vec!["dan@example.com", "alice@example.com"],
            json!([
                {"email": "dan@example.com"},
                {"email": "alice@example.com", "self": true, "organizer": true, "responseStatus": "accepted"},
            ]),
        ),
        (
            "an entry without an address",
            Some(json!([
                own.clone(),
                {"displayName": "Someone", "responseStatus": "accepted"},
                {"email": "bob@example.com", "responseStatus": "tentative"},
            ])),
            vec!["bob@example.com"],
            json!([
                {"email": "bob@example.com", "responseStatus": "tentative"},
                own.clone(),
            ]),
        ),
        (
            "no attendees member",
            None,
            vec!["dan@example.com"],
            json!([{"email": "dan@example.com"}]),
        ),
    ];
    for (n, (label, attendees, approved, body)) in cases.into_iter().enumerate() {
        let mut event = json!({
            "id": format!("adv{n}"), "summary": "Review",
            "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"},
            "organizer": {"email": "carol@example.com"},
        });
        if let Some(list) = &attendees {
            event["attendees"] = list.clone();
        }
        let (id, etag) = put(&world, &p, event);
        let before = changes(&world).len();
        let report = run(
            &mut world,
            &alice,
            &p,
            invitation(&p, event_ref(&id, &etag), &approved, SendUpdates::All),
            None,
        );
        assert!(
            matches!(
                report.outcome,
                ChangeOutcome::Applied {
                    confirmed: true,
                    ..
                }
            ),
            "{label}: {report:?}"
        );
        let sent = changes(&world);
        assert_eq!(sent.len(), before + 1, "{label}");
        assert_eq!(
            sent[before].body_json(),
            Some(json!({ "attendees": body })),
            "{label}"
        );
        // Afterwards: the approved people once each (the own entry counts as Alice), nobody
        // else, and every earlier attendee who stays keeps the answer they had.
        let after = stored(&world, &p, &id)["attendees"].clone();
        let after = after.as_array().cloned().unwrap_or_default();
        let key = |address: &str| {
            let (local, domain) = address.split_once('@').expect("an address");
            format!("{local}@{}", domain.to_ascii_lowercase())
        };
        let mut people: Vec<String> = after
            .iter()
            .map(|a| key(a["email"].as_str().expect("every entry has an address")))
            .collect();
        people.sort();
        let mut want: Vec<String> = approved.iter().map(|a| key(a)).collect();
        if attendees.is_some() && !want.contains(&key("alice@example.com")) {
            want.push(key("alice@example.com"));
        }
        want.sort();
        assert_eq!(people, want, "{label}");
        for entry in &after {
            let address = key(entry["email"].as_str().expect("an address"));
            let earlier = attendees
                .iter()
                .flat_map(|l| l.as_array().cloned().unwrap_or_default())
                .find(|a| a["email"].as_str().is_some_and(|e| key(e) == address));
            if let Some(earlier) = earlier {
                assert_eq!(
                    entry["responseStatus"], earlier["responseStatus"],
                    "{label}"
                );
            }
        }
    }
    world.check_log();
}

/// g35: the whole list is sent and only the own entry differs; an occurrence is addressed by
/// its instance id and its original start is checked; without an own entry, and on a shared
/// calendar, Rejected and no PATCH.
#[test]
fn a_response_sends_the_whole_list_with_only_the_own_answer_changed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = invited_event(&world, &p, "alice@example.com");
    let read_list = stored(&world, &p, &id)["attendees"].clone();
    let report = run(
        &mut world,
        &alice,
        &p,
        response(
            &p,
            event_ref(&id, &etag),
            ProposedResponse::Accepted,
            SendUpdates::All,
        ),
        None,
    );
    assert!(
        matches!(
            report.outcome,
            ChangeOutcome::Applied {
                confirmed: true,
                notification: Notification::Sent,
                ..
            }
        ),
        "{report:?}"
    );
    let patch = &changes(&world)[0];
    assert_eq!(patch.if_match.as_deref(), Some(etag.as_str()));
    let mut expected = read_list.clone();
    expected[0]["responseStatus"] = json!("accepted");
    assert_eq!(patch.body_json(), Some(json!({"attendees": expected})));
    assert_eq!(stored(&world, &p, &id)["attendees"], expected);

    // One occurrence: the instance id in the path, its original start checked.
    put(
        &world,
        &p,
        json!({
            "id": "series9_20261127T080000Z", "recurringEventId": "series9",
            "originalStartTime": {"dateTime": "2026-11-27T09:00:00+01:00"},
            "start": {"dateTime": "2026-11-27T09:00:00+01:00"}, "end": {"dateTime": "2026-11-27T10:00:00+01:00"},
            "organizer": {"email": "carol@example.com"},
            "attendees": [{"email": "alice@example.com", "self": true, "responseStatus": "needsAction"}, {"email": "carol@example.com"}],
        }),
    );
    let occ_etag = world
        .fake
        .with_state(|s| s.etag(p.provider_id.as_str(), "series9_20261127T080000Z"))
        .expect("etag");
    let before = changes(&world).len();
    let wrong = occurrence(
        "series9_20261127T080000Z",
        at("2026-11-27T10:00:00+01:00", ZONE),
        &occ_etag,
    );
    let report = run(
        &mut world,
        &alice,
        &p,
        response_to(&p, wrong, ProposedResponse::Declined, SendUpdates::None),
        None,
    );
    assert_eq!(
        rejected(&report),
        (FailureCode::RevisionConflict, Sentence::EventChanged.text())
    );
    assert_eq!(changes(&world).len(), before);
    // The same instant written with another offset is the approved instant.
    let right = occurrence(
        "series9_20261127T080000Z",
        at("2026-11-27T08:00:00+00:00", ZONE),
        &occ_etag,
    );
    let report = run(
        &mut world,
        &alice,
        &p,
        response_to(&p, right, ProposedResponse::Declined, SendUpdates::None),
        None,
    );
    assert!(
        matches!(
            report.outcome,
            ChangeOutcome::Applied {
                notification: Notification::NotRequested,
                ..
            }
        ),
        "{report:?}"
    );
    let patch = changes(&world).last().cloned().expect("a PATCH");
    assert_eq!(patch.segments[5], "series9_20261127T080000Z");

    // No own entry: an event where Alice is not invited.
    let (lone, lone_etag) = put(&world, &p, timed("lonely", "Alone", 12, 13));
    let before = changes(&world).len();
    let report = run(
        &mut world,
        &alice,
        &p,
        response(
            &p,
            event_ref(&lone, &lone_etag),
            ProposedResponse::Accepted,
            SendUpdates::All,
        ),
        None,
    );
    assert_eq!(
        rejected(&report),
        (FailureCode::ProviderRejected, Sentence::NotInvited.text())
    );
    // A shared calendar: the entry marked `self` is Bob, and his invitation is never answered.
    let shared = world.calendar(
        &alice,
        CalendarEntry::new("bob@example.com", "writer").time_zone(ZONE),
    );
    let (sid, setag) = invited_event(&world, &shared, "bob@example.com");
    let report = run(
        &mut world,
        &alice,
        &shared,
        response(
            &shared,
            event_ref(&sid, &setag),
            ProposedResponse::Accepted,
            SendUpdates::All,
        ),
        None,
    );
    assert_eq!(
        rejected(&report),
        (FailureCode::ProviderRejected, Sentence::NotInvited.text())
    );
    assert_eq!(changes(&world).len(), before, "no PATCH");
    world.check_log();
}

/// g36: `If-Match` and `sendUpdates` as approved; a 204 is followed by one read of the event,
/// which confirms the deletion (18.7 point 1); 412, 404 and 410 are `revision_conflict`, with
/// no read after them.
#[test]
fn a_delete_carries_if_match_and_send_updates_and_a_changed_or_gone_event_is_a_conflict() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    let report = run(
        &mut world,
        &alice,
        &p,
        delete(&p, event_ref(&id, &etag), SendUpdates::All),
        None,
    );
    assert_eq!(
        report.outcome,
        ChangeOutcome::Applied {
            event: None,
            confirmed: true,
            differs: None,
            notification: Notification::Sent
        }
    );
    let sent = changes(&world);
    assert_eq!(sent[0].method, "DELETE");
    assert_eq!(sent[0].if_match.as_deref(), Some(etag.as_str()));
    assert_eq!(sent[0].query_value("sendUpdates"), Some("all"));
    assert_eq!(stored(&world, &p, &id)["status"], json!("cancelled"));
    // Gone: 410 on the cancelled event, 404 once it is removed.
    let report = run(
        &mut world,
        &alice,
        &p,
        delete(&p, event_ref(&id, &etag), SendUpdates::None),
        None,
    );
    assert_eq!(
        rejected(&report),
        (
            FailureCode::RevisionConflict,
            Sentence::EventAlreadyGone.text()
        )
    );
    world
        .fake
        .with_state(|s| s.remove_event(p.provider_id.as_str(), &id));
    let report = run(
        &mut world,
        &alice,
        &p,
        delete(&p, event_ref(&id, &etag), SendUpdates::None),
        None,
    );
    assert_eq!(
        rejected(&report),
        (
            FailureCode::RevisionConflict,
            Sentence::EventAlreadyGone.text()
        )
    );
    // Changed: 412.
    let (id2, etag2) = put(&world, &p, timed("other1", "Other", 9, 10));
    world
        .fake
        .change_event(p.provider_id.as_str(), &id2, json!({"summary": "Changed"}));
    let report = run(
        &mut world,
        &alice,
        &p,
        delete(&p, event_ref(&id2, &etag2), SendUpdates::ExternalOnly),
        None,
    );
    assert_eq!(
        rejected(&report),
        (FailureCode::RevisionConflict, Sentence::EventChanged.text())
    );
    let answers: Vec<Option<u16>> = world
        .fake
        .log()
        .entries()
        .iter()
        .map(|e| e.answered)
        .collect();
    assert_eq!(
        answers,
        [Some(204), Some(200), Some(410), Some(404), Some(412)]
    );
    assert_eq!(world.fake.log().entries()[1].method, "GET");
    assert_eq!(stored(&world, &p, &id2)["status"], json!("confirmed"));
    world.check_log();
}

/// The orders above are refused before any request when they name another account or
/// calendar than their target, or a tool that changes nothing.
#[test]
fn an_order_that_does_not_fit_its_target_makes_no_request() {
    let (mut world, alice) = World::with_alice();
    let bob = world.person("bob");
    let p = alice.primary.clone();
    let before = world.fake.log().len();
    let report = run(
        &mut world,
        &alice,
        &bob.primary,
        create(&p),
        Some(reserved(1)),
    );
    assert_eq!(
        rejected(&report),
        (
            FailureCode::InvalidArguments,
            Sentence::OrderMismatch.text()
        )
    );
    let report = run(&mut world, &bob, &p, create(&p), Some(reserved(1)));
    assert_eq!(
        rejected(&report),
        (
            FailureCode::InvalidArguments,
            Sentence::OrderMismatch.text()
        )
    );
    let list = Action::CalendarList(fabric_desktop_contracts::agent::CalendarListArguments {});
    let report = run(&mut world, &alice, &p, list, None);
    assert_eq!(
        rejected(&report),
        (
            FailureCode::InvalidArguments,
            Sentence::OrderMismatch.text()
        )
    );
    assert_eq!(world.fake.log().len(), before);
    world.check_log();
}

fn read_rejected(sentence: Sentence) -> ReadFailure {
    ReadFailure::rejected(FailureCode::InvalidArguments, sentence)
}

/// The check of an order's fit is one check, made by a change and by its reconcile read: an
/// effect that names another calendar or account than its target, or a target of another
/// account, makes no request. A delete read on the wrong calendar would read 404 and be judged
/// applied.
#[test]
fn a_reconcile_read_whose_effect_does_not_fit_its_target_makes_no_request() {
    let (mut world, alice) = World::with_alice();
    let bob = world.person("bob");
    let p = alice.primary.clone();
    let team = world.calendar(
        &alice,
        CalendarEntry::new("team@group.example.com", "owner").time_zone(ZONE),
    );
    let (id, etag) = meeting(&world, &p);
    let deadline = world.deadline();
    let before = world.fake.log().len();
    let delete_on = |t: &CalendarTarget| delete(t, event_ref(&id, &etag), SendUpdates::All);
    let list = Action::CalendarList(fabric_desktop_contracts::agent::CalendarListArguments {});
    for (target, action) in [
        (&team, delete_on(&p)),
        (&p, delete_on(&team)),
        (&p, delete_on(&bob.primary)),
        (&bob.primary, delete_on(&bob.primary)),
        (&p, list),
    ] {
        let effect = Effect::new(action.clone(), None);
        let back = world.as_account(&alice, |g| {
            g.reconcile_read(&alice.id, target, &effect, deadline)
        });
        assert_eq!(back, Err(read_rejected(Sentence::OrderMismatch)));
        let report = run(&mut world, &alice, target, action, None);
        assert_eq!(
            rejected(&report),
            (
                FailureCode::InvalidArguments,
                Sentence::OrderMismatch.text()
            )
        );
    }
    assert_eq!(world.fake.log().len(), before, "no request");
    world.check_log();
}

/// An event read that answers with another event than the one asked for is no usable answer:
/// Transient for an observation, a reconcile read and the read before an invitation or a
/// response; no PATCH is sent.
#[test]
fn an_event_read_answered_with_another_event_is_no_usable_answer() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = invited_event(&world, &p, "alice@example.com");
    let mut other = stored(&world, &p, &id);
    other["id"] = json!("anotherevent");
    let arm = |world: &World| {
        world.fake.arm(Fault::on(
            "GET",
            "/calendar/v3/calendars/",
            FaultAction::Json(200, other.clone()),
        ));
    };
    let deadline = world.deadline();
    arm(&world);
    let observed = world.as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, deadline));
    assert_eq!(observed, Err(ReadFailure::transient()));
    let answer = response(
        &p,
        event_ref(&id, &etag),
        ProposedResponse::Accepted,
        SendUpdates::All,
    );
    arm(&world);
    let effect = Effect::new(answer.clone(), None);
    let back = world.as_account(&alice, |g| {
        g.reconcile_read(&alice.id, &p, &effect, deadline)
    });
    assert_eq!(back, Err(ReadFailure::transient()));
    let invite = invitation(
        &p,
        event_ref(&id, &etag),
        &["dan@example.com"],
        SendUpdates::All,
    );
    for action in [invite, answer] {
        arm(&world);
        let report = run(&mut world, &alice, &p, action, None);
        assert_eq!(report.outcome, ChangeOutcome::transient());
    }
    assert!(changes(&world).is_empty(), "no PATCH");
    world.check_log();
}

/// An event that Google gives as well-formed JSON but not as a usable event (a member of the
/// wrong type, an unknown answer, start and end of different kinds) is Rejected with the fixed
/// sentence for an observation, a reconcile read and the read before a change; no PATCH is
/// sent. Reading it again would give the same, so it is not Transient.
#[test]
fn an_unusable_event_is_rejected_by_every_event_read_and_no_patch_is_sent() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = invited_event(&world, &p, "alice@example.com");
    let good = stored(&world, &p, &id);
    let mut wrong_type = good.clone();
    wrong_type["summary"] = json!(5);
    let mut unknown_answer = good.clone();
    unknown_answer["attendees"][1]["responseStatus"] = json!("maybe");
    let mut mixed = good;
    mixed["start"] = json!({"date": "2026-11-20"});
    let deadline = world.deadline();
    let answer = response(
        &p,
        event_ref(&id, &etag),
        ProposedResponse::Accepted,
        SendUpdates::All,
    );
    let invite = invitation(
        &p,
        event_ref(&id, &etag),
        &["dan@example.com"],
        SendUpdates::All,
    );
    for body in [wrong_type, unknown_answer, mixed] {
        let arm = |world: &World| {
            world.fake.arm(Fault::on(
                "GET",
                "/calendar/v3/calendars/",
                FaultAction::Json(200, body.clone()),
            ));
        };
        arm(&world);
        let observed = world.as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, deadline));
        assert_eq!(observed, Err(read_rejected(Sentence::EventUnreadable)));
        arm(&world);
        let effect = Effect::new(answer.clone(), None);
        let back = world.as_account(&alice, |g| {
            g.reconcile_read(&alice.id, &p, &effect, deadline)
        });
        assert_eq!(back, Err(read_rejected(Sentence::EventUnreadable)));
        for action in [invite.clone(), answer.clone()] {
            arm(&world);
            let report = run(&mut world, &alice, &p, action, None);
            assert_eq!(
                rejected(&report),
                (
                    FailureCode::InvalidArguments,
                    Sentence::EventUnreadable.text()
                )
            );
        }
    }
    assert!(changes(&world).is_empty(), "no PATCH");
    world.check_log();
}

/// Local parts are compared exactly (the plan forbids inventing equivalences): two approved
/// addresses that differ in the local part's case are two people, and the order goes through.
/// Two that differ only in the domain's case cannot be approved: the contract's `email` holds
/// a lower-case domain (the refusal itself is a unit test of `translate`).
#[test]
fn an_invitation_of_addresses_that_differ_in_the_local_parts_case_names_two_people() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = invited_event(&world, &p, "alice@example.com");
    let report = run(
        &mut world,
        &alice,
        &p,
        invitation(
            &p,
            event_ref(&id, &etag),
            &["ann@example.com", "Ann@example.com"],
            SendUpdates::All,
        ),
        None,
    );
    assert!(
        matches!(report.outcome, ChangeOutcome::Applied { .. }),
        "{report:?}"
    );
    let sent = changes(&world)[0].body_json().expect("a body");
    let addresses: Vec<&str> = sent["attendees"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|a| a["email"].as_str().expect("an address"))
        .collect();
    assert_eq!(
        addresses,
        ["ann@example.com", "Ann@Example.com", "alice@example.com"]
    );
    world.check_log();
}

/// An update that sets no member would send `PATCH {}`, could still tell everyone, and would
/// read as confirmed: it is refused before any request.
#[test]
fn an_update_that_changes_nothing_is_refused_before_any_request() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    for send in [SendUpdates::All, SendUpdates::None] {
        let report = run(
            &mut world,
            &alice,
            &p,
            update(&p, event_ref(&id, &etag), no_changes(), send),
            None,
        );
        assert_eq!(
            rejected(&report),
            (
                FailureCode::InvalidArguments,
                Sentence::NothingToChange.text()
            )
        );
    }
    assert!(world.fake.log().is_empty(), "no request");
    world.check_log();
}

/// A time in the order that the adapter cannot read is refused with a sentence of its own,
/// never with one that says Google refused: nothing was sent (PD27).
#[test]
fn an_unreadable_original_start_is_refused_with_its_own_sentence_and_no_request() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    // 30 February has the contract's form but is no day.
    let to = occurrence(
        "series9_20260301T080000Z",
        at("2026-02-30T09:00:00+01:00", ZONE),
        "\"1\"",
    );
    let action = response_to(&p, to, ProposedResponse::Accepted, SendUpdates::All);
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(
        rejected(&report),
        (
            FailureCode::InvalidArguments,
            Sentence::TimeUnreadable.text()
        )
    );
    let deadline = world.deadline();
    let effect = Effect::new(action, None);
    let back = world.as_account(&alice, |g| {
        g.reconcile_read(&alice.id, &p, &effect, deadline)
    });
    assert_eq!(back, Err(read_rejected(Sentence::TimeUnreadable)));
    assert!(world.fake.log().is_empty(), "no request");
    assert!(!Sentence::TimeUnreadable.text().starts_with("Google"));
    world.check_log();
}

/// CD-Q05b binds instance id, original start and revision: a reconcile read of an answer to
/// one occurrence does not match when the instance's original start is not the approved one,
/// even if the own entry has the approved answer.
#[test]
fn a_reconcile_read_of_an_occurrence_answer_checks_the_original_start() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let instance = "series9_20261127T080000Z";
    let (_, etag) = put(
        &world,
        &p,
        json!({
            "id": instance, "recurringEventId": "series9",
            "originalStartTime": {"dateTime": "2026-11-27T09:00:00+01:00"},
            "start": {"dateTime": "2026-11-27T09:00:00+01:00"}, "end": {"dateTime": "2026-11-27T10:00:00+01:00"},
            "organizer": {"email": "carol@example.com"},
            "attendees": [{"email": "alice@example.com", "self": true, "responseStatus": "accepted"}, {"email": "carol@example.com"}],
        }),
    );
    let deadline = world.deadline();
    let read = |world: &mut World, original: &str| {
        let action = response_to(
            &p,
            occurrence(instance, at(original, ZONE), &etag),
            ProposedResponse::Accepted,
            SendUpdates::None,
        );
        let effect = Effect::new(action, None);
        world
            .as_account(&alice, |g| {
                g.reconcile_read(&alice.id, &p, &effect, deadline)
            })
            .expect("a read-back")
    };
    assert!(matches!(
        read(&mut world, "2026-11-27T08:00:00+00:00"),
        ReadBack::Found { matches: true, .. }
    ));
    assert!(matches!(
        read(&mut world, "2026-11-27T10:00:00+01:00"),
        ReadBack::Found { matches: false, .. }
    ));
    world.check_log();
}

/// One event with many attendees can be larger than 256 KiB: the read before a change and the
/// answer to the PATCH use the large bound, so the answer is read and the change Applied.
#[test]
fn a_change_of_an_event_larger_than_256_kib_is_read_and_applied() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let local = "x".repeat(60);
    let mut attendees =
        vec![json!({"email": "alice@example.com", "self": true, "responseStatus": "needsAction"})];
    attendees.extend((0..1_999).map(|n| {
        json!({"email": format!("{local}{n:04}@example.com"), "responseStatus": "needsAction", "comment": "y".repeat(40)})
    }));
    let (id, etag) = put(
        &world,
        &p,
        json!({
            "id": "crowd", "summary": "All hands",
            "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"},
            "organizer": {"email": "carol@example.com"},
            "attendees": attendees,
        }),
    );
    assert!(
        serde_json::to_vec(&stored(&world, &p, &id))
            .expect("json")
            .len()
            > 256 * 1024
    );
    let report = run(
        &mut world,
        &alice,
        &p,
        response(
            &p,
            event_ref(&id, &etag),
            ProposedResponse::Accepted,
            SendUpdates::None,
        ),
        None,
    );
    assert!(
        matches!(
            report.outcome,
            ChangeOutcome::Applied {
                confirmed: true,
                ..
            }
        ),
        "{report:?}"
    );
    assert_eq!(changes(&world).len(), 1);
    world.check_log();
}

/// The requests a delete order made, with their methods, and its report.
fn delete_once(
    world: &mut World,
    who: &Person,
    target: &CalendarTarget,
    id: &str,
    etag: &str,
) -> (ChangeReport, Vec<LoggedRequest>) {
    let start = world.fake.log().len();
    let report = run(
        world,
        who,
        target,
        delete(target, event_ref(id, etag), SendUpdates::None),
        None,
    );
    (report, world.log_since(start))
}

fn applied_delete(confirmed: bool) -> ChangeOutcome {
    ChangeOutcome::Applied {
        event: None,
        confirmed,
        differs: None,
        notification: Notification::NotRequested,
    }
}

/// The delete order's requests are one DELETE and at most one GET after it, of the same
/// event, with the same token.
fn assert_one_delete_then_at_most_one_get(requests: &[LoggedRequest], gets: usize) {
    assert_eq!(requests.len(), 1 + gets, "{requests:?}");
    assert_eq!(requests[0].method, "DELETE");
    for read in &requests[1..] {
        assert_eq!(read.method, "GET");
        assert_eq!(read.segments, requests[0].segments, "the same event");
        assert_eq!(read.token_account, requests[0].token_account);
        assert!(read.authorization);
        assert_eq!(read.query_value("timeZone"), Some(ZONE));
    }
}

/// 18.7 point 1: a delete answered 204 is confirmed by one read of the event in the same
/// order, whichever of the three answers the fake gives for a deleted event (FA3): 200 with
/// `status: cancelled`, 404 or 410. The read uses the DELETE's token: no second token is
/// asked for.
#[test]
fn a_delete_is_confirmed_by_one_read_for_each_answer_to_a_deleted_event() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    for (n, mode) in [
        DeletedEventGet::Cancelled,
        DeletedEventGet::NotFound,
        DeletedEventGet::Gone,
    ]
    .into_iter()
    .enumerate()
    {
        world.fake.with_state(|s| s.set_deleted_event_get(mode));
        let (id, etag) = put(&world, &p, timed(&format!("gone{n}"), "Gone", 9, 10));
        let calls = world.tokens.calls();
        let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
        assert_eq!(report.outcome, applied_delete(true), "{mode:?}");
        assert_eq!(report.side, None);
        assert_one_delete_then_at_most_one_get(&requests, 1);
        assert_eq!(
            requests[1].answered,
            Some(match mode {
                DeletedEventGet::Cancelled => 200,
                DeletedEventGet::NotFound => 404,
                DeletedEventGet::Gone => 410,
            })
        );
        assert_eq!(
            world.tokens.calls(),
            calls + 1,
            "one token for both requests"
        );
    }
    world.check_log();
}

/// 18.7 point 1: a fault on the read after the delete (a 500, a 403, a 401, a body that is
/// not JSON, a closed connection, a 200 with another event's id even if cancelled) leaves
/// the order Applied, with `confirmed: false`; the read is never repeated and the outcome is
/// never Transient, Rejected or NoAnswer because of it. Each order asks for exactly one token,
/// the DELETE's: the read refreshes nothing and asks for no other. A 401 on the read drops no
/// cached token, and the adapter reaches no tokens but through that one call.
#[test]
fn a_fault_on_the_read_after_a_delete_leaves_it_applied_and_not_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let faults = [
        FaultAction::error(500, "backendError"),
        FaultAction::error(403, "forbidden"),
        FaultAction::error(403, "rateLimitExceeded"),
        FaultAction::error(429, "rateLimitExceeded"),
        FaultAction::error(401, "authError"),
        FaultAction::NotJson,
        FaultAction::CloseWithoutApplying,
        FaultAction::Redirect("http://127.0.0.1:9/x".into()),
        FaultAction::Json(200, json!({"id": "anotherevent", "status": "cancelled"})),
        FaultAction::Json(200, json!({"status": "cancelled"})),
        FaultAction::Json(200, json!("cancelled")),
        FaultAction::Json(204, json!({})),
    ];
    for (n, fault) in faults.into_iter().enumerate() {
        let (id, etag) = put(&world, &p, timed(&format!("ev{n}"), "Event", 9, 10));
        world
            .fake
            .arm(Fault::on("GET", "/calendar/v3/calendars/", fault.clone()));
        let calls = world.tokens.calls();
        let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
        assert_eq!(report.outcome, applied_delete(false), "{fault:?}");
        assert_eq!(report.side, None, "{fault:?}");
        assert_one_delete_then_at_most_one_get(&requests, 1);
        assert_eq!(stored(&world, &p, &id)["status"], json!("cancelled"));
        assert_eq!(
            world.tokens.calls(),
            calls + 1,
            "{fault:?}: one token per order"
        );
        assert!(world.tokens.dropped().is_empty(), "{fault:?}");
    }
    world.check_log();
}

/// 18.7 point 1: a 204 that did not delete (the fake answers it without applying), and a read
/// that finds a live event, are Applied with `confirmed: false`.
#[test]
fn a_delete_whose_event_is_still_live_at_the_read_is_applied_and_not_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = put(&world, &p, timed("still", "Still here", 9, 10));
    world.fake.arm(Fault::on(
        "DELETE",
        "/calendar/v3/calendars/",
        FaultAction::Raw(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec()),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 1);
    assert_eq!(stored(&world, &p, &id)["status"], json!("confirmed"));

    // The DELETE applied, the read answered with a live event.
    let live = stored(&world, &p, &id);
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::Json(200, live),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 1);
    world.check_log();
}

/// 18.7 point 1: with no time left of the order's deadline after the DELETE, no read is sent
/// and the order is Applied with `confirmed: false`.
#[test]
fn a_delete_with_no_time_left_sends_no_read_and_is_not_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = put(&world, &p, timed("late", "Late", 9, 10));
    // The DELETE is handled; meanwhile the adapter's clock passes the order's deadline.
    let clock = world.clock.clone();
    world.fake.arm(Fault::on(
        "DELETE",
        "/calendar/v3/calendars/",
        FaultAction::before(move |_| clock.advance(31_000)),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 0);
    assert_eq!(stored(&world, &p, &id)["status"], json!("cancelled"));
    world.check_log();
}

/// 18.7 point 1, the bound of the read: the DELETE is answered with 300 ms of the order's
/// deadline left, and the read's answer comes only 3 s later. The read is sent once, its answer
/// does not come inside the deadline, and the order is Applied with `confirmed: false`.
#[test]
fn a_read_after_a_delete_whose_answer_comes_after_the_deadline_is_not_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    world
        .fake
        .with_state(|s| s.set_deleted_event_get(DeletedEventGet::Cancelled));
    let (id, etag) = put(&world, &p, timed("slow", "Slow", 9, 10));
    let clock = world.clock.clone();
    world.fake.arm(Fault::on(
        "DELETE",
        "/calendar/v3/calendars/",
        FaultAction::before(move |_| clock.advance(30_000 - 300)),
    ));
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::Delay(std::time::Duration::from_secs(3)),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 1);
    world.check_log();
}

/// 18.7 point 1, the boundary: with the clock exactly at the order's deadline when the DELETE
/// has been answered, no read is sent. (One millisecond before, the read is due:
/// `read_back_due` in the adapter's unit tests; a 1 ms request cannot be seen reliably at the
/// fake.)
#[test]
fn a_delete_answered_exactly_at_the_deadline_sends_no_read() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = put(&world, &p, timed("edge", "Edge", 9, 10));
    let clock = world.clock.clone();
    world.fake.arm(Fault::on(
        "DELETE",
        "/calendar/v3/calendars/",
        FaultAction::before(move |_| clock.advance(30_000)),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 0);
    assert_eq!(stored(&world, &p, &id)["status"], json!("cancelled"));
    world.check_log();
}

/// 18.7 point 1: a read after a delete that answers 200 with the right id and no `status`
/// member (a live event), or 200 with a body over the 4 MiB bound, is `confirmed: false`.
#[test]
fn a_read_after_a_delete_with_no_status_or_an_oversized_body_is_not_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    world
        .fake
        .with_state(|s| s.set_deleted_event_get(DeletedEventGet::Cancelled));
    let (id, etag) = put(&world, &p, timed("nostatus", "No status", 9, 10));
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::Json(
            200,
            json!({"id": id, "etag": "\"9\"", "summary": "No status"}),
        ),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 1);

    // The real answer (200, cancelled), padded past the bound of the read.
    let (id, etag) = put(&world, &p, timed("huge", "Huge", 9, 10));
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::TooLong(4 * 1024 * 1024 + 1),
    ));
    let (report, requests) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(false));
    assert_one_delete_then_at_most_one_get(&requests, 1);
    assert_eq!(requests[1].answered, Some(200));
    // Without the padding the same answer confirms the delete.
    let (id, etag) = put(&world, &p, timed("plain", "Plain", 9, 10));
    let (report, _) = delete_once(&mut world, &alice, &p, &id, &etag);
    assert_eq!(report.outcome, applied_delete(true));
    world.check_log();
}

/// A create of a timed event in `zone`, "Planning" with this description.
fn create_in(target: &CalendarTarget, zone: &str, description: &str) -> Action {
    let Action::ProposeCreate(mut a) = create(target) else {
        panic!("a create");
    };
    a.event.start = at("2026-11-20T14:30:00+05:30", zone);
    a.event.end = at("2026-11-20T15:30:00+05:30", zone);
    a.event.description = Some(EventDescription::new(description).expect("valid"));
    Action::ProposeCreate(a)
}

fn applied_with(report: &ChangeReport) -> (bool, Option<Differs>) {
    let ChangeOutcome::Applied {
        event,
        confirmed,
        differs,
        ..
    } = &report.outcome
    else {
        panic!("applied, not {report:?}");
    };
    assert!(event.is_some());
    assert_eq!(*confirmed, differs.is_none());
    (*confirmed, *differs)
}

/// 18.7 point 2: an update whose description ends in spaces, answered with the description
/// trimmed (and stored so at the fake), is Applied with `confirmed: false` and `differs`
/// naming the description; the same update without the fault is confirmed. A later reconcile
/// read of the rewritten event does not match.
#[test]
fn an_update_answered_with_a_trimmed_description_is_applied_and_names_the_description() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.description = Some(EventDescription::new("Agenda  ").expect("valid"));
    let action = update(&p, event_ref(&id, &etag), c.clone(), SendUpdates::None);
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::RewriteDescriptionTrimmed,
    ));
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(applied_with(&report), (false, Some(Differs::Description)));
    assert_eq!(stored(&world, &p, &id)["description"], json!("Agenda"));
    assert!(matches!(
        reconcile(&mut world, &alice, &p, &Effect::new(action, None)),
        ReadBack::Found { matches: false, .. }
    ));
    assert_eq!(changes(&world).len(), 1, "never sent again");

    // Without the fault: confirmed, nothing differs.
    let etag = stored(&world, &p, &id)["etag"]
        .as_str()
        .expect("etag")
        .to_owned();
    let action = update(&p, event_ref(&id, &etag), c, SendUpdates::None);
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(applied_with(&report), (true, None));
    assert_eq!(stored(&world, &p, &id)["description"], json!("Agenda  "));
    assert!(matches!(
        reconcile(&mut world, &alice, &p, &Effect::new(action, None)),
        ReadBack::Found { matches: true, .. }
    ));
    world.check_log();
}

/// 18.7 point 2: a create and an update whose zone the fake writes as an alias (`Asia/Calcutta`
/// for `Asia/Kolkata`) are Applied with `confirmed: false` and `differs` naming the zone; the
/// same changes without the fault are confirmed. A reconcile read of the rewritten event does
/// not match.
#[test]
fn a_create_and_an_update_answered_with_a_zone_alias_are_applied_and_name_the_zone() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let alias = || FaultAction::RewriteZoneAlias {
        from: "Asia/Kolkata".to_owned(),
        to: "Asia/Calcutta".to_owned(),
    };
    // Create.
    let action = create_in(&p, "Asia/Kolkata", "Agenda");
    world
        .fake
        .arm(Fault::on("POST", "/calendar/v3/calendars/", alias()));
    let id = reserved(31);
    let report = run(&mut world, &alice, &p, action.clone(), Some(id.clone()));
    assert_eq!(applied_with(&report), (false, Some(Differs::Zone)));
    let after = stored(&world, &p, &id);
    assert_eq!(after["start"]["timeZone"], json!("Asia/Calcutta"));
    assert_eq!(after["end"]["timeZone"], json!("Asia/Calcutta"));
    assert!(matches!(
        reconcile(&mut world, &alice, &p, &Effect::new(action, Some(id))),
        ReadBack::Found { matches: false, .. }
    ));
    let id = reserved(32);
    let report = run(
        &mut world,
        &alice,
        &p,
        create_in(&p, "Asia/Kolkata", "Agenda"),
        Some(id),
    );
    assert_eq!(applied_with(&report), (true, None));

    // Update.
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.start = Some(at("2026-11-20T14:30:00+05:30", "Asia/Kolkata"));
    c.end = Some(at("2026-11-20T15:30:00+05:30", "Asia/Kolkata"));
    let action = update(&p, event_ref(&id, &etag), c.clone(), SendUpdates::None);
    world
        .fake
        .arm(Fault::on("PATCH", "/calendar/v3/calendars/", alias()));
    let report = run(&mut world, &alice, &p, action.clone(), None);
    assert_eq!(applied_with(&report), (false, Some(Differs::Zone)));
    assert_eq!(
        stored(&world, &p, &id)["start"]["timeZone"],
        json!("Asia/Calcutta")
    );
    assert!(matches!(
        reconcile(&mut world, &alice, &p, &Effect::new(action, None)),
        ReadBack::Found { matches: false, .. }
    ));
    let etag = stored(&world, &p, &id)["etag"]
        .as_str()
        .expect("etag")
        .to_owned();
    let report = run(
        &mut world,
        &alice,
        &p,
        update(&p, event_ref(&id, &etag), c, SendUpdates::None),
        None,
    );
    assert_eq!(applied_with(&report), (true, None));
    world.check_log();
}

/// 18.7 point 2: a 200 whose event has the right id is Applied for a create, an update, an
/// invitation and a response whatever it holds, with `differs` naming the first member that
/// is not what was approved; it is never NoAnswer and the change is never sent again.
#[test]
fn a_yes_with_the_right_id_and_another_content_is_applied_for_every_tool() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let answer_patch = |world: &World, body: Value| {
        world.fake.arm(Fault::on(
            "PATCH",
            "/calendar/v3/calendars/",
            FaultAction::Json(200, body),
        ));
    };
    // Create: the reserved id, another title.
    let id = reserved(41);
    world.fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars/",
        FaultAction::Json(
            200,
            json!({"id": id, "etag": "\"77\"", "summary": "Other",
            "start": {"dateTime": "2026-11-20T10:00:00+01:00"}, "end": {"dateTime": "2026-11-20T11:00:00+01:00"}}),
        ),
    ));
    let report = run(&mut world, &alice, &p, create(&p), Some(id));
    assert_eq!(applied_with(&report), (false, Some(Differs::Title)));

    // Update: the target's id, another place.
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.location = Some(EventLocation::new("Hall").expect("valid"));
    let mut body = stored(&world, &p, &id);
    body["etag"] = json!("\"78\"");
    answer_patch(&world, body);
    let report = run(
        &mut world,
        &alice,
        &p,
        update(&p, event_ref(&id, &etag), c, SendUpdates::None),
        None,
    );
    assert_eq!(applied_with(&report), (false, Some(Differs::Location)));

    // Invitation: the list as it was.
    let (id, etag) = invited_event(&world, &p, "alice@example.com");
    let mut body = stored(&world, &p, &id);
    body["etag"] = json!("\"79\"");
    answer_patch(&world, body.clone());
    let report = run(
        &mut world,
        &alice,
        &p,
        invitation(
            &p,
            event_ref(&id, &etag),
            &["dan@example.com"],
            SendUpdates::None,
        ),
        None,
    );
    assert_eq!(applied_with(&report), (false, Some(Differs::Attendees)));

    // Response: the own answer unchanged, then another attendee's entry changed.
    answer_patch(&world, body.clone());
    let answer = response(
        &p,
        event_ref(&id, &etag),
        ProposedResponse::Accepted,
        SendUpdates::None,
    );
    let report = run(&mut world, &alice, &p, answer.clone(), None);
    assert_eq!(applied_with(&report), (false, Some(Differs::OwnResponse)));
    let mut other = body;
    other["attendees"][0]["responseStatus"] = json!("accepted");
    other["attendees"][3]["responseStatus"] = json!("declined");
    answer_patch(&world, other);
    let report = run(&mut world, &alice, &p, answer, None);
    assert_eq!(applied_with(&report), (false, Some(Differs::OtherAttendee)));
    assert_eq!(changes(&world).len(), 5, "one changing request per order");

    // A 200 with the right id is Applied whatever else the body holds: a cancelled event
    // names the status; a body or a revision that cannot be read names the event, which is
    // given only when its revision fits the contract.
    let odd = |id: &str| -> Vec<(Value, Option<&'static str>, Differs)> {
        vec![
            (
                json!({"id": id, "etag": "\"90\"", "status": "cancelled"}),
                Some("\"90\""),
                Differs::Status,
            ),
            (
                json!({"id": id, "etag": "\"91\"", "summary": 5}),
                Some("\"91\""),
                Differs::Event,
            ),
            (
                json!({"id": id, "etag": "\"92\"", "recurrence": "x"}),
                Some("\"92\""),
                Differs::Event,
            ),
            (
                json!({"id": id, "etag": "\"93\"", "attendees": [5]}),
                Some("\"93\""),
                Differs::Event,
            ),
            (json!({"id": id, "summary": "Weekly"}), None, Differs::Event),
            (
                json!({"id": id, "etag": "two words", "summary": "Weekly"}),
                None,
                Differs::Event,
            ),
            (json!({"id": id, "etag": 7}), None, Differs::Event),
        ]
    };
    let (id, etag) = meeting(&world, &p);
    let mut sent = changes(&world).len();
    for (n, (body, revision, member)) in (50..).zip(odd("")) {
        // Create: the reserved id.
        let reserved_id = reserved(n);
        let mut body = body;
        body["id"] = json!(reserved_id);
        world.fake.arm(Fault::on(
            "POST",
            "/calendar/v3/calendars/",
            FaultAction::Json(200, body.clone()),
        ));
        let report = run(
            &mut world,
            &alice,
            &p,
            create(&p),
            Some(reserved_id.clone()),
        );
        assert_eq!(
            applied_parts(&report),
            (
                revision.map(|r| (reserved_id.clone(), r.to_owned())),
                false,
                Some(member)
            ),
            "create {body}"
        );
        // PATCH: the target's id.
        body["id"] = json!(id);
        answer_patch(&world, body.clone());
        let mut c = no_changes();
        c.title = Some(EventTitle::new("Weekly").expect("valid"));
        let report = run(
            &mut world,
            &alice,
            &p,
            update(&p, event_ref(&id, &etag), c, SendUpdates::None),
            None,
        );
        assert_eq!(
            applied_parts(&report),
            (
                revision.map(|r| (id.clone(), r.to_owned())),
                false,
                Some(member)
            ),
            "update {body}"
        );
        sent += 2;
        assert_eq!(changes(&world).len(), sent, "never sent again: {body}");
    }

    // Controls: another id, no string id, a body that is not an object: NoAnswer.
    for body in [
        json!({"id": "someotherid", "etag": "\"94\"", "status": "cancelled"}),
        json!({"etag": "\"95\""}),
        json!({"id": 7, "etag": "\"96\""}),
        json!([{"id": id}]),
        json!(id),
    ] {
        world.fake.arm(Fault::on(
            "POST",
            "/calendar/v3/calendars/",
            FaultAction::Json(200, body.clone()),
        ));
        let report = run(&mut world, &alice, &p, create(&p), Some(reserved(70)));
        assert_eq!(report.outcome, ChangeOutcome::NoAnswer, "create {body}");
        answer_patch(&world, body.clone());
        let mut c = no_changes();
        c.title = Some(EventTitle::new("Weekly").expect("valid"));
        let report = run(
            &mut world,
            &alice,
            &p,
            update(&p, event_ref(&id, &etag), c, SendUpdates::None),
            None,
        );
        assert_eq!(report.outcome, ChangeOutcome::NoAnswer, "update {body}");
    }
    world.check_log();
}

/// The parts of an Applied outcome: the event's id and revision when given, `confirmed` and
/// `differs`.
fn applied_parts(report: &ChangeReport) -> (Option<(String, String)>, bool, Option<Differs>) {
    let ChangeOutcome::Applied {
        event,
        confirmed,
        differs,
        ..
    } = &report.outcome
    else {
        panic!("applied, not {report:?}");
    };
    (
        event.as_ref().map(|e| {
            (
                e.event_id.as_str().to_owned(),
                e.revision.as_str().to_owned(),
            )
        }),
        *confirmed,
        *differs,
    )
}

/// 18.7 point 2, end to end: Google answers a PATCH with the approved instants written with
/// another offset (`10:00:00+01:00` for an approved `09:00:00+00:00`). The instants are the
/// same, so the change is confirmed.
#[test]
fn an_update_answered_with_the_approved_instant_in_another_offset_is_confirmed() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p);
    let mut c = no_changes();
    c.start = Some(at("2026-11-20T09:00:00+00:00", ZONE));
    c.end = Some(at("2026-11-20T10:00:00+00:00", ZONE));
    let mut body = stored(&world, &p, &id);
    body["etag"] = json!("\"98\"");
    body["start"] = json!({"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": ZONE});
    body["end"] = json!({"dateTime": "2026-11-20T11:00:00+01:00", "timeZone": ZONE});
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::Json(200, body),
    ));
    let report = run(
        &mut world,
        &alice,
        &p,
        update(&p, event_ref(&id, &etag), c, SendUpdates::None),
        None,
    );
    assert_eq!(applied_with(&report), (true, None));
    assert_eq!(changes(&world).len(), 1);
    world.check_log();
}
