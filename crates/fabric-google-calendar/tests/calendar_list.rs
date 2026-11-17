//! The calendar list (6.2) and two accounts side by side (6.1): g18, g19.

mod calendar_common;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use calendar_common::{CLIENT, World, ZONE};
use fabric_desktop_contracts::common::{CalendarAccess, FailureCode};
use fabric_google_calendar::outcome::ReadFailure;
use fabric_google_fake::{CalendarEntry, Fault, FaultAction};
use serde_json::json;

/// g18: pages, the roles of 6.2, names, hidden entries listed, an unknown role and an unusable
/// id left out.
#[test]
fn the_calendar_list_reads_every_page_maps_the_roles_lists_hidden_entries_and_leaves_out_the_unusable()
 {
    let (mut world, alice) = World::with_alice();
    for (id, role) in [
        ("writer@example.com", "writer"),
        ("wwpa@example.com", "writerWithoutPrivateAccess"),
        ("reader@example.com", "reader"),
        ("fb@example.com", "freeBusyReader"),
        ("odd-role@example.com", "none"),
        ("..", "owner"),
        ("a/b@example.com", "owner"),
    ] {
        world
            .fake
            .add_calendar(&alice.subject, CalendarEntry::new(id, role).time_zone(ZONE));
    }
    world.fake.add_calendar(
        &alice.subject,
        CalendarEntry::new("hidden@example.com", "reader")
            .hidden()
            .summary_override("Mine"),
    );
    world.fake.add_calendar(
        &alice.subject,
        CalendarEntry::new("noname@example.com", "reader").summary(None),
    );
    world.fake.add_calendar(
        &alice.subject,
        CalendarEntry::new("long@example.com", "reader").summary(Some(&"\u{e9}".repeat(300))),
    );
    // The first page is scripted with two entries and a page token that the fake reads as
    // "from the third entry on", so the listing must follow it.
    let first_page = json!({
        "kind": "calendar#calendarList",
        "items": [
            {"id": alice.email, "summary": alice.email, "accessRole": "owner", "primary": true, "timeZone": ZONE},
            {"id": "writer@example.com", "summary": "Team", "accessRole": "writer", "timeZone": ZONE},
        ],
        "nextPageToken": URL_SAFE_NO_PAD.encode("fake-page:2"),
    });
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/users/me/calendarList",
        FaultAction::Json(200, first_page),
    ));
    let deadline = world.deadline();
    let list = world
        .as_account(&alice, |g| g.list_calendars(&alice.id, deadline))
        .expect("a list");

    let requests = world.fake.log();
    assert_eq!(requests.len(), 2, "two pages");
    for request in requests.entries() {
        assert_eq!(request.query_value("maxResults"), Some("250"));
        assert_eq!(request.query_value("minAccessRole"), Some("freeBusyReader"));
        assert_eq!(request.query_value("showHidden"), Some("true"));
        assert_eq!(request.query_value("showDeleted"), None);
    }
    assert_eq!(requests.entries()[0].query_value("pageToken"), None);
    assert!(requests.entries()[1].query_value("pageToken").is_some());

    let ids: Vec<&str> = list.iter().map(|e| e.provider_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "alice@example.com",
            "writer@example.com",
            "wwpa@example.com",
            "reader@example.com",
            "fb@example.com",
            "hidden@example.com",
            "noname@example.com",
            "long@example.com",
        ]
    );
    let access: Vec<CalendarAccess> = list.iter().map(|e| e.access).collect();
    assert_eq!(
        access,
        [
            CalendarAccess::Owner,
            CalendarAccess::Writer,
            CalendarAccess::Reader,
            CalendarAccess::Reader,
            CalendarAccess::FreeBusyReader,
            CalendarAccess::Reader,
            CalendarAccess::Reader,
            CalendarAccess::Reader,
        ]
    );
    let writable: Vec<bool> = list.iter().map(|e| e.writable()).collect();
    assert_eq!(
        writable,
        [true, true, false, false, false, false, false, false]
    );
    assert!(list[0].primary && list.iter().skip(1).all(|e| !e.primary));
    assert_eq!(list[0].time_zone.as_str(), ZONE);
    assert_eq!(list[1].name.as_str(), "Team");
    assert_eq!(list[5].name.as_str(), "Mine");
    assert_eq!(list[6].name.as_str(), "(no name)");
    assert_eq!(list[7].name.as_str(), "\u{e9}".repeat(200));
    world.check_log();
}

/// g18: more than 250 calendars in all fails the listing; exactly 250 does not.
#[test]
fn a_calendar_list_past_250_calendars_fails_and_250_is_listed() {
    let (mut world, alice) = World::with_alice();
    for n in 1..250 {
        world.fake.add_calendar(
            &alice.subject,
            CalendarEntry::new(&format!("c{n}@group.example.com"), "reader"),
        );
    }
    let deadline = world.deadline();
    let list = world
        .as_account(&alice, |g| g.list_calendars(&alice.id, deadline))
        .expect("250 are listed");
    assert_eq!(list.len(), 250);
    world.fake.add_calendar(
        &alice.subject,
        CalendarEntry::new("c250@group.example.com", "reader"),
    );
    let failure = world
        .as_account(&alice, |g| g.list_calendars(&alice.id, deadline))
        .expect_err("251 fail");
    let ReadFailure::Rejected {
        code, explanation, ..
    } = failure
    else {
        panic!("rejected");
    };
    assert_eq!(code, FailureCode::ProviderRejected);
    assert!(explanation.as_str().contains("250"));
    world.check_log();
}

/// g19: each call carries its own account's token; A's token on a calendar only B has is
/// refused by the fake as Google would (404, a calendar unknown to A).
#[test]
fn two_accounts_each_use_their_own_token_and_one_cannot_reach_the_others_calendar() {
    let (mut world, alice) = World::with_alice();
    let bob = world.person("bob");
    let bobs = world.calendar(
        &bob,
        CalendarEntry::new("bob-team@group.example.com", "owner").time_zone(ZONE),
    );
    let deadline = world.deadline();
    let a = world
        .as_account(&alice, |g| g.list_calendars(&alice.id, deadline))
        .expect("a list");
    let b = world
        .as_account(&bob, |g| g.list_calendars(&bob.id, deadline))
        .expect("a list");
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 2);
    let window = calendar_common::window();
    world
        .as_account(&bob, |g| {
            g.free_busy(&bob.id, std::slice::from_ref(&bobs), &window, deadline)
        })
        .expect("bob reads his calendar");

    // Alice's token on Bob's calendar: an order that names Bob's calendar under Alice's account
    // is refused before any request (the target belongs to Bob) ...
    let before = world.fake.log().len();
    let refused = world.as_account(&alice, |g| {
        g.free_busy(&alice.id, std::slice::from_ref(&bobs), &window, deadline)
    });
    assert!(matches!(
        refused,
        Err(ReadFailure::Rejected {
            code: FailureCode::InvalidArguments,
            ..
        })
    ));
    assert_eq!(world.fake.log().len(), before);
    // ... and a target that claims Alice's account for Bob's Google calendar reaches the fake
    // with Alice's token, which the fake refuses as Google would.
    let mut forged = bobs.clone();
    forged.calendar.account_id = alice.id.clone();
    let query = calendar_common::query(
        std::slice::from_ref(&forged),
        &[fabric_desktop_contracts::common::EventField::Time],
    );
    let refused = world.as_account(&alice, |g| {
        g.read_events(&alice.id, &query, deadline, &|| true)
    });
    let Err(ReadFailure::Rejected { code, .. }) = refused else {
        panic!("rejected");
    };
    assert_eq!(code, FailureCode::ProviderRejected);
    let last = world.fake.log().last().cloned().expect("a request");
    assert_eq!(last.answered, Some(404));
    assert_eq!(last.token_account.as_deref(), Some(alice.subject.as_str()));
    assert_ne!(world.tokens.token(&alice.id), world.tokens.token(&bob.id));
    let _ = CLIENT;
    world.check_log();
}
