//! Refresh, revocation and token-store failures against the fake (design 3.8, 3.9, section
//! 4): g14, g15, g17.

mod common;
mod oauth_common;

use std::sync::Arc;

use common::ScriptedTokenStore;
use fabric_google_calendar::http::{BodyBound, Exchange, Method, Request, Target};
use fabric_google_calendar::oauth::{
    CancelReason, ConnectJob, ConnectOutcome, GoogleAccess, Revocation, revoke,
};
use fabric_google_calendar::{
    AccessCache, AccessFailure, AccessTokens, AuthLostCause, Clock, Secret, TokenStore,
    TokenStoreError,
};
use fabric_google_fake::{Decision, Fault, FaultAction, TokenFault};
use oauth_common::*;

/// A rig with account 1 connected as A.
fn connected() -> Rig {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    rig
}

fn refresh_fault(action: FaultAction) -> Fault {
    Fault::token(action).grant_type("refresh_token")
}

#[test]
fn g14_a_cached_token_is_used_and_one_near_its_end_is_refreshed_before_the_call() {
    let rig = connected();
    let mut access = rig.access();
    let cached = rig
        .cache
        .get_valid(&account(1), rig.clock.now_ms(), 0)
        .expect("cached");
    let token = access
        .access_token(&account(1), rig.deadline())
        .expect("a token");
    assert_eq!(token.expose(), cached.expose());
    assert_eq!(rig.token_requests("refresh_token"), 0);

    // 3599 s of life: with 61 s left it is used, with 60 s left it is refreshed (3.8).
    rig.advance_ms((3599 - 61) * 1000);
    let same = access
        .access_token(&account(1), rig.deadline())
        .expect("a token");
    assert_eq!(same.expose(), cached.expose(), "61 s left is enough");
    rig.advance_ms(1000);
    let fresh = access
        .access_token(&account(1), rig.deadline())
        .expect("a token");
    assert_ne!(fresh.expose(), cached.expose());
    assert_eq!(rig.token_requests("refresh_token"), 1);
    assert!(
        rig.fake
            .with_state(|s| s.access_token_valid(fresh.expose()))
    );
    let refresh = rig
        .fake
        .log()
        .token_requests()
        .last()
        .cloned()
        .expect("a refresh");
    assert_eq!(refresh.form_value("client_id"), Some(CLIENT_ID));
    assert_eq!(refresh.form_value("client_secret"), Some(CLIENT_SECRET));
    assert_eq!(
        refresh.form_value("refresh_token").map(str::to_owned),
        rig.store.token(&account(1))
    );
    rig.check_log();
}

#[test]
fn g14_a_401_from_a_calendar_call_drops_the_cached_token_and_the_next_call_refreshes() {
    let rig = connected();
    let mut access = rig.access();
    let token = access
        .access_token(&account(1), rig.deadline())
        .expect("a token");
    rig.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::ExpiredAccessToken,
    ));
    let request = Request::new(Method::Get, Target::CalendarList, BodyBound::Large)
        .bearer(&token)
        .expect("a bearer");
    let Exchange::Answer(answer) = rig.transport.send(request, rig.deadline()) else {
        panic!("no answer");
    };
    assert_eq!(answer.status(), 401);
    // The Calendar call's classification (6.10: Transient) is `calendar`'s; the token part
    // drops the cached token on its word.
    access.drop_cached(&account(1));
    assert!(
        rig.cache
            .get_valid(&account(1), rig.clock.now_ms(), 0)
            .is_none()
    );
    let fresh = access
        .access_token(&account(1), rig.deadline())
        .expect("a token");
    assert_ne!(fresh.expose(), token.expose());
    assert_eq!(rig.token_requests("refresh_token"), 1);
    assert!(
        rig.store.token(&account(1)).is_some(),
        "the refresh token stays"
    );
    rig.check_log();
}

#[test]
fn g14_a_refusal_of_the_refresh_token_is_auth_lost_and_forgets_the_account() {
    let cases = [
        (FaultAction::invalid_grant(), AuthLostCause::GrantEnded),
        (
            FaultAction::OAuthError {
                status: 400,
                error: "invalid_grant".to_owned(),
                subtype: Some("invalid_rapt".to_owned()),
            },
            AuthLostCause::GrantEnded,
        ),
        (
            FaultAction::admin_policy_enforced(),
            AuthLostCause::GrantEnded,
        ),
        (FaultAction::invalid_client(), AuthLostCause::ClientRejected),
        (
            FaultAction::oauth_error(400, "unauthorized_client"),
            AuthLostCause::ClientRejected,
        ),
    ];
    for (action, cause) in cases {
        let label = format!("{action:?}");
        let rig = connected();
        let mut access = rig.access();
        rig.cache.drop_account(&account(1));
        rig.cache.put(
            &account(1),
            Secret::from("ya29.nearly-over"),
            rig.clock.now_ms() + 59_000,
        );
        rig.fake.arm(refresh_fault(action));
        assert_eq!(
            access.access_token(&account(1), rig.deadline()).err(),
            Some(AccessFailure::AuthLost(cause)),
            "{label}"
        );
        assert!(rig.store.token(&account(1)).is_none(), "{label}");
        assert!(rig.cache.get_valid(&account(1), 0, 0).is_none(), "{label}");
        assert_eq!(
            rig.token_requests("refresh_token"),
            1,
            "{label}: one attempt"
        );
        assert!(rig.fake.log().revocations().is_empty());
        rig.check_log();
    }
}

#[test]
fn g14_a_new_refresh_token_in_the_answer_replaces_the_stored_one() {
    let rig = connected();
    let old = rig.store.token(&account(1)).expect("stored");
    rig.cache.drop_account(&account(1));
    rig.fake.arm(refresh_fault(FaultAction::Token(
        TokenFault::RotateRefreshToken,
    )));
    rig.access()
        .access_token(&account(1), rig.deadline())
        .expect("a token");
    let new = rig.store.token(&account(1)).expect("stored");
    assert_ne!(new, old);
    assert!(rig.fake.with_state(|s| s.refresh_token_valid(&new)));
    assert!(!rig.fake.with_state(|s| s.refresh_token_valid(&old)));
    rig.check_log();
}

#[test]
fn g14_other_scopes_in_a_refresh_answer_are_auth_lost() {
    for scope in [
        "openid email https://www.googleapis.com/auth/calendar.events",
        "openid email https://www.googleapis.com/auth/calendar.calendarlist.readonly \
         https://www.googleapis.com/auth/calendar.freebusy https://www.googleapis.com/auth/calendar.events \
         https://www.googleapis.com/auth/calendar",
    ] {
        let rig = connected();
        rig.cache.drop_account(&account(1));
        rig.fake
            .arm(refresh_fault(FaultAction::Token(TokenFault::Scope(
                scope.to_owned(),
            ))));
        assert_eq!(
            rig.access().access_token(&account(1), rig.deadline()).err(),
            Some(AccessFailure::AuthLost(AuthLostCause::GrantEnded))
        );
        assert!(rig.store.token(&account(1)).is_none());
        rig.check_log();
    }
}

#[test]
fn g14_any_other_answer_or_none_is_transient_and_keeps_the_stored_token() {
    for action in [
        FaultAction::error(503, "backendError"),
        FaultAction::oauth_error(400, "invalid_request"),
        FaultAction::oauth_error(500, "internal_failure"),
        FaultAction::CloseWithoutApplying,
        FaultAction::NotJson,
        FaultAction::Gzip,
        FaultAction::Token(TokenFault::DropMember("expires_in".to_owned())),
        FaultAction::Redirect("http://127.0.0.1:1/".to_owned()),
    ] {
        let label = format!("{action:?}");
        let rig = connected();
        let stored = rig.store.token(&account(1));
        rig.cache.drop_account(&account(1));
        rig.fake.arm(refresh_fault(action));
        assert_eq!(
            rig.access().access_token(&account(1), rig.deadline()).err(),
            Some(AccessFailure::Transient),
            "{label}"
        );
        assert_eq!(rig.store.token(&account(1)), stored, "{label}");
        rig.check_log();
    }
}

#[test]
fn g14_no_stored_refresh_token_is_auth_lost_and_sends_nothing() {
    let rig = Rig::new();
    assert_eq!(
        rig.access().access_token(&account(1), rig.deadline()).err(),
        Some(AccessFailure::AuthLost(AuthLostCause::GrantEnded))
    );
    assert!(rig.fake.log().is_empty());
}

#[test]
fn g15_revocation_200_and_invalid_token_are_revoked_and_anything_else_is_not_confirmed() {
    let rig = connected();
    let stored = rig.store.token(&account(1)).expect("stored");
    assert_eq!(
        revoke(&rig.transport, &rig.store, &account(1), rig.deadline()),
        Revocation::Revoked
    );
    assert_eq!(
        rig.store.token(&account(1)),
        Some(stored.clone()),
        "the caller deletes it"
    );
    assert!(!rig.fake.with_state(|s| s.refresh_token_valid(&stored)));
    let request = rig
        .fake
        .log()
        .revocations()
        .last()
        .cloned()
        .expect("a revocation");
    assert_eq!(request.form_value("token"), Some(stored.as_str()));
    // Google no longer knows it: 400 invalid_token.
    assert_eq!(
        revoke(&rig.transport, &rig.store, &account(1), rig.deadline()),
        Revocation::Revoked
    );
    for action in [
        FaultAction::oauth_error(400, "invalid_request"),
        FaultAction::error(503, "backendError"),
        FaultAction::oauth_error(401, "invalid_token"),
        FaultAction::CloseWithoutApplying,
        FaultAction::Redirect("http://127.0.0.1:1/".to_owned()),
    ] {
        let label = format!("{action:?}");
        rig.fake.arm(Fault::on("POST", "/revoke", action));
        assert_eq!(
            revoke(&rig.transport, &rig.store, &account(1), rig.deadline()),
            Revocation::NotConfirmed,
            "{label}"
        );
    }
    // No stored token: nothing is sent.
    let before = rig.fake.log().revocations().len();
    assert_eq!(
        revoke(&rig.transport, &rig.store, &account(2), rig.deadline()),
        Revocation::NotConfirmed
    );
    assert_eq!(rig.fake.log().revocations().len(), before);
    assert_eq!(before, 7, "one attempt each");
    rig.check_log();
}

#[test]
fn g17_token_store_failures_are_transient_when_locked_and_rejected_otherwise_and_send_nothing() {
    let rig = Rig::new();
    let store = ScriptedTokenStore::new();
    let mut writer = store.clone();
    writer
        .write(&account(1), &Secret::from("1//never-sent"))
        .expect("write");
    let mut access = GoogleAccess::new(
        Arc::clone(&rig.transport),
        Arc::clone(&rig.client),
        store.clone(),
        AccessCache::new(),
        Arc::new(rig.clock.clone()),
    );
    for (error, expected) in [
        (TokenStoreError::Locked, AccessFailure::Transient),
        (
            TokenStoreError::Refused,
            AccessFailure::Store(TokenStoreError::Refused),
        ),
        (
            TokenStoreError::Failed { status: -25_293 },
            AccessFailure::Store(TokenStoreError::Failed { status: -25_293 }),
        ),
    ] {
        store.fail_with(Some(error));
        assert_eq!(
            access.access_token(&account(1), rig.deadline()).err(),
            Some(expected)
        );
        assert!(rig.fake.log().is_empty(), "the fake saw no request");
        // Revocation reads the store too: nothing is sent.
        assert_eq!(
            revoke(&rig.transport, &store, &account(1), rig.deadline()),
            Revocation::NotConfirmed
        );
        assert!(rig.fake.log().is_empty());
    }
}

#[test]
fn g17_a_sign_in_whose_token_store_write_fails_is_cancelled_token_store_and_keeps_nothing() {
    let rig = Rig::new();
    for error in [
        TokenStoreError::Locked,
        TokenStoreError::Refused,
        TokenStoreError::Failed { status: -1 },
    ] {
        let mut store = ScriptedTokenStore::new();
        store.fail_with(Some(error));
        let (pending, address) = rig.start(ConnectJob::new_account(account(1), []));
        let cache = AccessCache::new();
        let outcome = std::thread::scope(|scope| {
            let visit = scope.spawn(|| {
                rig.fake
                    .browser()
                    .visit(address.expose(), Decision::allow())
                    .expect("visit")
            });
            let outcome = pending.wait(&mut store, &cache);
            visit.join().expect("the browser thread ends");
            outcome
        });
        assert_eq!(
            outcome,
            ConnectOutcome::Cancelled(CancelReason::TokenStore(error))
        );
        assert!(cache.get_valid(&account(1), 0, 0).is_none());
        store.fail_with(None);
        assert!(store.read(&account(1)).expect("read").is_none());
    }
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}
