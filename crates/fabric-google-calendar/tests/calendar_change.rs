//! Changing events (6.6 to 6.8) and the reconciliation of an unknown outcome (section 7):
//! g28 to g36.

mod calendar_common;

use calendar_common::{
    Person, World, ZONE, at, create, delete, event_ref, invitation, no_changes, occurrence, put,
    reserved, response, response_to, timed, update,
};
use fabric_desktop_contracts::agent::{Action, ProposedResponse};
use fabric_desktop_contracts::common::{FailureCode, SendUpdates};
use fabric_desktop_contracts::scalar::{EventLocation, EventTitle};
use fabric_google_calendar::Clock;
use fabric_google_calendar::calendar::{CalendarTarget, ChangeOrder};
use fabric_google_calendar::outcome::{ChangeOutcome, ChangeReport, Notification, Sentence};
use fabric_google_calendar::reconcile::{ChangeTool, Effect, ReadBack, Verdict, decide};
use fabric_google_fake::{CalendarEntry, Fault, FaultAction, LoggedRequest};
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
        notification,
    } = &report.outcome
    else {
        panic!("applied, not {report:?}");
    };
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
        decide(ChangeTool::Create, None, 0, &back, world.clock.now_ms()),
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
        decide(ChangeTool::Create, None, 0, &first, t),
        Verdict::ReadAgain {
            not_before_ms: t + 60_000
        }
    );
    world.clock.advance(60_000);
    let second = reconcile(&mut world, &alice, &p, &effect);
    assert!(matches!(second, ReadBack::Found { matches: true, .. }));
    assert!(matches!(
        decide(ChangeTool::Create, None, 1, &second, world.clock.now_ms()),
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
        notification,
    } = &report.outcome
    else {
        panic!("applied, not {report:?}");
    };
    assert!(*confirmed);
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
        0,
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

/// g36: `If-Match` and `sendUpdates` as approved; 412, 404 and 410 are `revision_conflict`.
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
            confirmed: false,
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
    assert_eq!(answers, [Some(204), Some(410), Some(404), Some(412)]);
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
