//! Reconciliation (section 7) with the fake as the source of the read-back: g43.

mod calendar_common;

use calendar_common::{
    Person, World, ZONE, create, create_with, delete, event_ref, invitation, no_changes, put,
    reserved, response, update,
};
use fabric_desktop_contracts::agent::{Action, ProposedResponse};
use fabric_desktop_contracts::common::SendUpdates;
use fabric_desktop_contracts::scalar::EventTitle;
use fabric_google_calendar::Clock;
use fabric_google_calendar::calendar::{CalendarTarget, ChangeOrder};
use fabric_google_calendar::outcome::{ChangeOutcome, Notification};
use fabric_google_calendar::reconcile::{
    ChangeTool, Effect, NeedsOwnerReason, Verdict, decide, earliest_read_ms,
};
use fabric_google_fake::{DeletedEventGet, Fault, FaultAction, LateTrigger};
use serde_json::{Value, json};

/// Reads the effect back and decides, as the core would.
fn verdict(
    world: &mut World,
    who: &Person,
    target: &CalendarTarget,
    effect: &Effect,
    reads_before: u32,
) -> Verdict {
    let deadline = world.deadline();
    let back = world
        .as_account(who, |g| g.reconcile_read(&who.id, target, effect, deadline))
        .expect("a read-back");
    decide(
        effect.tool().expect("a change"),
        effect.approved_revision(),
        reads_before,
        &back,
        world.clock.now_ms(),
    )
}

fn invited(world: &World, target: &CalendarTarget) -> (String, String) {
    put(
        world,
        target,
        json!({
            "id": "inv1", "summary": "Review",
            "start": {"dateTime": "2026-11-20T09:00:00+01:00", "timeZone": ZONE},
            "end": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": ZONE},
            "organizer": {"email": "carol@example.com"},
            "attendees": [
                {"email": "alice@example.com", "self": true, "responseStatus": "needsAction"},
                {"email": "carol@example.com", "organizer": true, "responseStatus": "accepted"},
            ],
        }),
    )
}

/// The three changes of an existing event and what each looks like once applied at Google.
fn changes_of_an_event(
    target: &CalendarTarget,
    id: &str,
    etag: &str,
) -> Vec<(ChangeTool, Action, Value)> {
    let mut c = no_changes();
    c.title = Some(EventTitle::new("Renamed").expect("valid"));
    vec![
        (
            ChangeTool::Update,
            update(target, event_ref(id, etag), c, SendUpdates::All),
            json!({"summary": "Renamed"}),
        ),
        (
            ChangeTool::Invitation,
            invitation(
                target,
                event_ref(id, etag),
                &["carol@example.com", "dan@example.com"],
                SendUpdates::All,
            ),
            json!({"attendees": [
                {"email": "alice@example.com", "self": true, "responseStatus": "needsAction"},
                {"email": "carol@example.com", "organizer": true, "responseStatus": "accepted"},
                {"email": "dan@example.com", "responseStatus": "needsAction"},
            ]}),
        ),
        (
            ChangeTool::Response,
            response(
                target,
                event_ref(id, etag),
                ProposedResponse::Accepted,
                SendUpdates::All,
            ),
            json!({"attendees": [
                {"email": "alice@example.com", "self": true, "responseStatus": "accepted"},
                {"email": "carol@example.com", "organizer": true, "responseStatus": "accepted"},
            ]}),
        ),
    ]
}

/// g43: every row for a create.
#[test]
fn reconciling_a_create_follows_every_row_of_section_7() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let cal = p.provider_id.as_str().to_owned();

    // Found and equal: Applied, with the revision Google holds.
    let id = reserved(1);
    let effect = Effect::new(create(&p), Some(id.clone()));
    let deadline = world.deadline();
    let order = ChangeOrder::new(create(&p), Some(id.clone()), 1);
    let report = world.as_account(&alice, |g| g.change(&alice.id, &p, &order, deadline));
    assert!(matches!(report.outcome, ChangeOutcome::Applied { .. }));
    let etag = world.fake.with_state(|s| s.etag(&cal, &id)).expect("etag");
    let Verdict::Applied {
        revision,
        confirmed,
        notification,
    } = verdict(&mut world, &alice, &p, &effect, 0)
    else {
        panic!("applied");
    };
    assert_eq!(revision.expect("a revision").as_str(), etag);
    assert!(confirmed);
    assert_eq!(notification, Notification::NotRequested);

    // Found and not equal: an event under the reserved id that is not what was approved.
    let other = reserved(2);
    let order = ChangeOrder::new(
        create_with(&p, "Something else", None, None),
        Some(other.clone()),
        1,
    );
    world.as_account(&alice, |g| g.change(&alice.id, &p, &order, deadline));
    let effect = Effect::new(create(&p), Some(other.clone()));
    assert_eq!(
        verdict(&mut world, &alice, &p, &effect, 0),
        Verdict::NeedsOwner(NeedsOwnerReason::CreateDiffers)
    );

    // Cancelled: created and then deleted.
    world.fake.with_state(|s| s.cancel_event(&cal, &id));
    let effect = Effect::new(create(&p), Some(id));
    assert_eq!(
        verdict(&mut world, &alice, &p, &effect, 0),
        Verdict::NeedsOwner(NeedsOwnerReason::CreatedThenDeleted)
    );

    // Not found: read again no sooner than 60 s later, then absent.
    let effect = Effect::new(create(&p), Some(reserved(3)));
    let now = world.clock.now_ms();
    assert_eq!(
        verdict(&mut world, &alice, &p, &effect, 0),
        Verdict::ReadAgain {
            not_before_ms: now + 60_000
        }
    );
    world.clock.advance(60_000);
    assert_eq!(verdict(&mut world, &alice, &p, &effect, 1), Verdict::Absent);
    assert_eq!(earliest_read_ms(now), now + 10_000);
    world.check_log();
}

/// g43: every row for an update, an invitation and a response.
#[test]
fn reconciling_a_change_of_an_existing_event_follows_every_row_of_section_7() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let cal = p.provider_id.as_str().to_owned();
    let (id, etag) = invited(&world, &p);
    for (tool, action, applied) in changes_of_an_event(&p, &id, &etag) {
        let effect = Effect::new(action, None);
        assert_eq!(effect.tool(), Some(tool));
        // Found, the approved revision, not equal: not yet, then absent.
        let (_, etag_now) = invited(&world, &p);
        let fresh: Vec<_> = changes_of_an_event(&p, &id, &etag_now);
        let effect_now = Effect::new(
            fresh
                .iter()
                .find(|c| c.0 == tool)
                .expect("present")
                .1
                .clone(),
            None,
        );
        let now = world.clock.now_ms();
        assert_eq!(
            verdict(&mut world, &alice, &p, &effect_now, 0),
            Verdict::ReadAgain {
                not_before_ms: now + 60_000
            },
            "{tool:?}"
        );
        assert_eq!(
            verdict(&mut world, &alice, &p, &effect_now, 1),
            Verdict::Absent,
            "{tool:?}"
        );
        // Found and equal, whatever the revision: Applied, notification unknown.
        world.fake.change_event(&cal, &id, applied.clone());
        assert!(
            matches!(
                verdict(&mut world, &alice, &p, &effect_now, 0),
                Verdict::Applied {
                    notification: Notification::Unknown,
                    confirmed: true,
                    ..
                }
            ),
            "{tool:?}"
        );
        // Found, another revision, not equal: the owner looks.
        invited(&world, &p);
        world
            .fake
            .change_event(&cal, &id, json!({"location": "Elsewhere"}));
        assert_eq!(
            verdict(&mut world, &alice, &p, &effect, 0),
            Verdict::NeedsOwner(NeedsOwnerReason::ChangedUnclear),
            "{tool:?}"
        );
        // Cancelled, and not found: the owner looks.
        world.fake.with_state(|s| s.cancel_event(&cal, &id));
        assert_eq!(
            verdict(&mut world, &alice, &p, &effect, 0),
            Verdict::NeedsOwner(NeedsOwnerReason::GoneUnclear),
            "{tool:?}"
        );
        world
            .fake
            .with_state(|s| s.set_deleted_event_get(DeletedEventGet::Gone));
        assert_eq!(
            verdict(&mut world, &alice, &p, &effect, 1),
            Verdict::NeedsOwner(NeedsOwnerReason::GoneUnclear),
            "{tool:?}"
        );
        world.fake.with_state(|s| {
            s.set_deleted_event_get(DeletedEventGet::Cancelled);
            s.remove_event(&cal, &id);
        });
        assert_eq!(
            verdict(&mut world, &alice, &p, &effect, 0),
            Verdict::NeedsOwner(NeedsOwnerReason::GoneUnclear),
            "{tool:?}"
        );
    }
    world.check_log();
}

/// g43: every row for a delete.
#[test]
fn reconciling_a_delete_follows_every_row_of_section_7() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let cal = p.provider_id.as_str().to_owned();
    let (id, etag) = invited(&world, &p);
    let effect = Effect::new(delete(&p, event_ref(&id, &etag), SendUpdates::None), None);
    // Found, the approved revision: not yet, then absent.
    let now = world.clock.now_ms();
    assert_eq!(
        verdict(&mut world, &alice, &p, &effect, 0),
        Verdict::ReadAgain {
            not_before_ms: now + 60_000
        }
    );
    assert_eq!(verdict(&mut world, &alice, &p, &effect, 1), Verdict::Absent);
    // Found, another revision: absent at once.
    world
        .fake
        .change_event(&cal, &id, json!({"summary": "Changed"}));
    assert_eq!(verdict(&mut world, &alice, &p, &effect, 0), Verdict::Absent);
    // Cancelled, gone (410) and not found (404): applied.
    let applied = Verdict::Applied {
        revision: None,
        confirmed: true,
        notification: Notification::Unknown,
    };
    world.fake.with_state(|s| s.cancel_event(&cal, &id));
    assert_eq!(verdict(&mut world, &alice, &p, &effect, 0), applied);
    world
        .fake
        .with_state(|s| s.set_deleted_event_get(DeletedEventGet::Gone));
    assert_eq!(verdict(&mut world, &alice, &p, &effect, 0), applied);
    world.fake.with_state(|s| s.remove_event(&cal, &id));
    assert_eq!(verdict(&mut world, &alice, &p, &effect, 0), applied);
    world.check_log();
}

/// g43: the two reads for a create with a late-visible insert and for an update with a late
/// PATCH: the first read says read again, the second finds the change applied.
#[test]
fn a_late_insert_and_a_late_patch_are_found_on_the_second_read() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let deadline = world.deadline();

    world.fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars/",
        FaultAction::ApplyLate(LateTrigger::AfterReads(1)),
    ));
    let id = reserved(5);
    let order = ChangeOrder::new(create(&p), Some(id.clone()), 1);
    let report = world.as_account(&alice, |g| g.change(&alice.id, &p, &order, deadline));
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    let effect = Effect::new(create(&p), Some(id));
    assert!(matches!(
        verdict(&mut world, &alice, &p, &effect, 0),
        Verdict::ReadAgain { .. }
    ));
    world.clock.advance(60_000);
    assert!(matches!(
        verdict(&mut world, &alice, &p, &effect, 1),
        Verdict::Applied {
            confirmed: true,
            ..
        }
    ));

    let (id, etag) = invited(&world, &p);
    let mut c = no_changes();
    c.title = Some(EventTitle::new("Late").expect("valid"));
    let action = update(&p, event_ref(&id, &etag), c, SendUpdates::All);
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::ApplyLate(LateTrigger::AfterReads(1)),
    ));
    let order = ChangeOrder::new(action.clone(), None, 1);
    let deadline = world.deadline();
    let report = world.as_account(&alice, |g| g.change(&alice.id, &p, &order, deadline));
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    let effect = Effect::new(action, None);
    assert!(matches!(
        verdict(&mut world, &alice, &p, &effect, 0),
        Verdict::ReadAgain { .. }
    ));
    world.clock.advance(60_000);
    assert!(matches!(
        verdict(&mut world, &alice, &p, &effect, 1),
        Verdict::Applied {
            notification: Notification::Unknown,
            ..
        }
    ));
    assert_eq!(
        world.fake.log().calendar_changes().len(),
        2,
        "one changing request per order"
    );
    world.check_log();
}
