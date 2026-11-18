//! Sign-in against the fake Google (design 3.2 to 3.7): g01 to g06, g12, g13.

#![cfg(debug_assertions)]

mod oauth_common;

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fabric_google_calendar::oauth::listener::PAGE_TEXT;
use fabric_google_calendar::oauth::{
    CancelReason, ConnectJob, ConnectOutcome, IdentityProblem, Scope,
};
use fabric_google_calendar::{AccessTokens, Clock, TokenStore};
use fabric_google_fake::{
    Account, Decision, Fault, FaultAction, IdTokenFault, SCOPE_EMAIL, SCOPE_EVENTS, SCOPE_FREEBUSY,
    SCOPE_PROFILE, TokenFault,
};
use oauth_common::*;
use sha2::{Digest, Sha256};

#[test]
fn g01_a_full_sign_in_ends_connected_with_subject_verified_address_five_scopes_and_the_stored_refresh_token()
 {
    let rig = Rig::new();
    let (outcome, visit, _) =
        rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
    let ConnectOutcome::Connected {
        subject,
        email,
        scopes,
        refresh_token_ends_at,
    } = outcome
    else {
        panic!("not connected: {outcome:?}");
    };
    assert_eq!(subject, SUB_A);
    assert_eq!(email.as_str(), "ann@example.com");
    assert_eq!(scopes, Scope::ALL.to_vec());
    assert_eq!(refresh_token_ends_at, None);

    let stored = rig
        .store
        .token(&account(1))
        .expect("a stored refresh token");
    assert_eq!(
        rig.fake.with_state(|s| s.valid_refresh_tokens(SUB_A)),
        vec![stored]
    );
    assert_eq!(rig.store.writes(), 1);
    let page = visit.listener_answer().expect("the listener answered");
    assert_eq!(page.status, 200);
    assert!(page.text().contains(PAGE_TEXT));
    // The access token of the answer is kept, and it works at the fake.
    let access = rig
        .cache
        .get_valid(&account(1), rig.clock.now_ms(), 60_000)
        .expect("a cached access token");
    assert!(
        rig.fake
            .with_state(|s| s.access_token_valid(access.expose()))
    );
    let log = rig.fake.log();
    assert_eq!(log.token_requests().len(), 1);
    assert_eq!(log.key_set_fetches().len(), 1);
    assert!(log.revocations().is_empty());
    let code = visit.code().expect("a code");
    assert_eq!(rig.fake.with_state(|s| s.code_used(&code)), Some(true));
    rig.check_log();
}

#[test]
fn g02_the_authorization_address_has_exactly_the_parameters_of_3_2_1_and_fresh_43_character_secrets()
 {
    let rig = Rig::new();
    let mut seen = Vec::new();
    for n in 1..=2 {
        let (_, _, address) = rig.sign_in(
            ConnectJob::new_account(account(n), []),
            Decision::allow().sign_in_as(if n == 1 { SUB_A } else { SUB_B }),
        );
        let pairs = query_pairs(address.expose());
        let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "client_id",
                "redirect_uri",
                "response_type",
                "scope",
                "code_challenge",
                "code_challenge_method",
                "state",
                "nonce",
                "prompt"
            ]
        );
        let value = |name: &str| query_value(address.expose(), name).expect("present");
        assert!(address.expose().starts_with(&format!(
            "http://127.0.0.1:{}/o/oauth2/v2/auth?",
            rig.fake.port()
        )));
        assert_eq!(value("client_id"), CLIENT_ID);
        let redirect = value("redirect_uri");
        assert!(redirect.starts_with("http://127.0.0.1:"));
        let port = redirect.trim_start_matches("http://127.0.0.1:");
        assert!(port.parse::<u16>().is_ok(), "no path after the port");
        assert_eq!(value("response_type"), "code");
        assert_eq!(
            value("scope"),
            "openid email https://www.googleapis.com/auth/calendar.calendarlist.readonly \
             https://www.googleapis.com/auth/calendar.freebusy https://www.googleapis.com/auth/calendar.events"
        );
        assert_eq!(value("code_challenge_method"), "S256");
        assert_eq!(value("prompt"), "select_account");

        let token_request = rig
            .fake
            .log()
            .token_requests()
            .last()
            .cloned()
            .expect("a token request");
        let verifier = token_request
            .form_value("code_verifier")
            .expect("a verifier")
            .to_owned();
        assert_eq!(
            token_request.form_value("redirect_uri"),
            Some(redirect.as_str())
        );
        assert_eq!(
            value("code_challenge"),
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        for secret in [&verifier, &value("state"), &value("nonce")] {
            assert_eq!(secret.len(), 43);
            assert!(
                secret
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            );
        }
        seen.extend([verifier, value("state"), value("nonce")]);
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "verifier, state and nonce differ per flow"
    );

    // A reconnect hints the stored subject and does not prompt.
    let (_, _, address) = rig.sign_in(ConnectJob::reconnect(account(1), SUB_A), Decision::allow());
    let pairs = query_pairs(address.expose());
    assert_eq!(
        pairs.last().map(|(n, v)| (n.as_str(), v.as_str())),
        Some(("login_hint", SUB_A))
    );
    assert!(!pairs.iter().any(|(n, _)| n == "prompt"));
    assert_eq!(pairs.len(), 9);
    rig.check_log();
}

#[test]
fn g03_denied_consent_is_consent_denied_stores_nothing_and_makes_no_token_request() {
    let rig = Rig::new();
    let (outcome, visit, _) =
        rig.sign_in(ConnectJob::new_account(account(1), []), Decision::deny());
    assert_eq!(outcome, ConnectOutcome::ConsentDenied);
    assert_eq!(visit.listener_answer().expect("answered").status, 200);
    assert!(rig.store.inner.is_empty());
    assert_eq!(rig.store.writes(), 0);
    assert!(rig.fake.log().token_requests().is_empty());
    rig.check_log();
}

#[test]
fn g04_a_missing_calendar_scope_or_address_scope_is_partial_grant_stores_nothing_and_revokes_nothing()
 {
    let rig = Rig::new();
    for (untick, missing) in [
        (SCOPE_EVENTS, vec![Scope::Events]),
        (SCOPE_FREEBUSY, vec![Scope::FreeBusy]),
        (SCOPE_EMAIL, vec![Scope::Email]),
    ] {
        let (outcome, _, _) = rig.sign_in(
            ConnectJob::new_account(account(1), []),
            Decision::allow().untick(&[untick]),
        );
        assert_eq!(outcome, ConnectOutcome::PartialGrant { missing });
    }
    assert!(rig.store.inner.is_empty());
    assert_eq!(rig.store.writes(), 0);
    assert!(rig.fake.log().revocations().is_empty());
    // What Google issued stays valid at Google: nothing was revoked (PD34).
    assert_eq!(
        rig.fake.with_state(|s| s.valid_refresh_tokens(SUB_A)).len(),
        3
    );
    rig.check_log();
}

#[test]
fn g05_a_scope_beyond_the_five_is_unexpected_scope_stores_nothing_and_revokes_nothing() {
    let rig = Rig::new();
    let (outcome, _, _) = rig.sign_in(
        ConnectJob::new_account(account(1), []),
        Decision::allow().grant_extra(&[SCOPE_PROFILE]),
    );
    assert_eq!(
        outcome,
        ConnectOutcome::Cancelled(CancelReason::UnexpectedScope)
    );
    let (outcome, _, _) = rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
    assert!(outcome.is_connected());
    rig.fake.arm(Fault::token_answer(TokenFault::Scope(
        "openid email https://www.googleapis.com/auth/calendar".to_owned(),
    )));
    let (outcome, _, _) = rig.sign_in(
        ConnectJob::new_account(account(2), []),
        Decision::allow().sign_in_as(SUB_B),
    );
    assert_eq!(
        outcome,
        ConnectOutcome::Cancelled(CancelReason::UnexpectedScope)
    );
    assert!(rig.store.token(&account(2)).is_none());
    assert_eq!(rig.store.writes(), 1);
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}

#[test]
fn g06_a_reconnect_as_another_subject_is_account_mismatch_writes_nothing_and_forgets_the_old_token_without_revoking_it()
 {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let old = rig.store.token(&account(1)).expect("stored");
    let writes = rig.store.writes();

    let (outcome, _, _) = rig.sign_in(
        ConnectJob::reconnect(account(1), SUB_A),
        Decision::allow().sign_in_as(SUB_B),
    );
    assert_eq!(outcome, ConnectOutcome::AccountMismatch);
    assert_eq!(
        rig.store.writes(),
        writes,
        "the token store is never written"
    );
    // Fail closed (3.7): the reconnected account's token is forgotten, with its access token.
    assert!(rig.store.token(&account(1)).is_none());
    assert!(
        rig.cache
            .get_valid(&account(1), rig.clock.now_ms(), 0)
            .is_none()
    );
    // Not revoked: the old token still refreshes at the fake (PD34).
    assert!(rig.fake.with_state(|s| s.refresh_token_valid(&old)));
    assert!(rig.fake.log().revocations().is_empty());
    let mut probe_store = fabric_google_calendar::MemoryTokenStore::new();
    probe_store
        .write(
            &account(9),
            &fabric_google_calendar::Secret::from(old.as_str()),
        )
        .expect("write");
    let mut separate = fabric_google_calendar::oauth::GoogleAccess::new(
        std::sync::Arc::clone(&rig.transport),
        std::sync::Arc::clone(&rig.client),
        probe_store,
        fabric_google_calendar::AccessCache::new(),
        std::sync::Arc::new(rig.clock.clone()),
    );
    assert!(separate.access_token(&account(9), rig.deadline()).is_ok());
    rig.check_log();
}

#[test]
fn g06_a_new_connect_as_a_subject_some_record_has_is_account_mismatch_and_the_existing_account_keeps_working()
 {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let existing = rig.store.token(&account(1)).expect("stored");
    let writes = rig.store.writes();
    let deletes = rig.store.deletes();

    let (outcome, _, _) = rig.sign_in(
        ConnectJob::new_account(account(2), [SUB_A.to_owned()]),
        Decision::allow(),
    );
    assert_eq!(outcome, ConnectOutcome::AccountMismatch);
    assert_eq!(rig.store.writes(), writes);
    assert_eq!(rig.store.deletes(), deletes);
    assert!(rig.store.token(&account(2)).is_none());
    assert_eq!(rig.store.token(&account(1)), Some(existing));
    // The existing account's token still refreshes at the fake.
    rig.cache.drop_account(&account(1));
    let before = rig.token_requests("refresh_token");
    rig.access()
        .access_token(&account(1), rig.deadline())
        .expect("the existing account still refreshes");
    assert_eq!(rig.token_requests("refresh_token"), before + 1);
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}

#[test]
fn g12_an_unverified_address_and_one_the_contract_cannot_hold_do_not_connect() {
    let rig = Rig::new();
    rig.fake
        .add_account(Account::new(SUB_A, EMAIL_A).unverified());
    let (outcome, _, _) = rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
    assert_eq!(
        outcome,
        ConnectOutcome::Cancelled(CancelReason::IdentityCheckFailed(
            IdentityProblem::EmailNotVerified
        ))
    );
    for bad in [
        "no-at-sign",
        "\u{e9}t\u{e9}@example.com",
        "a@localhost",
        "a b@example.com",
    ] {
        rig.fake
            .arm(Fault::id_token(IdTokenFault::Email(bad.to_owned())));
        let (outcome, _, _) = rig.sign_in(
            ConnectJob::new_account(account(2), []),
            Decision::allow().sign_in_as(SUB_B),
        );
        assert_eq!(
            outcome,
            ConnectOutcome::Cancelled(CancelReason::IdentityCheckFailed(
                IdentityProblem::EmailNotUsable
            )),
            "{bad}"
        );
    }
    assert!(rig.store.inner.is_empty());
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}

#[test]
fn g13_the_token_answer_is_checked_and_each_failure_is_cancelled_with_nothing_stored() {
    let rig = Rig::new();
    let cases: Vec<(Fault, CancelReason)> = vec![
        (
            Fault::token_answer(TokenFault::NoRefreshToken),
            CancelReason::NoRefreshToken,
        ),
        (
            Fault::token_answer(TokenFault::TokenType("MAC".to_owned())),
            CancelReason::GoogleError,
        ),
        (
            Fault::token_answer(TokenFault::DropMember("token_type".to_owned())),
            CancelReason::GoogleError,
        ),
        (
            Fault::token_answer(TokenFault::DropMember("scope".to_owned())),
            CancelReason::GoogleError,
        ),
        (
            Fault::token_answer(TokenFault::DropMember("access_token".to_owned())),
            CancelReason::GoogleError,
        ),
        (
            Fault::token_answer(TokenFault::NoIdToken),
            CancelReason::IdentityCheckFailed(IdentityProblem::NoIdToken),
        ),
        (Fault::token(FaultAction::NotJson), CancelReason::Network),
        (
            Fault::token(FaultAction::ContentType("text/html".to_owned())),
            CancelReason::Network,
        ),
        (
            Fault::token(FaultAction::CloseWithoutApplying),
            CancelReason::Network,
        ),
        (
            Fault::token(FaultAction::invalid_client()),
            CancelReason::ClientRejected,
        ),
        (
            Fault::token(FaultAction::oauth_error(400, "unauthorized_client")),
            CancelReason::ClientRejected,
        ),
        (
            Fault::token(FaultAction::invalid_grant()),
            CancelReason::GoogleError,
        ),
        (
            Fault::token(FaultAction::error(503, "backendError")),
            CancelReason::GoogleError,
        ),
    ];
    for (fault, reason) in cases {
        let label = format!("{fault:?}");
        rig.fake.arm(fault);
        let (outcome, _, _) =
            rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
        assert_eq!(outcome, ConnectOutcome::Cancelled(reason), "{label}");
        assert!(rig.store.inner.is_empty(), "{label}");
    }
    assert_eq!(
        rig.token_requests("authorization_code"),
        13,
        "one attempt each"
    );
    assert!(rig.fake.log().revocations().is_empty());

    // refresh_token_expires_in is passed on as the end of the connection.
    rig.fake
        .arm(Fault::token_answer(TokenFault::RefreshTokenExpiresIn(
            604_800,
        )));
    let now = rig.clock.now_ms();
    let (outcome, _, _) = rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
    let ConnectOutcome::Connected {
        refresh_token_ends_at,
        ..
    } = outcome
    else {
        panic!("not connected: {outcome:?}");
    };
    assert_eq!(refresh_token_ends_at, Some(now + 604_800_000));

    // expires_in is held to 60 s .. 86 400 s.
    for (given, held_s) in [(5, 60), (1_000_000, 86_400), (3599, 3599)] {
        rig.fake
            .arm(Fault::token_answer(TokenFault::ExpiresIn(given)));
        rig.sign_in(ConnectJob::new_account(account(3), []), Decision::allow());
        let now = rig.clock.now_ms();
        let end = now + held_s * 1000;
        assert!(
            rig.cache.get_valid(&account(3), end - 1, 0).is_some(),
            "{given}"
        );
        assert!(
            rig.cache.get_valid(&account(3), end, 0).is_none(),
            "{given}"
        );
    }
    rig.check_log();
}

#[test]
fn g13_an_access_or_refresh_token_that_cannot_go_in_a_header_is_google_error_with_nothing_stored() {
    let rig = Rig::new();
    for (member, value) in [
        ("access_token", "ya29.with space"),
        ("access_token", "ya29.\u{e9}"),
        ("refresh_token", "1//with\ttab"),
        ("refresh_token", &"r".repeat(4097)),
    ] {
        let label = format!("{member} of {} bytes", value.len());
        rig.fake.arm(Fault::token_answer(TokenFault::Member(
            member.to_owned(),
            serde_json::Value::String(value.to_owned()),
        )));
        let (outcome, _, _) =
            rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
        assert_eq!(
            outcome,
            ConnectOutcome::Cancelled(CancelReason::GoogleError),
            "{label}"
        );
        assert!(rig.store.inner.is_empty(), "{label}");
        assert!(rig.cache.get_valid(&account(1), 0, 0).is_none(), "{label}");
    }
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}

/// PD5, 3.3: flow A (a reconnect) is held inside its token request; flow B for the same
/// account starts, which supersedes A, and finishes; then A's request goes on. A ends
/// `Superseded`, writes nothing and forgets nothing: B's token stays and still refreshes.
#[test]
fn g06_a_superseded_reconnect_never_deletes_the_token_of_the_flow_that_superseded_it() {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let proxy = HoldingProxy::start(rig.fake.port());
    let flow = rig.flow_through(proxy.port());
    let job = || ConnectJob::reconnect(account(1), SUB_A);

    let (a, a_address) = rig.start_on(&flow, job());
    proxy.hold_next_token_request();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let mut a_store = rig.store.clone();
        let a_waiter = scope.spawn(move || a.wait(&mut a_store, cache));
        let a_visit = rig
            .fake
            .browser()
            .visit(a_address.expose(), Decision::allow())
            .expect("visit");
        assert_eq!(a_visit.listener_answer().expect("answered").status, 200);
        // A is inside its token request now, and stays there until released.
        proxy.wait_until_holding();

        let (b, b_address) = rig.start_on(&flow, job());
        let fake = &rig.fake;
        let b_visit = scope.spawn(move || {
            fake.browser()
                .visit(b_address.expose(), Decision::allow())
                .expect("visit")
        });
        let mut b_store = rig.store.clone();
        let b_outcome = b.wait(&mut b_store, cache);
        b_visit.join().expect("the browser thread ends");
        assert!(b_outcome.is_connected(), "{b_outcome:?}");
        let b_token = rig.store.token(&account(1)).expect("B stored its token");
        assert!(
            !a_waiter.is_finished(),
            "A is still held in its token request"
        );

        proxy.release();
        wait_until(Duration::from_secs(20), || a_waiter.is_finished());
        assert_eq!(
            a_waiter.join().expect("A ends"),
            ConnectOutcome::Cancelled(CancelReason::Superseded)
        );
        assert_eq!(
            rig.store.token(&account(1)),
            Some(b_token),
            "A neither deleted nor replaced B's token"
        );
    });
    assert_eq!(rig.token_requests("authorization_code"), 3);
    // B's token still refreshes.
    rig.cache.drop_token(&account(1));
    let before = rig.token_requests("refresh_token");
    rig.access()
        .access_token(&account(1), rig.deadline())
        .expect("B's token refreshes");
    assert_eq!(rig.token_requests("refresh_token"), before + 1);
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}

/// The access token the cache holds for an account, as text.
fn cached(rig: &Rig, n: u32) -> Option<String> {
    rig.cache
        .get_valid(&account(n), 0, 0)
        .map(|t| t.expose().to_owned())
}

/// 18.7 point 5(a): a reconnect held inside its token request is superseded by its generation
/// (`true`); released, it ends `Superseded`, and the token store and the cache were never
/// written: the account's earlier token and cached access token stay, nothing is forgotten. A
/// supersede with another generation answers `false` and changes nothing; a second supersede
/// of the same flow answers `false`.
#[test]
fn a_flow_superseded_by_its_generation_in_its_token_request_writes_nothing() {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let stored = rig.store.token(&account(1)).expect("stored");
    let access = cached(&rig, 1).expect("cached");
    let proxy = HoldingProxy::start(rig.fake.port());
    let flow = rig.flow_through(proxy.port());
    let (a, a_address) = rig.start_on(&flow, ConnectJob::reconnect(account(1), SUB_A));
    let generation = a.generation();
    proxy.hold_next_token_request();
    let (writes, deletes) = (rig.store.writes(), rig.store.deletes());
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let mut a_store = rig.store.clone();
        let waiter = scope.spawn(move || a.wait(&mut a_store, cache));
        let visit = rig
            .fake
            .browser()
            .visit(a_address.expose(), Decision::allow())
            .expect("visit");
        assert_eq!(visit.listener_answer().expect("answered").status, 200);
        proxy.wait_until_holding();

        assert!(!flow.supersede(generation + 1), "no such flow");
        assert!(!flow.supersede(generation - 1), "no such flow");
        assert!(!flow.supersede(0));
        assert!(!waiter.is_finished());
        assert!(flow.supersede(generation));
        assert!(!flow.supersede(generation), "already superseded");
        assert!(!flow.supersede_latest(), "already superseded");

        proxy.release();
        wait_until(Duration::from_secs(20), || waiter.is_finished());
        assert_eq!(
            waiter.join().expect("A ends"),
            ConnectOutcome::Cancelled(CancelReason::Superseded)
        );
    });
    assert_eq!(rig.store.writes(), writes, "the store was never written");
    assert_eq!(rig.store.deletes(), deletes, "nothing was forgotten");
    assert_eq!(rig.store.token(&account(1)), Some(stored));
    assert_eq!(cached(&rig, 1), Some(access), "the cache was never written");
    assert_eq!(rig.token_requests("authorization_code"), 2);
    rig.check_log();
}

/// 18.7 point 5(a): `supersede_latest` with no flow started, and after the latest flow ended
/// `Connected`, answers `false`, and the stored token stays.
#[test]
fn supersede_latest_with_no_flow_or_after_a_connected_flow_is_false_and_the_token_stays() {
    let rig = Rig::new();
    assert!(!rig.flow.supersede_latest(), "no flow was started");
    assert!(!rig.flow.supersede(1));
    let (pending, address) = rig.start(ConnectJob::new_account(account(1), []));
    let generation = pending.generation();
    let handle = pending.supersede_handle();
    let (outcome, _) = rig.finish(pending, &address, Decision::allow());
    assert!(outcome.is_connected(), "{outcome:?}");
    let stored = rig.store.token(&account(1)).expect("stored");
    assert!(!rig.flow.supersede_latest());
    assert!(!rig.flow.supersede(generation));
    assert!(!handle.supersede(), "the handle answers the same");
    assert_eq!(rig.store.token(&account(1)), Some(stored));
    assert!(cached(&rig, 1).is_some());
    // A flow that ended without writing has ended too.
    let (pending, address) = rig.start(ConnectJob::new_account(account(2), []));
    let (outcome, _) = rig.finish(pending, &address, Decision::deny());
    assert_eq!(outcome, ConnectOutcome::ConsentDenied);
    assert!(!rig.flow.supersede_latest());
    rig.check_log();
}

/// 18.7 point 5(a): a flow superseded before `wait` (by its generation, by `supersede_latest`
/// or by its handle) ends `Superseded` at once: no listener is run, no token request reaches
/// the fake, nothing is written or forgotten. `generation()` rises by one per start, and a
/// start supersedes the flow before it.
#[test]
fn a_flow_superseded_before_wait_ends_at_once_without_a_request() {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let stored = rig.store.token(&account(1)).expect("stored");
    let (writes, deletes) = (rig.store.writes(), rig.store.deletes());
    let token_requests = rig.token_requests("authorization_code");
    for way in 0..3 {
        let (pending, address) = rig.start(ConnectJob::reconnect(account(1), SUB_A));
        assert!(pending.port().is_some());
        let superseded = match way {
            0 => rig.flow.supersede(pending.generation()),
            1 => rig.flow.supersede_latest(),
            _ => pending.supersede_handle().supersede(),
        };
        assert!(superseded, "way {way}");
        let mut store = rig.store.clone();
        assert_eq!(
            pending.wait(&mut store, &rig.cache),
            ConnectOutcome::Cancelled(CancelReason::Superseded),
            "way {way}"
        );
        // The listener is closed: the browser's redirect finds nobody.
        let visit = rig
            .fake
            .browser()
            .visit(address.expose(), Decision::allow())
            .expect("visit");
        assert!(
            !visit
                .listener_answer()
                .is_some_and(|a| a.status == 200 && a.text().contains(PAGE_TEXT)),
            "way {way}: a superseded flow's listener answered"
        );
    }
    assert_eq!(rig.token_requests("authorization_code"), token_requests);
    assert_eq!((rig.store.writes(), rig.store.deletes()), (writes, deletes));
    assert_eq!(rig.store.token(&account(1)), Some(stored));

    // Generations rise by one per start; a start supersedes the flow before it.
    let (first, _) = rig.start(ConnectJob::new_account(account(5), []));
    let (second, _) = rig.start(ConnectJob::new_account(account(6), []));
    let (third, _) = rig.start(ConnectJob::new_account(account(7), []));
    assert_eq!(second.generation(), first.generation() + 1);
    assert_eq!(third.generation(), second.generation() + 1);
    assert!(
        !rig.flow.supersede(first.generation()),
        "superseded by the start"
    );
    let mut store = rig.store.clone();
    assert_eq!(
        first.wait(&mut store, &rig.cache),
        ConnectOutcome::Cancelled(CancelReason::Superseded)
    );
    assert!(rig.flow.supersede(third.generation()));
    drop((second, third));
    rig.check_log();
}

/// A token store whose writes wait until the test releases them, over a [`CountingStore`].
#[derive(Clone, Default)]
struct BlockingStore {
    inner: CountingStore,
    /// (a write has begun, writes may go on).
    gate: std::sync::Arc<(std::sync::Mutex<(bool, bool)>, std::sync::Condvar)>,
}

impl BlockingStore {
    fn wait_until_writing(&self) {
        let (state, signal) = &*self.gate;
        let guard = state.lock().expect("unpoisoned");
        let (guard, _) = signal
            .wait_timeout_while(guard, Duration::from_secs(20), |s| !s.0)
            .expect("unpoisoned");
        assert!(guard.0, "no write began");
    }

    fn release(&self) {
        let (state, signal) = &*self.gate;
        state.lock().expect("unpoisoned").1 = true;
        signal.notify_all();
    }
}

impl TokenStore for BlockingStore {
    fn read(
        &self,
        account: &fabric_desktop_contracts::scalar::Id,
    ) -> Result<Option<fabric_google_calendar::Secret>, fabric_google_calendar::TokenStoreError>
    {
        self.inner.read(account)
    }
    fn write(
        &mut self,
        account: &fabric_desktop_contracts::scalar::Id,
        token: &fabric_google_calendar::Secret,
    ) -> Result<(), fabric_google_calendar::TokenStoreError> {
        {
            let (state, signal) = &*self.gate;
            let mut guard = state.lock().expect("unpoisoned");
            guard.0 = true;
            signal.notify_all();
            let _ = signal
                .wait_timeout_while(guard, Duration::from_secs(20), |s| !s.1)
                .expect("unpoisoned");
        }
        self.inner.write(account, token)
    }
    fn delete(
        &mut self,
        account: &fabric_desktop_contracts::scalar::Id,
    ) -> Result<(), fabric_google_calendar::TokenStoreError> {
        self.inner.delete(account)
    }
}

/// 18.7 point 5(a), the lock. A supersede called while the flow is inside its write to the
/// token store waits for the write to finish and answers `false`: the token is stored and the
/// flow is `Connected`. (The other order, a supersede before the write, answers `true` and
/// nothing is stored: `a_flow_superseded_by_its_generation_in_its_token_request_writes_nothing`.)
/// So `true` with a stored token never happens. The supersede is shown blocked for 300 ms
/// while the write is held; that check cannot fail against a correct lock.
#[test]
fn a_supersede_during_the_write_waits_for_it_and_answers_false_with_the_token_stored() {
    let rig = Rig::new();
    let store = BlockingStore {
        inner: rig.store.clone(),
        ..BlockingStore::default()
    };
    let (pending, address) = rig.start(ConnectJob::new_account(account(1), []));
    let generation = pending.generation();
    let handle = pending.supersede_handle();
    let flow = rig.flow.clone();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let mut flow_store = store.clone();
        let waiter = scope.spawn(move || pending.wait(&mut flow_store, cache));
        let fake = &rig.fake;
        let visit = scope.spawn(move || {
            fake.browser()
                .visit(address.expose(), Decision::allow())
                .expect("visit")
        });
        store.wait_until_writing();
        let by_generation = scope.spawn(move || flow.supersede(generation));
        let by_handle = scope.spawn(move || handle.supersede());
        let held = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < held {
            assert!(
                !by_generation.is_finished() && !by_handle.is_finished(),
                "a supersede returned while the flow was writing"
            );
            std::thread::yield_now();
        }
        assert!(rig.store.token(&account(1)).is_none(), "the write is held");
        store.release();
        assert!(!by_generation.join().expect("returns"));
        assert!(!by_handle.join().expect("returns"));
        let outcome = waiter.join().expect("the flow ends");
        assert!(outcome.is_connected(), "{outcome:?}");
        visit.join().expect("the browser thread ends");
    });
    assert!(
        rig.store.token(&account(1)).is_some(),
        "the token is stored"
    );
    assert!(cached(&rig, 1).is_some());
    assert!(!rig.flow.supersede_latest());
    rig.check_log();
}
