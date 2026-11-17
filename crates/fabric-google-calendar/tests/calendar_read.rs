//! Free/busy, reading events and observing one event (6.3 to 6.5): g20 to g27.

mod calendar_common;

use std::cell::Cell;

use calendar_common::{World, ZONE, put, query, timed, window};
use fabric_desktop_contracts::agent::{EventSpan, EventView, ResponseStatus, ToolResult};
use fabric_desktop_contracts::common::{EventField, FailureCode, Window};
use fabric_desktop_contracts::scalar::{Id, ProviderRef, Timestamp};
use fabric_google_calendar::calendar::{CalendarTarget, Observed};
use fabric_google_calendar::outcome::{ReadFailure, Sentence};
use fabric_google_fake::{CalendarEntry, DeletedEventGet, Fault, FaultAction};
use serde_json::{Value, json};

fn events_of(result: ToolResult) -> Vec<EventView> {
    let ToolResult::Events { events, .. } = result else {
        panic!("an events result");
    };
    events.into_vec()
}

fn rejected_with(failure: &ReadFailure) -> (FailureCode, String) {
    let ReadFailure::Rejected {
        code, explanation, ..
    } = failure
    else {
        panic!("rejected, not {failure:?}");
    };
    (*code, explanation.as_str().to_owned())
}

fn team(world: &mut World, person: &calendar_common::Person, role: &str) -> CalendarTarget {
    world.calendar(
        person,
        CalendarEntry::new("team#ops@group.calendar.google.com", role).time_zone(ZONE),
    )
}

/// g20: periods per calendar under the broker's ids, in UTC, cut to the window, ordered by
/// calendar then start; the request is exactly 6.3.
#[test]
fn free_busy_gives_periods_per_calendar_in_utc_cut_to_the_window() {
    let (mut world, alice) = World::with_alice();
    let team = team(&mut world, &alice, "reader");
    put(&world, &alice.primary, timed("p2", "Later", 14, 15));
    put(
        &world,
        &alice.primary,
        json!({"id": "p1", "start": {"dateTime": "2026-11-19T22:00:00+01:00"}, "end": {"dateTime": "2026-11-20T02:00:00+01:00"}}),
    );
    put(&world, &team, timed("t1", "Team", 9, 10));
    put(
        &world,
        &team,
        json!({"id": "t2", "start": {"dateTime": "2026-11-20T23:30:00Z"}, "end": {"dateTime": "2026-11-21T03:00:00Z"}}),
    );
    let deadline = world.deadline();
    let targets = [alice.primary.clone(), team.clone()];
    let result = world
        .as_account(&alice, |g| {
            g.free_busy(&alice.id, &targets, &window(), deadline)
        })
        .expect("busy times");
    let ToolResult::Availability { observed_at, busy } = result else {
        panic!("availability");
    };
    assert_eq!(observed_at.as_str(), "2026-11-17T09:00:00.000Z");
    let got: Vec<(&str, &str, &str)> = busy
        .iter()
        .map(|b| (b.calendar_id.as_str(), b.start.as_str(), b.end.as_str()))
        .collect();
    let (p, t) = (
        alice.primary.calendar.calendar_id.as_str(),
        team.calendar.calendar_id.as_str(),
    );
    assert_eq!(
        got,
        [
            (p, "2026-11-20T00:00:00.000Z", "2026-11-20T01:00:00.000Z"),
            (p, "2026-11-20T13:00:00.000Z", "2026-11-20T14:00:00.000Z"),
            (t, "2026-11-20T08:00:00.000Z", "2026-11-20T09:00:00.000Z"),
            (t, "2026-11-20T23:30:00.000Z", "2026-11-21T00:00:00.000Z"),
        ]
    );
    let request = world.fake.log().last().cloned().expect("a request");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/calendar/v3/freeBusy");
    assert_eq!(
        request.body_json(),
        Some(json!({
            "timeMin": "2026-11-20T00:00:00.000Z", "timeMax": "2026-11-21T00:00:00.000Z", "timeZone": "UTC",
            "items": [{"id": "alice@example.com"}, {"id": "team#ops@group.calendar.google.com"}],
        }))
    );
    world.check_log();
}

/// g20: an error for one calendar, or a calendar missing from the answer, fails the whole read.
#[test]
fn free_busy_with_an_error_or_a_missing_calendar_fails_as_a_whole() {
    let (mut world, alice) = World::with_alice();
    put(&world, &alice.primary, timed("p1", "Busy", 9, 10));
    let mut unknown = alice.primary.clone();
    unknown.provider_id =
        fabric_google_calendar::CalendarId::new("gone@group.example.com").expect("valid");
    unknown.calendar.calendar_id = Id::new("cal-unknown").expect("valid");
    let deadline = world.deadline();
    let targets = [alice.primary.clone(), unknown];
    let failure = world
        .as_account(&alice, |g| {
            g.free_busy(&alice.id, &targets, &window(), deadline)
        })
        .expect_err("fails");
    assert_eq!(
        rejected_with(&failure),
        (
            FailureCode::ProviderRejected,
            Sentence::FreeBusyIncomplete.text().to_owned()
        )
    );
    world.fake.arm(Fault::on(
        "POST",
        "/calendar/v3/freeBusy",
        FaultAction::Json(200, json!({"kind": "calendar#freeBusy", "calendars": {}})),
    ));
    let failure = world
        .as_account(&alice, |g| {
            g.free_busy(
                &alice.id,
                std::slice::from_ref(&alice.primary),
                &window(),
                deadline,
            )
        })
        .expect_err("fails");
    assert_eq!(
        rejected_with(&failure).1,
        Sentence::FreeBusyIncomplete.text()
    );
    world.check_log();
}

/// g21: the window bounds, occurrences with their identity, an all-day event as dates, an
/// occurrence of an all-day series without `occurrence`, a recurring master never returned,
/// two calendars in one order.
#[test]
fn reading_events_follows_the_window_and_gives_occurrences_and_all_day_events() {
    let (mut world, alice) = World::with_alice();
    let team = team(&mut world, &alice, "writer");
    let p = &alice.primary;
    // Ends exactly at the window's start, starts exactly at its end: both outside.
    put(
        &world,
        p,
        json!({"id": "before", "start": {"dateTime": "2026-11-19T23:00:00+01:00"}, "end": {"dateTime": "2026-11-20T01:00:00+01:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "after", "start": {"dateTime": "2026-11-21T01:00:00+01:00"}, "end": {"dateTime": "2026-11-21T02:00:00+01:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "edge", "start": {"dateTime": "2026-11-20T00:59:00+01:00"}, "end": {"dateTime": "2026-11-20T01:30:00+01:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "series", "recurrence": ["RRULE:FREQ=DAILY"], "summary": "Standup",
        "start": {"dateTime": "2026-11-18T10:00:00+01:00"}, "end": {"dateTime": "2026-11-18T10:15:00+01:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "series_20261120T090000Z", "recurringEventId": "series", "summary": "Standup",
        "originalStartTime": {"dateTime": "2026-11-20T10:00:00+01:00"},
        "start": {"dateTime": "2026-11-20T10:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:15:00+01:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "holiday", "summary": "Day off", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
    );
    put(
        &world,
        p,
        json!({"id": "weekly_20261120", "recurringEventId": "weekly", "originalStartTime": {"date": "2026-11-20"},
        "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
    );
    put(&world, &team, timed("teamevent", "Review", 15, 16));
    let deadline = world.deadline();
    let mut q = query(
        &[p.clone(), team.clone()],
        &[EventField::Time, EventField::Title],
    );
    q.fabric_created.insert((
        team.calendar.calendar_id.clone(),
        ProviderRef::new("teamevent").expect("valid"),
    ));
    let events = events_of(
        world
            .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
            .expect("events"),
    );
    let ids: Vec<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "edge",
            "holiday",
            "weekly_20261120",
            "series_20261120T090000Z",
            "teamevent"
        ]
    );
    let by = |id: &str| {
        events
            .iter()
            .find(|e| e.event_id.as_str() == id)
            .expect("present")
    };
    let occurrence = by("series_20261120T090000Z")
        .occurrence
        .clone()
        .expect("an occurrence");
    assert_eq!(occurrence.instance_id.as_str(), "series_20261120T090000Z");
    assert_eq!(
        occurrence.original_start_time.date_time.as_str(),
        "2026-11-20T10:00:00+01:00"
    );
    assert_eq!(occurrence.original_start_time.zone.as_str(), ZONE);
    assert!(matches!(by("holiday").time, Some(EventSpan::AllDay { .. })));
    assert!(by("holiday").occurrence.is_none());
    assert!(by("weekly_20261120").occurrence.is_none());
    assert!(by("teamevent").fabric_created && !by("edge").fabric_created);
    assert_eq!(by("teamevent").calendar_id, team.calendar.calendar_id);
    assert_eq!(by("edge").calendar_id, p.calendar.calendar_id);
    for request in world.fake.log().entries() {
        assert_eq!(request.query_value("singleEvents"), Some("true"));
        assert_eq!(request.query_value("orderBy"), Some("startTime"));
        assert_eq!(request.query_value("showDeleted"), Some("false"));
        assert_eq!(
            request.query_value("timeMin"),
            Some("2026-11-20T00:00:00.000Z")
        );
        assert_eq!(
            request.query_value("timeMax"),
            Some("2026-11-21T00:00:00.000Z")
        );
        assert_eq!(request.query_value("maxResults"), Some("250"));
        assert_eq!(request.query_value("maxAttendees"), None);
        assert_eq!(request.query_value("timeZone"), Some(ZONE));
    }
    assert_eq!(
        world.fake.log().entries()[1].segments[3],
        "team#ops@group.calendar.google.com"
    );
    world.check_log();
}

/// g21: a cancelled item is skipped; several pages of several calendars in one order.
#[test]
fn a_read_follows_every_page_of_every_calendar_and_skips_cancelled_items() {
    let (mut world, alice) = World::with_alice();
    let team = team(&mut world, &alice, "reader");
    for n in 0..260 {
        put(
            &world,
            &alice.primary,
            json!({"id": format!("ev{n:04}"), "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T09:30:00+01:00"}}),
        );
    }
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/team#ops@group.calendar.google.com/events",
        FaultAction::Json(200, json!({"kind": "calendar#events", "items": [
            {"id": "gone", "status": "cancelled"},
            {"id": "kept", "etag": "\"9\"", "status": "confirmed",
             "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"}},
        ]})),
    ));
    let deadline = world.deadline();
    let q = query(&[alice.primary.clone(), team], &[EventField::Time]);
    let events = events_of(
        world
            .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
            .expect("events"),
    );
    assert_eq!(events.len(), 261);
    assert_eq!(events.last().expect("one").event_id.as_str(), "kept");
    let log = world.fake.log();
    assert_eq!(log.len(), 3);
    assert_eq!(log.entries()[0].query_value("pageToken"), None);
    assert!(log.entries()[1].query_value("pageToken").is_some());
    assert_eq!(log.entries()[2].query_value("pageToken"), None);
    world.check_log();
}

/// g22: `still_allowed` turned false between two pages: `Stopped`, nothing returned, no
/// further request (PLAN 5 R08, the adapter's half).
#[test]
fn a_read_stops_between_pages_when_it_is_no_longer_allowed_and_returns_nothing() {
    let (mut world, alice) = World::with_alice();
    for n in 0..300 {
        put(
            &world,
            &alice.primary,
            json!({"id": format!("ev{n:04}"), "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
    }
    let deadline = world.deadline();
    let asked = Cell::new(0);
    let allowed = || {
        asked.set(asked.get() + 1);
        asked.get() == 1
    };
    let q = query(std::slice::from_ref(&alice.primary), &[EventField::Time]);
    let result = world.as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &allowed));
    assert_eq!(result, Err(ReadFailure::Stopped));
    assert_eq!(asked.get(), 2);
    assert_eq!(world.fake.log().len(), 1, "no request after the stop");
    // Not allowed from the start: no request at all.
    let before = world.fake.log().len();
    let result = world.as_account(&alice, |g| {
        g.read_events(&alice.id, &q, deadline, &|| false)
    });
    assert_eq!(result, Err(ReadFailure::Stopped));
    assert_eq!(world.fake.log().len(), before);
    world.check_log();
}

/// g23: for each granted field, `fields=` asks only its members, and a view shows a field only
/// when it is granted even though the fake sends everything.
#[test]
fn each_granted_field_asks_only_its_members_and_the_view_shows_only_granted_fields() {
    let (mut world, alice) = World::with_alice();
    put(
        &world,
        &alice.primary,
        json!({
            "id": "full", "summary": "Title", "location": "Place", "description": "Notes",
            "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"},
            "organizer": {"email": "boss@example.com"},
            "attendees": [{"email": "alice@example.com", "self": true, "responseStatus": "accepted"}, {"email": "x@example.com", "responseStatus": "needsAction"}],
        }),
    );
    let base = "id,etag,status,recurringEventId";
    let cases = [
        (EventField::Time, "start,end,originalStartTime"),
        (EventField::Title, "summary"),
        (EventField::Location, "location"),
        (EventField::Description, "description"),
        (
            EventField::Attendees,
            "attendees(email,responseStatus,self),attendeesOmitted",
        ),
        (EventField::Organizer, "organizer(email)"),
        (
            EventField::OwnResponse,
            "attendees(email,responseStatus,self),attendeesOmitted",
        ),
    ];
    let deadline = world.deadline();
    for (field, members) in cases {
        let q = query(std::slice::from_ref(&alice.primary), &[field]);
        let view = events_of(
            world
                .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
                .expect("events"),
        )
        .remove(0);
        let request = world.fake.log().last().cloned().expect("a request");
        assert_eq!(
            request.query_value("fields"),
            Some(format!("nextPageToken,items({base},{members})").as_str())
        );
        assert_eq!(view.time.is_some(), field == EventField::Time, "{field:?}");
        assert_eq!(view.occurrence, None);
        assert_eq!(
            view.title.is_some(),
            field == EventField::Title,
            "{field:?}"
        );
        assert_eq!(
            view.location.is_some(),
            field == EventField::Location,
            "{field:?}"
        );
        assert_eq!(
            view.description.is_some(),
            field == EventField::Description,
            "{field:?}"
        );
        assert_eq!(
            view.attendees.is_some(),
            field == EventField::Attendees,
            "{field:?}"
        );
        assert_eq!(
            view.organizer.is_some(),
            field == EventField::Organizer,
            "{field:?}"
        );
        assert_eq!(
            view.own_response.is_some(),
            field == EventField::OwnResponse,
            "{field:?}"
        );
    }
    world.check_log();
}

/// g24: past 2 500 events, past 768 KiB, a cut attendee list, more than 2 000 attendees: the
/// read fails with the fixed sentence and nothing partial is returned.
#[test]
fn a_read_past_its_bounds_fails_with_the_fixed_sentence() {
    let (mut world, alice) = World::with_alice();
    let deadline = world.deadline();
    let too_large = (
        FailureCode::InvalidArguments,
        Sentence::WindowTooLarge.text().to_owned(),
    );

    let mut many = World::new();
    let bob = many.person("bob");
    // Short ids and etags keep 2 500 views under 768 KiB, so the count bound is what fails.
    for n in 0..2_501 {
        put(
            &many,
            &bob.primary,
            json!({"id": format!("e{n:05}"), "etag": "\"1\"", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
    }
    let q = query(std::slice::from_ref(&bob.primary), &[EventField::Time]);
    let failure = many
        .as_account(&bob, |g| g.read_events(&bob.id, &q, deadline, &|| true))
        .expect_err("past 2 500");
    assert_eq!(rejected_with(&failure), too_large);
    many.fake
        .with_state(|s| s.remove_event(bob.primary.provider_id.as_str(), "e02500"));
    let ok = many
        .as_account(&bob, |g| g.read_events(&bob.id, &q, deadline, &|| true))
        .expect("2 500 fit");
    assert_eq!(events_of(ok).len(), 2_500);
    many.check_log();

    for n in 0..100 {
        put(
            &world,
            &alice.primary,
            json!({"id": format!("big{n:03}"), "description": "d".repeat(8_000),
            "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
    }
    let q = query(
        std::slice::from_ref(&alice.primary),
        &[EventField::Description],
    );
    let failure = world
        .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
        .expect_err("past 768 KiB");
    assert_eq!(rejected_with(&failure), too_large);

    let mut fresh = World::new();
    let carol = fresh.person("carol");
    put(
        &fresh,
        &carol.primary,
        json!({"id": "cut", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"},
        "attendees": [{"email": "carol@example.com", "self": true}, {"email": "x@example.com"}]}),
    );
    fresh.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::CutAttendees,
    ));
    let q = query(
        std::slice::from_ref(&carol.primary),
        &[EventField::Attendees],
    );
    let failure = fresh
        .as_account(&carol, |g| g.read_events(&carol.id, &q, deadline, &|| true))
        .expect_err("a cut list");
    assert_eq!(rejected_with(&failure), too_large);
    let attendees: Vec<Value> = (0..2_001)
        .map(|i| json!({"email": format!("p{i}@example.com")}))
        .collect();
    put(
        &fresh,
        &carol.primary,
        json!({"id": "cut", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}, "attendees": attendees}),
    );
    let failure = fresh
        .as_account(&carol, |g| g.read_events(&carol.id, &q, deadline, &|| true))
        .expect_err("past 2 000 attendees");
    assert_eq!(rejected_with(&failure), too_large);
    // Without attendees asked for, the same event reads.
    let q = query(std::slice::from_ref(&carol.primary), &[EventField::Title]);
    assert!(
        fresh
            .as_account(&carol, |g| g.read_events(&carol.id, &q, deadline, &|| true))
            .is_ok()
    );
    fresh.check_log();
    world.check_log();
}

/// g25: `Z`, an offset and a fraction; every events read names the calendar's zone and the
/// view pairs Google's offset with it; a daylight-saving gap and fold as Google writes them; an
/// all-day date is never an instant.
#[test]
fn times_pair_googles_offset_with_the_calendars_zone() {
    let (mut world, alice) = World::with_alice();
    let p = &alice.primary;
    put(
        &world,
        p,
        json!({"id": "zulu", "start": {"dateTime": "2026-11-20T08:00:00Z"}, "end": {"dateTime": "2026-11-20T09:00:00.999Z"}}),
    );
    put(
        &world,
        p,
        json!({"id": "gap", "start": {"dateTime": "2026-03-29T03:30:00+02:00"}, "end": {"dateTime": "2026-03-29T04:00:00+02:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "fold1", "start": {"dateTime": "2026-10-25T02:30:00+02:00"}, "end": {"dateTime": "2026-10-25T02:45:00+02:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "fold2", "start": {"dateTime": "2026-10-25T02:30:00+01:00"}, "end": {"dateTime": "2026-10-25T02:45:00+01:00"}}),
    );
    put(
        &world,
        p,
        json!({"id": "day", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
    );
    let mut q = query(std::slice::from_ref(p), &[EventField::Time]);
    q.window = Window {
        start: Timestamp::new("2026-03-01T00:00:00.000Z").expect("valid"),
        end: Timestamp::new("2026-12-01T00:00:00.000Z").expect("valid"),
    };
    let deadline = world.deadline();
    let events = events_of(
        world
            .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
            .expect("events"),
    );
    let timed: Vec<(&str, String, String, String)> = events
        .iter()
        .filter_map(|e| match &e.time {
            Some(EventSpan::Timed { start, end }) => Some((
                e.event_id.as_str(),
                start.date_time.as_str().to_owned(),
                end.date_time.as_str().to_owned(),
                start.zone.as_str().to_owned(),
            )),
            _ => None,
        })
        .collect();
    let z = ZONE.to_owned();
    assert_eq!(
        timed,
        [
            (
                "gap",
                "2026-03-29T03:30:00+02:00".into(),
                "2026-03-29T04:00:00+02:00".into(),
                z.clone()
            ),
            (
                "fold1",
                "2026-10-25T02:30:00+02:00".into(),
                "2026-10-25T02:45:00+02:00".into(),
                z.clone()
            ),
            (
                "fold2",
                "2026-10-25T02:30:00+01:00".into(),
                "2026-10-25T02:45:00+01:00".into(),
                z.clone()
            ),
            (
                "zulu",
                "2026-11-20T08:00:00+00:00".into(),
                "2026-11-20T09:00:00+00:00".into(),
                z
            ),
        ]
    );
    let day = events
        .iter()
        .find(|e| e.event_id.as_str() == "day")
        .expect("present");
    let Some(EventSpan::AllDay {
        start_date,
        end_date,
    }) = &day.time
    else {
        panic!("all day");
    };
    assert_eq!(
        (start_date.as_str(), end_date.as_str()),
        ("2026-11-20", "2026-11-21")
    );
    // The observation of an event reads in the calendar's zone too.
    world
        .as_account(&alice, |g| g.observe(&alice.id, p, "zulu", false, deadline))
        .expect("observed");
    for request in world.fake.log().entries() {
        assert_eq!(request.query_value("timeZone"), Some(ZONE));
    }
    world.check_log();
}

/// g26: on the primary calendar `own_response` and `invitation` follow the entry marked
/// `self`; on a shared calendar `own_response` is `null`, `invitation` is false and the marked
/// entry is among the attendees.
#[test]
fn the_own_entry_counts_only_on_the_primary_calendar() {
    let (mut world, alice) = World::with_alice();
    let shared = world.calendar(
        &alice,
        CalendarEntry::new("bob@example.com", "writer").time_zone(ZONE),
    );
    let invite = |owner: &str| {
        json!({"id": "inv", "summary": "Invite", "organizer": {"email": "carol@example.com"},
        "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"},
        "attendees": [
            {"email": owner, "self": true, "responseStatus": "tentative"},
            {"email": "carol@example.com", "organizer": true, "responseStatus": "accepted"},
        ]})
    };
    put(&world, &alice.primary, invite("alice@example.com"));
    put(&world, &shared, invite("bob@example.com"));
    let deadline = world.deadline();
    let q = query(
        &[alice.primary.clone(), shared.clone()],
        &[EventField::OwnResponse, EventField::Attendees],
    );
    let events = events_of(
        world
            .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
            .expect("events"),
    );
    assert_eq!(events[0].own_response, Some(ResponseStatus::Tentative));
    assert_eq!(events[1].own_response, None);
    assert_eq!(events[1].attendees.as_ref().expect("attendees").len(), 2);
    let observed = |world: &mut World, target: &CalendarTarget| {
        let Observed::Found { observation, .. } = world
            .as_account(&alice, |g| {
                g.observe(&alice.id, target, "inv", false, deadline)
            })
            .expect("observed")
        else {
            panic!("found");
        };
        observation
    };
    let own = observed(&mut world, &alice.primary);
    assert!(own.invitation);
    let addresses: Vec<&str> = own.attendees.iter().map(|e| e.as_str()).collect();
    assert_eq!(addresses, ["carol@example.com"]);
    let other = observed(&mut world, &shared);
    assert!(!other.invitation);
    let addresses: Vec<&str> = other.attendees.iter().map(|e| e.as_str()).collect();
    assert_eq!(addresses, ["bob@example.com", "carol@example.com"]);
    world.check_log();
}

/// g27: each member of 6.5 from fixtures; `Absent` for 404, 410 and cancelled; a cut attendee
/// list and an unusable attendee address fail the observation.
#[test]
fn an_observation_gives_each_member_of_6_5_and_absent_for_a_gone_event() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let deadline = world.deadline();
    let (_, plain_etag) = put(&world, &p, timed("plain", "Plain", 9, 10));
    put(
        &world,
        &p,
        json!({"id": "inv", "organizer": {"email": "boss@Example.com"},
        "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"},
        "attendees": [{"email": "alice@example.com", "self": true}, {"email": "Boss@Example.com", "organizer": true}, {"email": "room1@resource.example.com", "resource": true}]}),
    );
    put(
        &world,
        &p,
        json!({"id": "master", "recurrence": ["RRULE:FREQ=WEEKLY"], "start": {"dateTime": "2026-11-20T09:00:00+01:00"}, "end": {"dateTime": "2026-11-20T10:00:00+01:00"}}),
    );
    put(
        &world,
        &p,
        json!({"id": "master_20261127T080000Z", "recurringEventId": "master", "originalStartTime": {"dateTime": "2026-11-27T09:00:00+01:00"},
        "start": {"dateTime": "2026-11-27T11:00:00+01:00"}, "end": {"dateTime": "2026-11-27T12:00:00+01:00"}}),
    );
    put(
        &world,
        &p,
        json!({"id": "allday", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
    );
    put(
        &world,
        &p,
        json!({"id": "focus", "eventType": "focusTime", "start": {"dateTime": "2026-11-20T13:00:00+01:00"}, "end": {"dateTime": "2026-11-20T15:00:00+01:00"}}),
    );
    let observe = |world: &mut World, id: &str, created: bool| {
        world.as_account(&alice, |g| g.observe(&alice.id, &p, id, created, deadline))
    };
    let found = |o: Result<Observed, ReadFailure>| match o.expect("observed") {
        Observed::Found {
            observation,
            before,
        } => (observation, before),
        Observed::Absent => panic!("found"),
    };
    let (o, before) = found(observe(&mut world, "plain", true));
    assert_eq!(o.event_id.as_str(), "plain");
    assert_eq!(o.revision.as_str(), plain_etag);
    assert_eq!(o.calendar, p.calendar);
    assert!(o.fabric_created && !o.recurring && o.default_type && o.timed && !o.invitation);
    assert!(o.attendees.is_empty() && o.occurrence_start_ms.is_none());
    assert_eq!(before.title.as_ref().expect("title").as_str(), "Plain");
    assert!(before.fabric_created);
    let (o, _) = found(observe(&mut world, "inv", false));
    assert!(o.invitation);
    let addresses: Vec<&str> = o.attendees.iter().map(|e| e.as_str()).collect();
    assert_eq!(
        addresses,
        ["Boss@example.com", "room1@resource.example.com"]
    );
    let (o, _) = found(observe(&mut world, "master", false));
    assert!(o.recurring && o.occurrence_start_ms.is_none());
    let (o, _) = found(observe(&mut world, "master_20261127T080000Z", false));
    assert!(o.recurring);
    assert_eq!(o.occurrence_start_ms, Some(1_795_766_400_000));
    let (o, _) = found(observe(&mut world, "allday", false));
    assert!(!o.timed && o.default_type);
    let (o, _) = found(observe(&mut world, "focus", false));
    assert!(o.timed && !o.default_type);

    // Absent: 404, cancelled (200 with `status: cancelled`), 410.
    assert_eq!(
        observe(&mut world, "nosuchevent", false),
        Ok(Observed::Absent)
    );
    world
        .fake
        .with_state(|s| s.cancel_event(p.provider_id.as_str(), "plain"));
    assert_eq!(observe(&mut world, "plain", false), Ok(Observed::Absent));
    world
        .fake
        .with_state(|s| s.set_deleted_event_get(DeletedEventGet::Gone));
    assert_eq!(observe(&mut world, "plain", false), Ok(Observed::Absent));
    let statuses: Vec<Option<u16>> = world
        .fake
        .log()
        .entries()
        .iter()
        .rev()
        .take(3)
        .map(|e| e.answered)
        .collect();
    assert_eq!(statuses, [Some(410), Some(200), Some(404)]);

    // A cut attendee list and an unusable address fail it.
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::CutAttendees,
    ));
    let failure = observe(&mut world, "inv", false).expect_err("a cut list");
    assert_eq!(
        rejected_with(&failure),
        (
            FailureCode::InvalidArguments,
            Sentence::AttendeesCut.text().to_owned()
        )
    );
    put(
        &world,
        &p,
        json!({"id": "badaddr", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"},
        "attendees": [{"email": "J\u{f6}rg@example.com"}]}),
    );
    let failure = observe(&mut world, "badaddr", false).expect_err("an unusable address");
    assert_eq!(
        rejected_with(&failure),
        (
            FailureCode::InvalidArguments,
            Sentence::AttendeeAddress.text().to_owned()
        )
    );
    world.check_log();
}
