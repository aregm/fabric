//! The real access-token part together with a Calendar call (design 3.8, 6.10): a 401 from a
//! Calendar call drops the cached token in the adapter itself, and the next call refreshes.

#![cfg(debug_assertions)]

mod oauth_common;

use std::sync::Arc;

use fabric_google_calendar::Clock;
use fabric_google_calendar::calendar::Google;
use fabric_google_calendar::outcome::ReadFailure;
use fabric_google_fake::{Decision, Fault, FaultAction};
use oauth_common::*;

#[test]
fn g14_a_401_from_a_calendar_call_is_transient_drops_the_cached_token_and_the_next_call_refreshes()
{
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let cached = rig
        .cache
        .get_valid(&account(1), rig.clock.now_ms(), 0)
        .expect("cached at sign-in");
    let mut google = Google::new(Arc::clone(&rig.transport), Box::new(rig.access()));

    rig.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::ExpiredAccessToken,
    ));
    let first = google.list_calendars(&account(1), rig.deadline());
    assert!(
        matches!(first, Err(ReadFailure::Transient { .. })),
        "{first:?}"
    );
    assert!(
        rig.cache
            .get_valid(&account(1), rig.clock.now_ms(), 0)
            .is_none(),
        "the adapter dropped the cached token"
    );
    assert_eq!(rig.token_requests("refresh_token"), 0);

    google
        .list_calendars(&account(1), rig.deadline())
        .expect("the next call refreshes and succeeds");
    assert_eq!(rig.token_requests("refresh_token"), 1);
    let fresh = rig
        .cache
        .get_valid(&account(1), rig.clock.now_ms(), 0)
        .expect("cached again");
    assert_ne!(fresh.expose(), cached.expose());
    assert!(
        rig.store.token(&account(1)).is_some(),
        "the refresh token stays"
    );
    rig.check_log();
}
