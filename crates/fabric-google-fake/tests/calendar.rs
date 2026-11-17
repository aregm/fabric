//! f01, the Calendar side: `If-Match`, ids, revisions, merge patches, the list window,
//! free/busy, the calendar list and access, through raw HTTP (no adapter).

mod common;

use common::*;
use fabric_google_fake::{CalendarEntry, DeletedEventGet, FakeGoogle, SCOPE_EVENTS};
use serde_json::{Value, json};

fn timed(id: &str, start: &str, end: &str) -> Value {
    json!({"id": id, "summary": id, "start": {"dateTime": start}, "end": {"dateTime": end}})
}

fn etag_number(etag: &str) -> u64 {
    etag.trim_matches('"').parse().unwrap()
}

fn ids(answer: &Answer) -> Vec<String> {
    answer.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

fn list(fake: &FakeGoogle, token: &str, calendar: &str, query: &str) -> Answer {
    api(
        fake,
        token,
        "GET",
        &format!("{}?{query}", events_path(calendar)),
        &[],
        None,
    )
}

#[test]
fn if_match_on_patch_and_delete_answers_412_for_another_revision_and_star_passes() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        timed(
            "evt1",
            "2026-11-18T10:00:00+01:00",
            "2026-11-18T11:00:00+01:00",
        ),
    );
    let etag = fake.with_state(|s| s.etag(ALICE_EMAIL, "evt1")).unwrap();
    let path = event_path(ALICE_EMAIL, "evt1");
    let body = json!({"summary": "changed"});

    let stale = api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[("If-Match", "\"1\"")],
        Some(&body),
    );
    assert_eq!(
        (stale.status, stale.reason().as_deref()),
        (412, Some("conditionNotMet"))
    );
    assert_eq!(
        fake.event(ALICE_EMAIL, "evt1").unwrap()["summary"],
        "evt1",
        "unchanged"
    );

    let weak = format!("W/{etag}");
    let weak_answer = api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[("If-Match", &weak)],
        Some(&body),
    );
    assert_eq!(weak_answer.status, 412);

    let right = api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[("If-Match", &etag)],
        Some(&body),
    );
    assert_eq!(right.status, 200);
    let new_etag = right.json()["etag"].as_str().unwrap().to_string();
    assert_ne!(new_etag, etag);

    let old_delete = api(&fake, &t, "DELETE", &path, &[("If-Match", &etag)], None);
    assert_eq!(old_delete.status, 412);
    let star = api(&fake, &t, "DELETE", &path, &[("If-Match", "*")], None);
    assert_eq!((star.status, star.body.len()), (204, 0));
    assert_eq!(
        fake.event(ALICE_EMAIL, "evt1").unwrap()["status"],
        "cancelled"
    );
}

#[test]
fn an_insert_with_an_existing_id_is_a_duplicate_and_a_bad_id_is_invalid() {
    let fake = fake();
    let t = token(&fake, ALICE);
    let path = format!("{}?sendUpdates=none", events_path(ALICE_EMAIL));
    let event = timed(
        "abcdefghij0123456789abcdef",
        "2026-11-18T10:00:00Z",
        "2026-11-18T11:00:00Z",
    );
    let first = api(&fake, &t, "POST", &path, &[], Some(&event));
    assert_eq!(first.status, 200);
    let created = first.json();
    assert_eq!(created["id"], "abcdefghij0123456789abcdef");
    assert_eq!(created["status"], "confirmed");
    assert_eq!(
        created["organizer"],
        json!({"email": ALICE_EMAIL, "self": true})
    );
    assert!(created["etag"].as_str().unwrap().starts_with('"'));

    let again = api(&fake, &t, "POST", &path, &[], Some(&event));
    assert_eq!(
        (again.status, again.reason().as_deref()),
        (409, Some("duplicate"))
    );
    fake.with_state(|s| s.cancel_event(ALICE_EMAIL, "abcdefghij0123456789abcdef"));
    let after_delete = api(&fake, &t, "POST", &path, &[], Some(&event));
    assert_eq!(
        after_delete.status, 409,
        "a deleted event's id is not free again"
    );

    for bad in ["abcd", "ABCDEFGH", "abcdwxyz", "abc_defg"] {
        let mut e = event.clone();
        e["id"] = json!(bad);
        let answer = api(&fake, &t, "POST", &path, &[], Some(&e));
        assert_eq!(
            (answer.status, answer.reason().as_deref()),
            (400, Some("invalid")),
            "{bad}"
        );
    }
    let mut no_id = event.clone();
    no_id.as_object_mut().unwrap().remove("id");
    let generated = api(&fake, &t, "POST", &path, &[], Some(&no_id)).json();
    let id = generated["id"].as_str().unwrap();
    assert_eq!(id.len(), 26);
    assert!(id.bytes().all(|b| matches!(b, b'a'..=b'v' | b'0'..=b'9')));

    let backwards = timed("zzzzz", "2026-11-18T11:00:00Z", "2026-11-18T10:00:00Z");
    let mut backwards = backwards;
    backwards["id"] = json!("reverse01");
    let answer = api(&fake, &t, "POST", &path, &[], Some(&backwards));
    assert_eq!(answer.reason().as_deref(), Some("timeRangeEmpty"));
    let bad_updates = api(
        &fake,
        &t,
        "POST",
        &format!("{}?sendUpdates=some", events_path(ALICE_EMAIL)),
        &[],
        Some(&no_id),
    );
    assert_eq!(bad_updates.status, 400);
}

#[test]
fn revisions_rise_with_every_change() {
    let fake = fake();
    let t = token(&fake, ALICE);
    let id = fake.put_event(
        ALICE_EMAIL,
        timed("evt2", "2026-11-18T10:00:00Z", "2026-11-18T11:00:00Z"),
    );
    let path = event_path(ALICE_EMAIL, &id);
    let mut seen = vec![etag_number(
        &fake.with_state(|s| s.etag(ALICE_EMAIL, &id)).unwrap(),
    )];
    let patched = api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[],
        Some(&json!({"location": "x"})),
    );
    seen.push(etag_number(patched.json()["etag"].as_str().unwrap()));
    let same = api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[],
        Some(&json!({"location": "x"})),
    );
    seen.push(etag_number(same.json()["etag"].as_str().unwrap()));
    seen.push(etag_number(
        &fake
            .change_event(ALICE_EMAIL, &id, json!({"summary": "other"}))
            .unwrap(),
    ));
    api(&fake, &t, "DELETE", &path, &[], None);
    seen.push(etag_number(
        &fake.with_state(|s| s.etag(ALICE_EMAIL, &id)).unwrap(),
    ));
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
}

#[test]
fn patch_is_a_json_merge_patch() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        json!({
            "id": "evt3",
            "summary": "Plan",
            "description": "keep me",
            "location": "Room 1",
            "start": {"date": "2026-11-18"},
            "end": {"date": "2026-11-19"},
            "attendees": [{"email": "a@x.example", "responseStatus": "accepted"}, {"email": "b@x.example"}],
            "extendedProperties": {"private": {"k": "v"}},
        }),
    );
    let body = json!({
        "location": null,
        "start": {"dateTime": "2026-11-18T10:00:00+01:00", "timeZone": "Europe/Berlin", "date": null},
        "end": {"dateTime": "2026-11-18T11:00:00+01:00", "timeZone": "Europe/Berlin", "date": null},
        "attendees": [{"email": "c@x.example"}],
    });
    let answer = api(
        &fake,
        &t,
        "PATCH",
        &format!("{}?sendUpdates=all", event_path(ALICE_EMAIL, "evt3")),
        &[],
        Some(&body),
    );
    assert_eq!(answer.status, 200);
    let e = fake.event(ALICE_EMAIL, "evt3").unwrap();
    assert!(e.get("location").is_none(), "null removes");
    assert_eq!(
        e["start"],
        json!({"dateTime": "2026-11-18T10:00:00+01:00", "timeZone": "Europe/Berlin"})
    );
    assert_eq!(
        e["attendees"],
        json!([{"email": "c@x.example", "responseStatus": "needsAction"}]),
        "arrays replace"
    );
    assert_eq!(e["description"], "keep me");
    assert_eq!(e["extendedProperties"], json!({"private": {"k": "v"}}));
    assert_eq!(answer.json(), e, "the answer is the event after the change");

    let mixed = json!({"end": {"date": "2026-11-20", "dateTime": null}});
    let refused = api(
        &fake,
        &t,
        "PATCH",
        &event_path(ALICE_EMAIL, "evt3"),
        &[],
        Some(&mixed),
    );
    assert_eq!(refused.status, 400);
    assert_eq!(
        fake.event(ALICE_EMAIL, "evt3").unwrap()["end"]["timeZone"],
        "Europe/Berlin",
        "unchanged"
    );
    let other_id = api(
        &fake,
        &t,
        "PATCH",
        &event_path(ALICE_EMAIL, "evt3"),
        &[],
        Some(&json!({"id": "other"})),
    );
    assert_eq!(other_id.status, 400);
}

#[test]
fn attendees_omitted_merges_the_given_attendees_by_address() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        json!({
            "id": "evt4",
            "start": {"dateTime": "2026-11-18T10:00:00Z"}, "end": {"dateTime": "2026-11-18T11:00:00Z"},
            "attendees": [
                {"email": "org@x.example", "organizer": true, "responseStatus": "accepted"},
                {"email": ALICE_EMAIL, "self": true, "responseStatus": "needsAction"},
            ],
        }),
    );
    let body = json!({"attendeesOmitted": true, "attendees": [{"email": "ALICE@example.com", "responseStatus": "accepted"}]});
    let answer = api(
        &fake,
        &t,
        "PATCH",
        &event_path(ALICE_EMAIL, "evt4"),
        &[],
        Some(&body),
    );
    assert_eq!(answer.status, 200);
    let attendees = fake.event(ALICE_EMAIL, "evt4").unwrap()["attendees"].clone();
    assert_eq!(attendees.as_array().unwrap().len(), 2);
    assert_eq!(attendees[0]["responseStatus"], "accepted");
    assert_eq!(attendees[1]["responseStatus"], "accepted");
    assert_eq!(attendees[1]["email"], "ALICE@example.com");
    assert!(
        fake.event(ALICE_EMAIL, "evt4")
            .unwrap()
            .get("attendeesOmitted")
            .is_none()
    );
}

#[test]
fn a_deleted_event_reads_as_the_test_chose_and_a_second_delete_is_gone() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        timed("evt5", "2026-11-18T10:00:00Z", "2026-11-18T11:00:00Z"),
    );
    let path = event_path(ALICE_EMAIL, "evt5");
    assert_eq!(api(&fake, &t, "DELETE", &path, &[], None).status, 204);
    let get = api(&fake, &t, "GET", &path, &[], None);
    assert_eq!(
        (get.status, get.json()["status"].clone()),
        (200, json!("cancelled"))
    );
    fake.with_state(|s| s.set_deleted_event_get(DeletedEventGet::NotFound));
    assert_eq!(api(&fake, &t, "GET", &path, &[], None).status, 404);
    fake.with_state(|s| s.set_deleted_event_get(DeletedEventGet::Gone));
    assert_eq!(
        api(&fake, &t, "GET", &path, &[], None).reason().as_deref(),
        Some("deleted")
    );
    let second = api(&fake, &t, "DELETE", &path, &[], None);
    assert_eq!(
        (second.status, second.reason().as_deref()),
        (410, Some("deleted"))
    );
    assert_eq!(
        api(
            &fake,
            &t,
            "GET",
            &event_path(ALICE_EMAIL, "never"),
            &[],
            None
        )
        .status,
        404
    );
}

#[test]
fn the_list_window_bounds_the_end_by_time_min_and_the_start_by_time_max_both_exclusive() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        timed("before", "2026-11-18T08:00:00Z", "2026-11-18T09:00:00Z"),
    );
    fake.put_event(
        ALICE_EMAIL,
        timed(
            "overlapsstart",
            "2026-11-18T08:30:00Z",
            "2026-11-18T09:30:00Z",
        ),
    );
    fake.put_event(
        ALICE_EMAIL,
        timed(
            "inside",
            "2026-11-18T10:00:00+01:00",
            "2026-11-18T11:00:00+01:00",
        ),
    );
    fake.put_event(
        ALICE_EMAIL,
        timed("atend", "2026-11-18T12:00:00Z", "2026-11-18T13:00:00Z"),
    );
    fake.put_event(
        ALICE_EMAIL,
        json!({"id": "allday", "start": {"date": "2026-11-18"}, "end": {"date": "2026-11-19"}}),
    );
    let answer = list(
        &fake,
        &t,
        ALICE_EMAIL,
        "singleEvents=true&orderBy=startTime&timeMin=2026-11-18T09%3A00%3A00Z&timeMax=2026-11-18T12%3A00%3A00Z&timeZone=Europe%2FBerlin",
    );
    assert_eq!(answer.status, 200);
    assert_eq!(ids(&answer), vec!["allday", "overlapsstart", "inside"]);
    assert_eq!(answer.json()["timeZone"], "Europe/Berlin");
    assert_eq!(answer.json()["accessRole"], "owner");
    let logged = fake.log().last().cloned().unwrap();
    assert_eq!(logged.query_value("timeZone"), Some("Europe/Berlin"));

    let empty = list(
        &fake,
        &t,
        ALICE_EMAIL,
        "timeMin=2026-11-18T12%3A00%3A00Z&timeMax=2026-11-18T09%3A00%3A00Z",
    );
    assert_eq!(empty.reason().as_deref(), Some("timeRangeEmpty"));
    let unordered = list(&fake, &t, ALICE_EMAIL, "orderBy=startTime");
    assert_eq!(unordered.status, 400, "startTime needs singleEvents");
}

#[test]
fn single_events_lists_occurrences_never_masters_and_pages_and_hides_cancelled() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        json!({"id": "series", "recurrence": ["RRULE:FREQ=DAILY;COUNT=3"],
               "start": {"dateTime": "2026-11-18T10:00:00Z"}, "end": {"dateTime": "2026-11-18T11:00:00Z"}}),
    );
    for day in 18..21 {
        fake.put_event(
            ALICE_EMAIL,
            json!({"id": format!("series_202611{day}T100000Z"), "recurringEventId": "series",
                   "originalStartTime": {"dateTime": format!("2026-11-{day}T10:00:00Z")},
                   "start": {"dateTime": format!("2026-11-{day}T10:00:00Z")},
                   "end": {"dateTime": format!("2026-11-{day}T11:00:00Z")}}),
        );
    }
    fake.with_state(|s| s.cancel_event(ALICE_EMAIL, "series_20261119T100000Z"));
    let first = list(
        &fake,
        &t,
        ALICE_EMAIL,
        "singleEvents=true&orderBy=startTime&maxResults=1",
    );
    assert_eq!(ids(&first), vec!["series_20261118T100000Z"]);
    let token_value = first.json()["nextPageToken"].as_str().unwrap().to_string();
    let second = list(
        &fake,
        &t,
        ALICE_EMAIL,
        &format!(
            "singleEvents=true&orderBy=startTime&maxResults=1&pageToken={}",
            enc(&token_value)
        ),
    );
    assert_eq!(ids(&second), vec!["series_20261120T100000Z"]);
    assert!(second.json().get("nextPageToken").is_none());
    let deleted = list(&fake, &t, ALICE_EMAIL, "singleEvents=true&showDeleted=true");
    assert_eq!(ids(&deleted).len(), 3);
    let masters = list(&fake, &t, ALICE_EMAIL, "singleEvents=false");
    assert!(ids(&masters).contains(&"series".to_string()));
    let bad_token = list(&fake, &t, ALICE_EMAIL, "pageToken=nonsense");
    assert_eq!(bad_token.status, 400);
}

#[test]
fn max_attendees_and_a_cut_fault_leave_only_the_own_entry() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        json!({"id": "big", "start": {"dateTime": "2026-11-18T10:00:00Z"}, "end": {"dateTime": "2026-11-18T11:00:00Z"},
               "attendees": [{"email": "x@x.example"}, {"email": ALICE_EMAIL, "self": true}, {"email": "y@x.example"}]}),
    );
    let cut = api(
        &fake,
        &t,
        "GET",
        &format!("{}?maxAttendees=2", event_path(ALICE_EMAIL, "big")),
        &[],
        None,
    )
    .json();
    assert_eq!(cut["attendeesOmitted"], true);
    assert_eq!(
        cut["attendees"],
        json!([{"email": ALICE_EMAIL, "self": true}])
    );
    let whole = api(
        &fake,
        &t,
        "GET",
        &format!("{}?maxAttendees=3", event_path(ALICE_EMAIL, "big")),
        &[],
        None,
    )
    .json();
    assert!(whole.get("attendeesOmitted").is_none());
    fake.arm(fabric_google_fake::Fault::on(
        "GET",
        "/calendar/v3/calendars",
        fabric_google_fake::FaultAction::CutAttendees,
    ));
    let faulted = list(&fake, &t, ALICE_EMAIL, "singleEvents=true").json();
    assert_eq!(faulted["items"][0]["attendeesOmitted"], true);
    let again = list(&fake, &t, ALICE_EMAIL, "singleEvents=true").json();
    assert_eq!(again["items"][0]["attendees"].as_array().unwrap().len(), 3);
}

#[test]
fn free_busy_reports_timed_opaque_events_and_errors_for_a_calendar_the_account_cannot_see() {
    let fake = fake();
    let t = fake.issue_access_token(
        ALICE,
        CLIENT_ID,
        &[fabric_google_fake::SCOPE_FREEBUSY],
        3600,
    );
    fake.put_event(
        ALICE_EMAIL,
        timed(
            "a",
            "2026-11-18T10:00:00+01:00",
            "2026-11-18T11:00:00+01:00",
        ),
    );
    fake.put_event(
        ALICE_EMAIL,
        timed("b", "2026-11-18T09:30:00Z", "2026-11-18T12:00:00Z"),
    );
    let mut free = timed("c", "2026-11-18T13:00:00Z", "2026-11-18T14:00:00Z");
    free["transparency"] = json!("transparent");
    fake.put_event(ALICE_EMAIL, free);
    fake.put_event(
        ALICE_EMAIL,
        json!({"id": "d", "start": {"date": "2026-11-18"}, "end": {"date": "2026-11-19"}}),
    );
    fake.put_event(
        SHARED,
        timed("s", "2026-11-18T07:00:00Z", "2026-11-18T08:00:00Z"),
    );
    let body = json!({
        "timeMin": "2026-11-18T08:00:00Z", "timeMax": "2026-11-18T11:00:00Z", "timeZone": "UTC",
        "items": [{"id": ALICE_EMAIL}, {"id": SHARED}, {"id": BOB_EMAIL}],
    });
    let answer = api(&fake, &t, "POST", "/freeBusy", &[], Some(&body)).json();
    assert_eq!(
        answer["calendars"][ALICE_EMAIL]["busy"],
        json!([{"start": "2026-11-18T09:00:00Z", "end": "2026-11-18T12:00:00Z"}]),
        "merged, not cut to the window"
    );
    assert_eq!(
        answer["calendars"][SHARED]["busy"],
        json!([]),
        "touching the window is outside"
    );
    assert_eq!(
        answer["calendars"][BOB_EMAIL]["errors"][0]["reason"],
        "notFound"
    );
    fake.with_state(|s| s.set_free_busy_merge(false));
    let unmerged = api(&fake, &t, "POST", "/freeBusy", &[], Some(&body)).json();
    assert_eq!(
        unmerged["calendars"][ALICE_EMAIL]["busy"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let many: Vec<Value> = (0..51).map(|i| json!({"id": format!("c{i}")})).collect();
    let too_many = json!({"timeMin": "2026-11-18T08:00:00Z", "timeMax": "2026-11-18T11:00:00Z", "items": many});
    assert_eq!(
        api(&fake, &t, "POST", "/freeBusy", &[], Some(&too_many)).status,
        400
    );
}

#[test]
fn the_calendar_list_pages_and_filters_by_role_hidden_and_deleted() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.add_calendar(ALICE, CalendarEntry::new("hidden@x", "writer").hidden());
    fake.add_calendar(ALICE, CalendarEntry::new("gone@x", "writer").deleted());
    fake.add_calendar(
        ALICE,
        CalendarEntry::new("fb@x", "freeBusyReader").summary_override("Mine"),
    );
    fake.add_calendar(ALICE, CalendarEntry::new("odd/id", "someNewRole"));
    let path = "/users/me/calendarList";
    let default = api(&fake, &t, "GET", path, &[], None);
    assert_eq!(ids(&default), vec![ALICE_EMAIL, SHARED, "fb@x", "odd/id"]);
    let first = &default.json()["items"][0];
    assert_eq!(
        (first["primary"].clone(), first["timeZone"].clone()),
        (json!(true), json!("Europe/Berlin"))
    );
    assert_eq!(default.json()["items"][2]["summaryOverride"], "Mine");
    let readers = api(
        &fake,
        &t,
        "GET",
        &format!("{path}?minAccessRole=reader&showHidden=true"),
        &[],
        None,
    );
    assert_eq!(
        ids(&readers),
        vec![ALICE_EMAIL, SHARED, "hidden@x", "odd/id"]
    );
    let deleted = api(
        &fake,
        &t,
        "GET",
        &format!("{path}?showDeleted=true&maxResults=250"),
        &[],
        None,
    );
    assert!(ids(&deleted).contains(&"gone@x".to_string()));
    let paged = api(&fake, &t, "GET", &format!("{path}?maxResults=2"), &[], None);
    let next = paged.json()["nextPageToken"].as_str().unwrap().to_string();
    let rest = api(
        &fake,
        &t,
        "GET",
        &format!("{path}?maxResults=2&pageToken={}", enc(&next)),
        &[],
        None,
    );
    assert_eq!(ids(&rest), vec!["fb@x", "odd/id"]);
}

#[test]
fn a_set_calendar_list_page_size_fills_each_page_with_fewer_than_asked() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.add_calendar(ALICE, CalendarEntry::new("fb@x", "freeBusyReader"));
    fake.with_state(|s| s.set_calendar_list_page_size(Some(2)));
    let path = "/users/me/calendarList";
    let first = api(
        &fake,
        &t,
        "GET",
        &format!("{path}?maxResults=250"),
        &[],
        None,
    );
    assert_eq!(ids(&first), vec![ALICE_EMAIL, SHARED]);
    let next = first.json()["nextPageToken"].as_str().unwrap().to_string();
    let rest = api(
        &fake,
        &t,
        "GET",
        &format!("{path}?maxResults=250&pageToken={}", enc(&next)),
        &[],
        None,
    );
    assert_eq!(ids(&rest), vec!["fb@x"]);
    assert!(rest.json().get("nextPageToken").is_none());
    // A request that asks for fewer than the set size gets what it asked for.
    let one = api(&fake, &t, "GET", &format!("{path}?maxResults=1"), &[], None);
    assert_eq!(ids(&one), vec![ALICE_EMAIL]);
    fake.with_state(|s| s.set_calendar_list_page_size(None));
    let whole = api(
        &fake,
        &t,
        "GET",
        &format!("{path}?maxResults=250"),
        &[],
        None,
    );
    assert_eq!(ids(&whole), vec![ALICE_EMAIL, SHARED, "fb@x"]);
}

#[test]
fn a_call_needs_a_valid_token_with_its_scope_and_a_listed_calendar_and_a_writer_role() {
    let fake = fake();
    let alice = token(&fake, ALICE);
    let bob = token(&fake, BOB);
    let none = request(
        fake.port(),
        "GET",
        "/calendar/v3/users/me/calendarList",
        &[],
        b"",
    );
    assert_eq!(
        (none.status, none.reason().as_deref()),
        (401, Some("required"))
    );
    assert!(none.header("WWW-Authenticate").is_some());
    let unknown = api(
        &fake,
        "ya29.nobody",
        "GET",
        "/users/me/calendarList",
        &[],
        None,
    );
    assert_eq!(unknown.reason().as_deref(), Some("authError"));

    let bobs_calendar = list(&fake, &alice, BOB_EMAIL, "singleEvents=true");
    assert_eq!(
        (bobs_calendar.status, bobs_calendar.reason().as_deref()),
        (404, Some("notFound"))
    );
    assert_eq!(list(&fake, &bob, BOB_EMAIL, "").status, 200);

    let write = api(
        &fake,
        &alice,
        "POST",
        &events_path(SHARED),
        &[],
        Some(&timed(
            "abcdef",
            "2026-11-18T10:00:00Z",
            "2026-11-18T11:00:00Z",
        )),
    );
    assert_eq!(
        (write.status, write.reason().as_deref()),
        (403, Some("requiredAccessLevel"))
    );
    fake.with_state(|s| s.set_access_role(ALICE, SHARED, "writerWithoutPrivateAccess"));
    let write = api(
        &fake,
        &alice,
        "POST",
        &events_path(SHARED),
        &[],
        Some(&timed(
            "abcdef",
            "2026-11-18T10:00:00Z",
            "2026-11-18T11:00:00Z",
        )),
    );
    assert_eq!(write.status, 200, "this role may write (G-DOC)");

    let events_only = fake.issue_access_token(ALICE, CLIENT_ID, &[SCOPE_EVENTS], 3600);
    let body =
        json!({"timeMin": "2026-11-18T08:00:00Z", "timeMax": "2026-11-18T11:00:00Z", "items": []});
    let fb = api(&fake, &events_only, "POST", "/freeBusy", &[], Some(&body));
    assert_eq!(
        (fb.status, fb.reason().as_deref()),
        (403, Some("insufficientPermissions"))
    );
    let cl = api(
        &fake,
        &events_only,
        "GET",
        "/users/me/calendarList",
        &[],
        None,
    );
    assert_eq!(cl.status, 403);

    fake.with_state(|s| s.remove_calendar(ALICE, SHARED));
    assert_eq!(
        list(&fake, &alice, SHARED, "").status,
        404,
        "a vanished calendar"
    );
    assert_eq!(fake.log().calendar().entries()[1].token_account, None);
    assert_eq!(
        fake.log().calendar().entries()[2].token_account.as_deref(),
        Some(ALICE)
    );
}

#[test]
fn a_private_event_shows_a_reader_only_its_time() {
    let fake = fake();
    let alice = token(&fake, ALICE);
    let mut event = timed("priv1", "2026-11-18T10:00:00Z", "2026-11-18T11:00:00Z");
    event["visibility"] = json!("private");
    event["description"] = json!("secret");
    fake.put_event(SHARED, event);
    let seen = api(
        &fake,
        &alice,
        "GET",
        &event_path(SHARED, "priv1"),
        &[],
        None,
    )
    .json();
    assert!(seen.get("summary").is_none() && seen.get("description").is_none());
    assert_eq!(seen["start"]["dateTime"], "2026-11-18T10:00:00Z");
    let bob = token(&fake, BOB);
    let owner_view = api(&fake, &bob, "GET", &event_path(SHARED, "priv1"), &[], None).json();
    assert_eq!(owner_view["description"], "secret");
}

#[test]
fn the_keyword_primary_names_the_accounts_primary_calendar_and_shows_in_the_log() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(
        ALICE_EMAIL,
        timed("p1", "2026-11-18T10:00:00Z", "2026-11-18T11:00:00Z"),
    );
    assert!(!fake.log().any_path_contains("primary"));
    let answer = list(&fake, &t, "primary", "");
    assert_eq!(ids(&answer), vec!["p1"]);
    assert!(fake.log().any_path_contains("primary"));
}

#[test]
fn a_late_visible_insert_answers_404_to_its_first_reads() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.arm(
        fabric_google_fake::Fault::on(
            "POST",
            "/calendar/v3/calendars",
            fabric_google_fake::FaultAction::HideInsertForReads(2),
        )
        .and(fabric_google_fake::FaultAction::apply_then_503()),
    );
    let event = timed("late0", "2026-11-18T10:00:00Z", "2026-11-18T11:00:00Z");
    let answer = api(
        &fake,
        &t,
        "POST",
        &events_path(ALICE_EMAIL),
        &[],
        Some(&event),
    );
    assert_eq!(
        (answer.status, answer.reason().as_deref()),
        (503, Some("backendError"))
    );
    let path = event_path(ALICE_EMAIL, "late0");
    assert_eq!(api(&fake, &t, "GET", &path, &[], None).status, 404);
    assert!(ids(&list(&fake, &t, ALICE_EMAIL, "")).is_empty());
    assert_eq!(api(&fake, &t, "GET", &path, &[], None).status, 404);
    assert_eq!(api(&fake, &t, "GET", &path, &[], None).status, 200);
    assert_eq!(fake.events(ALICE_EMAIL).len(), 1, "one event");
}
