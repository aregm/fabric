//! g16 for the sign-in part: no secret in any `Debug`, `Display` or error text (3.10, PD27).
//!
//! Full runs with injected faults; then every formatted value the tests can reach is searched
//! for the verifier, the state, the nonce, every code, every token, the client secret and the
//! authorization address. The Calendar part (calendar ids, event titles) is searched by the
//! Calendar tests.

#![cfg(debug_assertions)]

mod oauth_common;

use std::time::Duration;

use fabric_google_calendar::oauth::exchange::{AnswerProblem, OAuthError, read_token_json};
use fabric_google_calendar::oauth::id_token::{CheckFailure, KeyCache};
use fabric_google_calendar::oauth::listener::{CallbackEnd, Listener};
use fabric_google_calendar::oauth::pkce::FlowSecrets;
use fabric_google_calendar::oauth::{
    BrowserNotOpened, CancelReason, ConnectJob, IdentityProblem, RecordingBrowser, Revocation,
    Supersede, revoke,
};
use fabric_google_calendar::{AccessTokens, Secret};
use fabric_google_fake::{
    Decision, Fault, FaultAction, IdTokenFault, SCOPE_EVENTS, Tamper, TokenFault,
};
use oauth_common::*;

#[test]
fn g16_no_secret_of_a_sign_in_appears_in_any_debug_display_or_error_text() {
    let rig = Rig::new();
    let mut texts: Vec<String> = Vec::new();
    let mut secrets: Vec<String> = vec![CLIENT_SECRET.to_owned()];

    // Full runs: a connect, then faults of every kind.
    let decisions: Vec<(Option<Fault>, Decision, u32)> = vec![
        (None, Decision::allow(), 1),
        (None, Decision::deny(), 2),
        (None, Decision::allow().untick(&[SCOPE_EVENTS]), 2),
        (
            None,
            Decision::allow().tamper(Tamper::Iss("https://evil.example".into())),
            2,
        ),
        (
            Some(Fault::id_token(IdTokenFault::BadSignature)),
            Decision::allow(),
            2,
        ),
        (
            Some(Fault::id_token(IdTokenFault::Nonce("n".into()))),
            Decision::allow(),
            2,
        ),
        (
            Some(Fault::token_answer(TokenFault::NoRefreshToken)),
            Decision::allow(),
            2,
        ),
        (
            Some(Fault::token(FaultAction::invalid_client())),
            Decision::allow(),
            2,
        ),
        (
            Some(Fault::token(FaultAction::NotJson)),
            Decision::allow(),
            2,
        ),
        (None, Decision::allow().sign_in_as(SUB_B), 5),
        (None, Decision::allow(), 3),
    ];
    for (fault, decision, n) in decisions {
        if let Some(fault) = fault {
            rig.fake.arm(fault);
        }
        let taken = if n == 5 {
            vec![SUB_B.to_owned()]
        } else {
            Vec::new()
        };
        let job = ConnectJob::new_account(account(n), taken);
        texts.push(format!("{job:?}"));
        let (pending, address) = rig.start(job);
        texts.push(format!("{pending:?} {address:?} {:?}", rig.flow));
        texts.push(format!("{:?}", pending.supersede_handle()));
        let (outcome, visit) = rig.finish(pending, &address, decision);
        texts.push(format!("{outcome:?} {outcome:#?}"));
        if let fabric_google_calendar::oauth::ConnectOutcome::Cancelled(reason) = &outcome {
            texts.push(format!("{reason} {reason:?}"));
        }
        secrets.push(address.expose().to_owned());
        for name in ["state", "nonce"] {
            secrets.extend(query_value(address.expose(), name));
        }
        secrets.extend(visit.code());
        secrets.extend(visit.callback_address);
    }

    // A reconnect that times out, and one superseded.
    let (pending, address) = rig.start(ConnectJob::reconnect(account(1), SUB_A));
    let handle = pending.supersede_handle();
    let mut store = rig.store.clone();
    let cache = &rig.cache;
    let outcome = std::thread::scope(|scope| {
        let waiter = scope.spawn(move || pending.wait(&mut store, cache));
        // `wait` opens the browser; superseded after that.
        secrets.push(address.expose().to_owned());
        handle.supersede();
        wait_until(Duration::from_secs(20), || waiter.is_finished());
        waiter.join().expect("ends")
    });
    texts.push(format!("{outcome:?} {handle:?}"));

    // Access tokens: refreshes, a rotation, refusals, and the token part's own Debug.
    rig.connect(4, Decision::allow());
    let mut access = rig.access();
    rig.cache.drop_account(&account(4));
    rig.fake.arm(
        Fault::token(FaultAction::Token(TokenFault::RotateRefreshToken))
            .grant_type("refresh_token"),
    );
    let token = access.access_token(&account(4), rig.deadline());
    texts.push(format!(
        "{token:?} {access:?} {:?} {:?}",
        rig.cache, rig.store
    ));
    rig.cache.drop_account(&account(4));
    rig.fake
        .arm(Fault::token(FaultAction::invalid_grant()).grant_type("refresh_token"));
    let lost = access.access_token(&account(4), rig.deadline());
    texts.push(format!("{lost:?}"));
    let revoked = revoke(&rig.transport, &rig.store, &account(3), rig.deadline());
    texts.push(format!("{revoked:?} {:?}", Revocation::NotConfirmed));
    texts.push(format!("{:?} {:?}", rig.transport, rig.client));

    // Every secret the fake saw: codes, verifiers, refresh tokens, revoked tokens, the client
    // secret, the bearer tokens; and every token it still holds.
    for entry in rig.fake.log().iter() {
        for name in [
            "code",
            "code_verifier",
            "refresh_token",
            "token",
            "client_secret",
        ] {
            secrets.extend(entry.form_value(name).map(str::to_owned));
        }
        for name in ["state", "nonce"] {
            secrets.extend(entry.query_value(name).map(str::to_owned));
        }
        if let Some(bearer) = entry.header("Authorization") {
            secrets.push(bearer.trim_start_matches("Bearer ").to_owned());
        }
    }
    for subject in [SUB_A, SUB_B] {
        secrets.extend(rig.fake.with_state(|s| s.valid_refresh_tokens(subject)));
        secrets.extend(rig.fake.with_state(|s| s.valid_access_tokens(subject)));
    }
    let stored = rig.store.token(&account(3)).expect("account 3 connected");
    secrets.push(stored.clone());

    // Values built directly from secrets.
    let flow_secrets = FlowSecrets::new().expect("random");
    secrets.extend([
        flow_secrets.verifier.expose().to_owned(),
        flow_secrets.state.expose().to_owned(),
        flow_secrets.nonce.expose().to_owned(),
    ]);
    texts.push(format!("{flow_secrets:?}"));
    let code = Secret::from("4/0fake-code-direct-value");
    secrets.push(code.expose().to_owned());
    texts.push(format!("{:?}", CallbackEnd::Code(code)));
    let listener = Listener::bind().expect("a listener");
    texts.push(format!("{listener:?}"));
    let answer = read_token_json(&serde_json::json!({
        "access_token": stored, "expires_in": 3600, "refresh_token": stored,
        "id_token": stored, "scope": "openid"
    }))
    .expect("an answer");
    texts.push(format!("{answer:?}"));
    let mut browser = RecordingBrowser::new();
    let _ = fabric_google_calendar::oauth::BrowserOpener::open(
        &mut browser,
        &Secret::from(stored.as_str()),
    );
    texts.push(format!("{browser:?}"));
    texts.push(format!("{:?}", KeyCache::new()));
    texts.push(format!(
        "{BrowserNotOpened} {BrowserNotOpened:?} {:?}",
        Supersede::default()
    ));
    texts.push(format!(
        "{:?} {:?} {:?}",
        AnswerProblem::Error(OAuthError::Other),
        CheckFailure::KeysUnavailable,
        CheckFailure::Identity(IdentityProblem::Nonce)
    ));
    for reason in [
        CancelReason::TimedOut,
        CancelReason::Network,
        CancelReason::IdentityCheckFailed(IdentityProblem::Signature),
        CancelReason::TokenStore(fabric_google_calendar::TokenStoreError::Failed { status: 1 }),
    ] {
        texts.push(format!("{reason} {reason:?}"));
    }

    secrets.retain(|s| s.len() >= 8);
    secrets.sort();
    secrets.dedup();
    assert!(
        secrets.len() > 40,
        "the search covers the secrets of the runs"
    );
    // A failure names the text's index and the needle's length, never the text: it would
    // print the secret it found.
    for (i, text) in texts.iter().enumerate() {
        for secret in &secrets {
            assert!(
                !text.contains(secret.as_str()),
                "a secret of {} bytes in formatted value {i}",
                secret.len()
            );
        }
        // No address either.
        assert!(
            !text.contains("127.0.0.1:"),
            "an address in formatted value {i}"
        );
        for subject in [SUB_A, SUB_B] {
            assert!(
                !text.contains(subject),
                "a subject of {} bytes in formatted value {i}",
                subject.len()
            );
        }
    }
    rig.check_log();
}
