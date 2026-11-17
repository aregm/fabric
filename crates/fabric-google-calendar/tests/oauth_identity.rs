//! The identity token and Google's key set against the fake (design 3.5): g10, g11.

mod oauth_common;

use fabric_google_calendar::oauth::{CancelReason, ConnectJob, ConnectOutcome, IdentityProblem};
use fabric_google_fake::{Decision, Fault, IdTokenFault, KeyId, TokenFault};
use oauth_common::*;
use serde_json::json;

fn identity_failed(problem: IdentityProblem) -> ConnectOutcome {
    ConnectOutcome::Cancelled(CancelReason::IdentityCheckFailed(problem))
}

#[test]
fn g10_each_identity_token_fault_is_identity_check_failed_and_nothing_is_stored() {
    use IdentityProblem as P;
    let rig = Rig::new();
    let cases: Vec<(Fault, IdentityProblem)> = vec![
        (
            Fault::id_token(IdTokenFault::Issuer("https://evil.example".into())),
            P::Issuer,
        ),
        (
            Fault::id_token(IdTokenFault::Audience(json!("other-client"))),
            P::Audience,
        ),
        (
            Fault::id_token(IdTokenFault::Audience(json!([CLIENT_ID, "other"]))),
            P::Audience,
        ),
        (
            Fault::id_token(IdTokenFault::Azp("other-client".into())),
            P::AuthorizedParty,
        ),
        (
            Fault::id_token(IdTokenFault::Nonce("another-nonce".into())),
            P::Nonce,
        ),
        (Fault::id_token(IdTokenFault::NoNonce), P::Nonce),
        (Fault::id_token(IdTokenFault::Expired), P::Times),
        (Fault::id_token(IdTokenFault::IssuedInFuture), P::Times),
        (Fault::id_token(IdTokenFault::TooOld), P::Times),
        (Fault::id_token(IdTokenFault::BadSignature), P::Signature),
        (Fault::id_token(IdTokenFault::AlgNone), P::Algorithm),
        (Fault::id_token(IdTokenFault::AlgHs256), P::Algorithm),
        (Fault::id_token(IdTokenFault::UnknownKid), P::UnknownKey),
        (Fault::id_token(IdTokenFault::NoKid), P::NoKeyId),
        (Fault::id_token(IdTokenFault::Crit), P::Critical),
        (Fault::id_token(IdTokenFault::JwkHeader), P::Signature),
        (
            Fault::id_token(IdTokenFault::DoubledHeaderMember),
            P::Header,
        ),
        (
            Fault::id_token(IdTokenFault::DoubledPayloadMember),
            P::Payload,
        ),
        (
            Fault::id_token(IdTokenFault::WeakExponentKey),
            P::KeyNotUsable,
        ),
        (
            Fault::id_token(IdTokenFault::ShortModulusKey),
            P::KeyNotUsable,
        ),
        (
            Fault::id_token(IdTokenFault::SignWith(KeyId::Attacker)),
            P::UnknownKey,
        ),
        (
            Fault::id_token(IdTokenFault::EmailUnverified),
            P::EmailNotVerified,
        ),
        (
            Fault::id_token(IdTokenFault::Subject(String::new())),
            P::Subject,
        ),
        (
            Fault::id_token(IdTokenFault::DropClaim("exp".into())),
            P::Times,
        ),
        (Fault::token_answer(TokenFault::NoIdToken), P::NoIdToken),
    ];
    for (fault, problem) in cases {
        let label = format!("{fault:?}");
        // A minute apart, so that each unknown key may fetch the key set once more (3.5 item 3).
        rig.advance_ms(61_000);
        rig.fake.arm(fault);
        let (outcome, _, _) =
            rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
        assert_eq!(outcome, identity_failed(problem), "{label}");
        assert!(rig.store.inner.is_empty(), "{label}");
        assert!(
            rig.cache.get_valid(&account(1), 0, 0).is_none(),
            "{label}: no access token is kept"
        );
    }
    assert_eq!(rig.store.writes(), 0);
    assert!(rig.fake.log().revocations().is_empty());
    // The key set was the only source of keys: the attacker's key in a jwk header was never
    // trusted, and no request went anywhere else.
    rig.check_log();
}

#[test]
fn g11_an_unknown_key_id_fetches_the_key_set_once_more_and_then_verifies() {
    let rig = Rig::new();
    rig.fake.with_state(|s| {
        s.unpublish_key(KeyId::B);
        s.publish_key(KeyId::B, 1);
        s.set_signing_key(KeyId::B);
    });
    rig.connect(1, Decision::allow());
    assert_eq!(rig.fake.with_state(|s| s.key_set_fetches()), 2);
    // The next sign-in uses the kept set.
    rig.connect(2, Decision::allow().sign_in_as(SUB_B));
    assert_eq!(rig.fake.with_state(|s| s.key_set_fetches()), 2);
    rig.check_log();
}

#[test]
fn g11_an_unknown_key_id_refetches_at_most_once_a_minute() {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    assert_eq!(rig.fake.with_state(|s| s.key_set_fetches()), 1);
    let unknown = || {
        rig.fake.arm(Fault::id_token(IdTokenFault::UnknownKid));
        let (outcome, _, _) =
            rig.sign_in(ConnectJob::new_account(account(2), []), Decision::allow());
        assert_eq!(outcome, identity_failed(IdentityProblem::UnknownKey));
        rig.fake.with_state(|s| s.key_set_fetches())
    };
    assert_eq!(unknown(), 2);
    rig.advance_ms(59_999);
    assert_eq!(unknown(), 2);
    rig.advance_ms(1);
    assert_eq!(unknown(), 3);
    rig.check_log();
}

#[test]
fn g11_the_key_set_is_kept_for_its_max_age_held_to_five_minutes_and_24_hours() {
    for (max_age_s, kept_s) in [(10, 300), (21_600, 21_600), (604_800, 86_400)] {
        let rig = Rig::new();
        rig.fake.with_state(|s| s.set_key_set_max_age(max_age_s));
        rig.connect(1, Decision::allow());
        assert_eq!(rig.fake.with_state(|s| s.key_set_fetches()), 1);
        rig.advance_ms(kept_s * 1000 - 1);
        rig.connect(2, Decision::allow());
        assert_eq!(
            rig.fake.with_state(|s| s.key_set_fetches()),
            1,
            "kept within {kept_s} s for max-age {max_age_s}"
        );
        rig.advance_ms(1);
        rig.connect(3, Decision::allow());
        assert_eq!(
            rig.fake.with_state(|s| s.key_set_fetches()),
            2,
            "fetched again after {kept_s} s for max-age {max_age_s}"
        );
        rig.check_log();
    }
}
