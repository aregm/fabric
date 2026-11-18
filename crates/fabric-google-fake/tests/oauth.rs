//! f01, the sign-in side: codes, tokens, identity tokens, the key set, revocation and the
//! browser, through raw HTTP (no adapter).

mod common;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::*;
use fabric_google_fake::{
    Client, Decision, Fault, FaultAction, IdTokenFault, KeyId, SCOPE_EVENTS, SCOPE_USERINFO_EMAIL,
    Tamper, TokenFault,
};
use rsa::{BoxedUint, Pkcs1v15Sign, PublicKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Signs in with `decision` through the browser and gives the code from the redirect.
fn code_for(
    fake: &fabric_google_fake::FakeGoogle,
    listener: &Listener,
    decision: Decision,
) -> String {
    let address = authorization_address(fake, &listener.redirect(), "state-1", "nonce-1");
    let visit = fake.browser().visit(&address, decision).unwrap();
    assert_eq!(visit.authorization.status, 302);
    let _ = listener.next();
    visit.code().expect("a code")
}

fn exchange(
    fake: &fabric_google_fake::FakeGoogle,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Answer {
    form(
        fake,
        "/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("redirect_uri", redirect),
        ],
    )
}

fn refresh(fake: &fabric_google_fake::FakeGoogle, refresh_token: &str) -> Answer {
    form(
        fake,
        "/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ],
    )
}

/// The header, payload and signature of a JWT, the JSON read as it comes.
fn jwt(token: &str) -> (Value, Value, Vec<u8>, String) {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header = String::from_utf8(URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    let payload = URL_SAFE_NO_PAD.decode(parts[1]).unwrap();
    (
        serde_json::from_str(&header).unwrap(),
        serde_json::from_slice(&payload).unwrap(),
        URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
        header,
    )
}

fn verifies_with(jwk: &Value, token: &str) -> bool {
    let n = URL_SAFE_NO_PAD.decode(jwk["n"].as_str().unwrap()).unwrap();
    let e = URL_SAFE_NO_PAD.decode(jwk["e"].as_str().unwrap()).unwrap();
    let key = PublicKey::new(
        BoxedUint::from_be_slice_vartime(&n),
        BoxedUint::from_be_slice_vartime(&e),
    )
    .unwrap();
    let (input, signature) = token.rsplit_once('.').unwrap();
    let digest = Sha256::digest(input.as_bytes());
    key.verify(
        Pkcs1v15Sign::new::<Sha256>(),
        &digest,
        &URL_SAFE_NO_PAD.decode(signature).unwrap(),
    )
    .is_ok()
}

fn key_set(fake: &fabric_google_fake::FakeGoogle) -> Answer {
    request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"")
}

#[test]
fn a_full_sign_in_redirects_with_state_code_iss_scope_and_authuser() {
    let fake = fake();
    let listener = Listener::start();
    let address = authorization_address(&fake, &listener.redirect(), "state-1", "nonce-1");
    let visit = fake.browser().visit(&address, Decision::allow()).unwrap();
    assert_eq!(visit.authorization.status, 302);
    let head = listener.next();
    assert!(head.starts_with("GET /?state=state-1&iss=https%3A%2F%2Faccounts.google.com&code="));
    assert!(head.contains(&format!("Host: 127.0.0.1:{}", listener.port)));
    assert_eq!(visit.listener_answer().unwrap().text(), "listener page");
    assert_eq!(visit.redirect_parameter("authuser").as_deref(), Some("0"));
    let scope = visit.redirect_parameter("scope").unwrap();
    assert!(scope.contains(SCOPE_USERINFO_EMAIL) && scope.contains(SCOPE_EVENTS));
    let log = fake.log();
    assert_eq!(log.authorizations().len(), 1);
    assert_eq!(fake.with_state(|s| s.codes_issued()), 1);
}

#[test]
fn a_code_is_single_use() {
    let fake = fake();
    let listener = Listener::start();
    let code = code_for(&fake, &listener, Decision::allow());
    let first = exchange(&fake, &code, &listener.redirect(), VERIFIER);
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let answer = first.json();
    for member in [
        "access_token",
        "refresh_token",
        "id_token",
        "scope",
        "expires_in",
    ] {
        assert!(answer.get(member).is_some(), "{member}");
    }
    assert_eq!(answer["token_type"], "Bearer");
    assert_eq!(answer["expires_in"], 3599);
    assert_eq!(
        first
            .header("Cache-Control")
            .map(|v| v.contains("no-store")),
        Some(true)
    );
    let second = exchange(&fake, &code, &listener.redirect(), VERIFIER);
    assert_eq!(second.status, 400);
    assert_eq!(second.json()["error"], "invalid_grant");
    assert_eq!(fake.with_state(|s| s.code_used(&code)), Some(true));
}

#[test]
fn a_code_is_bound_to_its_challenge_and_used_by_a_wrong_verifier() {
    let fake = fake();
    let listener = Listener::start();
    let code = code_for(&fake, &listener, Decision::allow());
    let wrong = exchange(&fake, &code, &listener.redirect(), &"x".repeat(43));
    assert_eq!(
        (wrong.status, wrong.json()["error"].clone()),
        (400, json!("invalid_grant"))
    );
    let right = exchange(&fake, &code, &listener.redirect(), VERIFIER);
    assert_eq!(right.status, 400, "the first presentation used the code");
}

#[test]
fn a_code_is_bound_to_its_redirect_and_its_client() {
    let fake = fake();
    fake.add_client(Client::new("other-client", "other-secret"));
    let listener = Listener::start();
    let code = code_for(&fake, &listener, Decision::allow());
    let other_redirect = exchange(&fake, &code, "http://127.0.0.1:1", VERIFIER);
    assert_eq!(other_redirect.json()["error"], "redirect_uri_mismatch");

    let code = code_for(&fake, &listener, Decision::allow());
    let other_client = form(
        &fake,
        "/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", VERIFIER),
            ("client_id", "other-client"),
            ("client_secret", "other-secret"),
            ("redirect_uri", &listener.redirect()),
        ],
    );
    assert_eq!(other_client.json()["error"], "invalid_grant");
}

#[test]
fn the_token_endpoint_refuses_an_unknown_client_a_wrong_secret_and_a_missing_one() {
    let fake = fake();
    let base = [("grant_type", "refresh_token"), ("refresh_token", "x")];
    let unknown = form(
        &fake,
        "/token",
        &[base[0], base[1], ("client_id", "nobody")],
    );
    assert_eq!(
        (unknown.status, unknown.json()["error"].clone()),
        (401, json!("invalid_client"))
    );
    let wrong = form(
        &fake,
        "/token",
        &[
            base[0],
            base[1],
            ("client_id", CLIENT_ID),
            ("client_secret", "no"),
        ],
    );
    assert_eq!(wrong.status, 401);
    let missing = form(
        &fake,
        "/token",
        &[base[0], base[1], ("client_id", CLIENT_ID)],
    );
    assert_eq!(
        (missing.status, missing.json()["error"].clone()),
        (400, json!("invalid_request"))
    );
    fake.add_client(Client::new(CLIENT_ID, CLIENT_SECRET).secret_optional());
    let optional = form(
        &fake,
        "/token",
        &[base[0], base[1], ("client_id", CLIENT_ID)],
    );
    assert_eq!(
        optional.json()["error"],
        "invalid_grant",
        "past the client check"
    );
}

#[test]
fn the_identity_token_is_rs256_by_a_listed_key_with_the_nonce_and_the_fake_clock() {
    let fake = fake();
    let listener = Listener::start();
    let code = code_for(&fake, &listener, Decision::allow());
    let answer = exchange(&fake, &code, &listener.redirect(), VERIFIER).json();
    let token = answer["id_token"].as_str().unwrap();
    let (header, payload, _, _) = jwt(token);
    assert_eq!(header["alg"], "RS256");
    assert_eq!(header["kid"], KeyId::A.kid());
    assert_eq!(payload["iss"], "https://accounts.google.com");
    assert_eq!(payload["aud"], CLIENT_ID);
    assert_eq!(payload["azp"], CLIENT_ID);
    assert_eq!(payload["sub"], ALICE);
    assert_eq!(payload["email"], ALICE_EMAIL);
    assert_eq!(payload["email_verified"], true);
    assert_eq!(payload["nonce"], "nonce-1");
    let now = fake.now_ms() / 1000;
    assert_eq!(payload["iat"], now);
    assert_eq!(payload["exp"], now + 3600);
    let keys = key_set(&fake).json();
    let jwk = keys["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["kid"] == header["kid"])
        .unwrap();
    assert!(verifies_with(jwk, token));
}

#[test]
fn signing_one_token_takes_well_under_a_second() {
    let started = std::time::Instant::now();
    let _ = fabric_google_fake::keys::sign_rs256(KeyId::A, b"warm up");
    let warm = std::time::Instant::now();
    let _ = fabric_google_fake::keys::sign_rs256(KeyId::A, b"header.payload");
    let one = warm.elapsed();
    eprintln!(
        "first RS256 signature (key built): {:?}; one more: {one:?}",
        warm - started
    );
    assert!(one < std::time::Duration::from_secs(1));
}

#[test]
fn the_authorization_endpoint_refuses_what_google_would_and_issues_nothing() {
    let fake = fake();
    let listener = Listener::start();
    let good = authorization_address(&fake, &listener.redirect(), "s", "n");
    let cases = [
        good.replace(&enc(CLIENT_ID), "nobody"),
        good.replace(&enc(&listener.redirect()), &enc("http://example.com:80")),
        good.replace("code_challenge_method=S256", "code_challenge_method=plain"),
        good.replace("&state=s", ""),
        format!("{good}&state=again"),
        format!("{good}&unknown=1"),
        good.replace("response_type=code", "response_type=token"),
        good.replace("scope=openid", "scope=bogus"),
    ];
    for address in cases {
        let answer = fake
            .browser()
            .authorize(&address, &Decision::allow())
            .unwrap();
        assert!(
            matches!(answer.status, 400 | 401),
            "{address}: {}",
            answer.status
        );
        assert!(answer.location.is_none());
        assert!(answer.body.contains("Error 40"));
    }
    assert_eq!(fake.with_state(|s| s.codes_issued()), 0);
}

#[test]
fn deny_untick_extra_close_and_sign_in_as_play_as_asked() {
    let fake = fake();
    let listener = Listener::start();
    let address = authorization_address(&fake, &listener.redirect(), "st", "nc");

    let denied = fake.browser().visit(&address, Decision::deny()).unwrap();
    assert_eq!(
        denied.redirect_parameter("error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(denied.redirect_parameter("state").as_deref(), Some("st"));
    assert!(denied.code().is_none());
    let _ = listener.next();

    let unticked = fake
        .browser()
        .visit(&address, Decision::allow().untick(&[SCOPE_EVENTS]))
        .unwrap();
    let _ = listener.next();
    let scope = unticked.redirect_parameter("scope").unwrap();
    assert!(!scope.contains(SCOPE_EVENTS));
    let answer = exchange(
        &fake,
        &unticked.code().unwrap(),
        &listener.redirect(),
        VERIFIER,
    )
    .json();
    assert!(!answer["scope"].as_str().unwrap().contains(SCOPE_EVENTS));

    let extra = "https://www.googleapis.com/auth/drive.readonly";
    let more = fake
        .browser()
        .visit(&address, Decision::allow().grant_extra(&[extra]))
        .unwrap();
    let _ = listener.next();
    assert!(more.redirect_parameter("scope").unwrap().contains(extra));

    let closed = fake
        .browser()
        .visit(&address, Decision::close_window())
        .unwrap();
    assert_eq!(closed.authorization.status, 200);
    assert!(closed.callback.is_none());
    assert!(
        listener.requests.try_recv().is_err(),
        "the listener heard nothing"
    );

    let bob = fake
        .browser()
        .visit(&address, Decision::allow().sign_in_as(BOB))
        .unwrap();
    let _ = listener.next();
    let answer = exchange(&fake, &bob.code().unwrap(), &listener.redirect(), VERIFIER).json();
    let (_, payload, _, _) = jwt(answer["id_token"].as_str().unwrap());
    assert_eq!(payload["sub"], BOB);
}

#[test]
fn a_tampered_callback_reaches_the_listener_as_asked() {
    let fake = fake();
    let listener = Listener::start();
    let address = authorization_address(&fake, &listener.redirect(), "right", "n");
    let cases: Vec<(Tamper, &str)> = vec![
        (Tamper::State("wrong".into()), "state=wrong"),
        (Tamper::DropParameter("state".into()), "GET /?iss="),
        (Tamper::DoubleParameter("code".into()), "&code="),
        (Tamper::Path("/favicon.ico".into()), "GET /favicon.ico?"),
        (Tamper::Method("POST".into()), "POST /?"),
        (
            Tamper::Host("evil.example:80".into()),
            "Host: evil.example:80",
        ),
        (
            Tamper::Iss("https://evil.example".into()),
            "iss=https%3A%2F%2Fevil.example",
        ),
    ];
    for (tamper, expected) in cases {
        let visit = fake
            .browser()
            .visit(&address, Decision::allow().tamper(tamper.clone()))
            .unwrap();
        let head = listener.next();
        assert!(head.contains(expected), "{tamper:?}: {head}");
        assert!(visit.listener_answer().is_some());
        if matches!(tamper, Tamper::DoubleParameter(_)) {
            assert_eq!(head.matches("&code=").count(), 2);
        }
    }
    // A second callback with the same address: the browser sends it again as asked.
    let visit = fake.browser().visit(&address, Decision::allow()).unwrap();
    let _ = listener.next();
    let again = fake
        .browser()
        .follow(visit.authorization.location.as_deref().unwrap(), &[])
        .unwrap();
    assert_eq!(again.status, 200);
    drop(listener);
    let refused = fake
        .browser()
        .follow(visit.authorization.location.as_deref().unwrap(), &[]);
    assert_eq!(refused, Err(fabric_google_fake::ClientError::Refused));
}

#[test]
fn the_browser_refuses_an_address_that_is_not_this_fakes_authorization_endpoint() {
    let fake = fake();
    let refused = fake.browser().authorize(
        "https://accounts.google.com/o/oauth2/v2/auth?x=1",
        &Decision::allow(),
    );
    assert_eq!(
        refused,
        Err(fabric_google_fake::BrowserError::NotTheAuthorizationEndpoint)
    );
    assert!(fake.log().is_empty(), "nothing was sent");
}

#[test]
fn refresh_issues_an_access_token_and_a_rotation_ends_the_old_refresh_token() {
    let fake = fake();
    let old = fake.issue_refresh_token(ALICE, CLIENT_ID, &["openid", "email", SCOPE_EVENTS]);
    let answer = refresh(&fake, &old);
    assert_eq!(answer.status, 200);
    let body = answer.json();
    assert!(body.get("refresh_token").is_none());
    let access = body["access_token"].as_str().unwrap().to_string();
    assert!(fake.with_state(|s| s.access_token_valid(&access)));
    let (_, payload, _, _) = jwt(body["id_token"].as_str().unwrap());
    assert!(payload.get("nonce").is_none(), "a refresh has no nonce");

    fake.arm(Fault::token_answer(TokenFault::RotateRefreshToken).grant_type("refresh_token"));
    let rotated = refresh(&fake, &old).json();
    let new = rotated["refresh_token"].as_str().unwrap().to_string();
    assert!(!fake.with_state(|s| s.refresh_token_valid(&old)));
    assert!(fake.with_state(|s| s.refresh_token_valid(&new)));
    assert_eq!(refresh(&fake, &old).json()["error"], "invalid_grant");
}

#[test]
fn token_times_run_on_the_fake_clock() {
    let fake = fake();
    fake.with_state(|s| s.set_refresh_token_lifetime(Some(7 * 86_400)));
    let refresh_token = fake.issue_refresh_token(ALICE, CLIENT_ID, &CALENDAR_SCOPES);
    let answer = refresh(&fake, &refresh_token).json();
    assert_eq!(answer["refresh_token_expires_in"], 7 * 86_400);
    let access = answer["access_token"].as_str().unwrap().to_string();
    let path = "/users/me/calendarList";
    assert_eq!(api(&fake, &access, "GET", path, &[], None).status, 200);
    fake.advance(std::time::Duration::from_secs(3599));
    let expired = api(&fake, &access, "GET", path, &[], None);
    assert_eq!(
        (expired.status, expired.reason().as_deref()),
        (401, Some("authError"))
    );
    fake.advance(std::time::Duration::from_secs(7 * 86_400));
    assert_eq!(
        refresh(&fake, &refresh_token).json()["error"],
        "invalid_grant"
    );
}

#[test]
fn a_revocation_ends_the_whole_grant_of_the_account_to_the_client() {
    let fake = fake();
    let one = fake.issue_refresh_token(ALICE, CLIENT_ID, &CALENDAR_SCOPES);
    let two = fake.issue_refresh_token(ALICE, CLIENT_ID, &CALENDAR_SCOPES);
    let access = token(&fake, ALICE);
    let bobs = fake.issue_refresh_token(BOB, CLIENT_ID, &CALENDAR_SCOPES);
    let answer = form(&fake, "/revoke", &[("token", &one)]);
    assert_eq!(answer.status, 200);
    fake.with_state(|s| {
        assert!(!s.refresh_token_valid(&one) && !s.refresh_token_valid(&two));
        assert!(!s.access_token_valid(&access));
        assert!(s.refresh_token_valid(&bobs));
    });
    let again = form(&fake, "/revoke", &[("token", &one)]);
    assert_eq!(
        (again.status, again.json()["error"].clone()),
        (400, json!("invalid_token"))
    );
    assert_eq!(fake.log().revocations().len(), 2);
    assert_eq!(
        fake.log().revocations().entries()[0]
            .token_account
            .as_deref(),
        Some(ALICE)
    );
}

#[test]
fn the_key_set_lists_a_key_from_the_fetch_it_was_published_for() {
    let fake = fake();
    fake.with_state(|s| {
        s.unpublish_key(KeyId::B);
        s.publish_key(KeyId::B, 1);
        s.set_key_set_max_age(1234);
    });
    let first = key_set(&fake);
    assert_eq!(
        first.header("Cache-Control"),
        Some("public, max-age=1234, must-revalidate, no-transform")
    );
    let kids = |a: &Answer| -> Vec<String> {
        a.json()["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["kid"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(kids(&first), vec![KeyId::A.kid()]);
    assert_eq!(kids(&key_set(&fake)), vec![KeyId::A.kid(), KeyId::B.kid()]);
    assert_eq!(fake.with_state(|s| s.key_set_fetches()), 2);
    let jwk = KeyId::A.jwk();
    assert_eq!(
        (jwk["kty"].as_str(), jwk["e"].as_str()),
        (Some("RSA"), Some("AQAB"))
    );
}

fn faulted_token(fake: &fabric_google_fake::FakeGoogle, fault: IdTokenFault) -> String {
    let refresh_token = fake.issue_refresh_token(ALICE, CLIENT_ID, &["openid", "email"]);
    fake.arm(Fault::id_token(fault));
    refresh(fake, &refresh_token).json()["id_token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn each_identity_token_fault_changes_what_it_names() {
    let fake = fake();
    let a = KeyId::A.jwk();
    let now = fake.now_ms() / 1000;

    let t = faulted_token(&fake, IdTokenFault::Issuer("https://evil.example".into()));
    assert_eq!(jwt(&t).1["iss"], "https://evil.example");
    assert!(verifies_with(&a, &t), "only the issuer differs");

    let t = faulted_token(&fake, IdTokenFault::Expired);
    assert!(jwt(&t).1["exp"].as_i64().unwrap() < now - 60);
    let t = faulted_token(&fake, IdTokenFault::IssuedInFuture);
    assert!(jwt(&t).1["iat"].as_i64().unwrap() > now + 60);
    let t = faulted_token(&fake, IdTokenFault::TooOld);
    assert!(jwt(&t).1["iat"].as_i64().unwrap() < now - 600);

    let t = faulted_token(&fake, IdTokenFault::BadSignature);
    assert!(!verifies_with(&a, &t));

    let t = faulted_token(&fake, IdTokenFault::AlgNone);
    assert_eq!(jwt(&t).0["alg"], "none");
    assert!(t.ends_with('.'));

    let t = faulted_token(&fake, IdTokenFault::AlgHs256);
    assert_eq!(
        (jwt(&t).0["alg"].clone(), jwt(&t).2.len()),
        (json!("HS256"), 32)
    );

    let t = faulted_token(&fake, IdTokenFault::NoKid);
    assert!(jwt(&t).0.get("kid").is_none());
    let t = faulted_token(&fake, IdTokenFault::UnknownKid);
    assert_eq!(jwt(&t).0["kid"], "fake-key-unknown");
    let t = faulted_token(&fake, IdTokenFault::Crit);
    assert!(jwt(&t).0.get("crit").is_some());

    let t = faulted_token(&fake, IdTokenFault::JwkHeader);
    let (header, ..) = jwt(&t);
    assert_eq!(header["kid"], KeyId::A.kid());
    assert!(!verifies_with(&a, &t));
    assert!(verifies_with(&header["jwk"], &t));

    let t = faulted_token(&fake, IdTokenFault::DoubledHeaderMember);
    let (header, _, _, text) = jwt(&t);
    assert_eq!(text.matches("\"alg\"").count(), 2);
    assert_eq!(header["alg"], "RS256", "the last one is the real one");
    assert!(verifies_with(&a, &t));

    let t = faulted_token(&fake, IdTokenFault::DoubledPayloadMember);
    let payload = String::from_utf8(
        URL_SAFE_NO_PAD
            .decode(t.split('.').nth(1).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload.matches("\"sub\"").count(), 2);

    let t = faulted_token(&fake, IdTokenFault::WeakExponentKey);
    let keys = key_set(&fake).json();
    let weak = keys["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["kid"] == KeyId::WeakExponent.kid())
        .cloned()
        .expect("the weak key is listed");
    assert_eq!(weak["e"], "Aw");
    assert!(verifies_with(&weak, &t));

    let t = faulted_token(&fake, IdTokenFault::ShortModulusKey);
    assert_eq!(jwt(&t).0["kid"], KeyId::ShortModulus.kid());
    assert_eq!(jwt(&t).2.len(), 128);

    let t = faulted_token(&fake, IdTokenFault::EmailUnverified);
    assert_eq!(jwt(&t).1["email_verified"], false);
    let t = faulted_token(&fake, IdTokenFault::NoNonce);
    assert!(jwt(&t).1.get("nonce").is_none());
    let t = faulted_token(&fake, IdTokenFault::Audience(json!(["other", CLIENT_ID])));
    assert_eq!(jwt(&t).1["aud"], json!(["other", CLIENT_ID]));
}

#[test]
fn token_faults_change_the_answer_and_errors_answer_in_the_oauth_shape() {
    let fake = fake();
    let refresh_token = fake.issue_refresh_token(ALICE, CLIENT_ID, &CALENDAR_SCOPES);
    fake.arm(
        Fault::token_answer(TokenFault::TokenType("MAC".into()))
            .and(FaultAction::Token(TokenFault::Scope("openid".into())))
            .and(FaultAction::Token(TokenFault::ExpiresIn(30)))
            .and(FaultAction::Token(TokenFault::DropMember(
                "expires_in".into(),
            ))),
    );
    let answer = refresh(&fake, &refresh_token).json();
    assert_eq!(answer["token_type"], "MAC");
    assert_eq!(answer["scope"], "openid");
    assert!(answer.get("expires_in").is_none());

    for (action, status, error) in [
        (FaultAction::invalid_grant(), 400, "invalid_grant"),
        (
            FaultAction::admin_policy_enforced(),
            400,
            "admin_policy_enforced",
        ),
        (FaultAction::invalid_client(), 401, "invalid_client"),
    ] {
        let handle = fake.arm(Fault::token(action));
        let answer = refresh(&fake, &refresh_token);
        assert_eq!(
            (answer.status, answer.json()["error"].clone()),
            (status, json!(error))
        );
        assert_eq!(handle.fired(), 1);
    }
    assert_eq!(
        refresh(&fake, &refresh_token).status,
        200,
        "each fault fired once"
    );
}

#[test]
fn a_code_exchange_without_a_refresh_token_issues_none() {
    let fake = fake();
    let listener = Listener::start();
    let code = code_for(&fake, &listener, Decision::allow());
    fake.arm(Fault::token_answer(TokenFault::NoRefreshToken));
    let answer = exchange(&fake, &code, &listener.redirect(), VERIFIER).json();
    assert!(answer.get("refresh_token").is_none());
    assert!(fake.with_state(|s| s.valid_refresh_tokens(ALICE).is_empty()));
}
