//! The sign-in side of the fake: the authorization endpoint, the token endpoint, revocation,
//! the key set, and the identity tokens it signs (3.2 to 3.9, 14.1, 14.2).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::browser::Choice;
use crate::faults::{IdTokenFault, Plan, TokenFault};
use crate::keys::{self, KeyId};
use crate::server::{Request, Response};
use crate::state::{
    Code, ISSUER, RefreshToken, SCOPE_EMAIL, SCOPE_PROFILE, SCOPE_USERINFO_EMAIL,
    SCOPE_USERINFO_PROFILE, ScopeSpelling, State,
};
use crate::url::{HttpAddress, encode_pairs};

/// The parameters the authorization endpoint knows. Any other is refused (G-DOC lists these;
/// Google answers "Parameter not allowed" for others).
const AUTHORIZATION_PARAMETERS: [&str; 14] = [
    "client_id",
    "redirect_uri",
    "response_type",
    "scope",
    "code_challenge",
    "code_challenge_method",
    "state",
    "nonce",
    "prompt",
    "login_hint",
    "access_type",
    "include_granted_scopes",
    "hd",
    "enable_granular_consent",
];

/// How long an authorization code lives on the fake clock.
const CODE_LIFETIME_MS: i64 = 600_000;

/// The consent page the authorization endpoint shows while nobody has decided.
pub(crate) const CONSENT_PAGE: &str = "Fake Google: choose an account and allow or deny access.";

fn error_page(status: u16, error: &str) -> Response {
    Response::html(status, &format!("Error {status}: {error}"))
}

/// `GET /o/oauth2/v2/auth`: checks every parameter as Google would, then plays the decision
/// the browser left (14.1): a redirect to the loopback `redirect_uri` with a code or an error,
/// or the consent page when the window is closed or nobody decided.
pub(crate) fn authorize(state: &mut State, request: &Request) -> Response {
    let decision = state.pending_decision.take();
    let mut seen: Vec<&str> = Vec::new();
    for (name, _) in &request.query {
        if seen.contains(&name.as_str()) {
            return error_page(400, "invalid_request");
        }
        if !AUTHORIZATION_PARAMETERS.contains(&name.as_str()) {
            return error_page(400, "invalid_request");
        }
        seen.push(name);
    }
    let param = |name: &str| {
        request
            .query
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let Some(client_id) = param("client_id") else {
        return error_page(400, "invalid_request");
    };
    if state.client(client_id).is_none() {
        return error_page(401, "invalid_client");
    }
    let Some(redirect_uri) = param("redirect_uri") else {
        return error_page(400, "invalid_request");
    };
    let Some(redirect) = HttpAddress::parse(redirect_uri) else {
        return error_page(400, "redirect_uri_mismatch");
    };
    if !matches!(redirect.host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
        || !redirect.query.is_empty()
    {
        return error_page(400, "redirect_uri_mismatch");
    }
    if param("response_type") != Some("code") {
        return error_page(400, "unsupported_response_type");
    }
    let scopes: Vec<String> = param("scope")
        .unwrap_or("")
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if scopes.is_empty() || !scopes.iter().all(|s| known_scope(s)) {
        return error_page(400, "invalid_scope");
    }
    // Stricter than Google, which also takes `plain` or no challenge: the adapter must always
    // send S256 (3.2.1), and the fake is here to notice when it does not.
    let challenge = param("code_challenge").unwrap_or("");
    if !(43..=128).contains(&challenge.len())
        || !challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
        || param("code_challenge_method") != Some("S256")
    {
        return error_page(400, "invalid_request");
    }
    // Also stricter than Google: the adapter always sends a state (3.2.1).
    let Some(oauth_state) = param("state").filter(|s| !s.is_empty()) else {
        return error_page(400, "invalid_request");
    };
    if let Some(prompt) = param("prompt")
        && !prompt
            .split(' ')
            .all(|p| matches!(p, "none" | "consent" | "select_account"))
    {
        return error_page(400, "invalid_request");
    }

    let Some(decision) = decision else {
        return Response::html(200, CONSENT_PAGE);
    };
    let mut target = redirect_uri.to_string();
    if redirect.path == "/" && !redirect_uri.ends_with('/') {
        target.push('/');
    }
    let pairs: Vec<(String, String)> = match decision.choice {
        Choice::CloseWindow => return Response::html(200, CONSENT_PAGE),
        Choice::Deny => vec![
            ("error".into(), "access_denied".into()),
            ("state".into(), oauth_state.into()),
            ("iss".into(), ISSUER.into()),
        ],
        Choice::Allow => {
            let account = decision
                .account
                .clone()
                .or_else(|| {
                    param("login_hint").and_then(|hint| {
                        state
                            .accounts
                            .iter()
                            .find(|a| a.subject == hint || a.email.eq_ignore_ascii_case(hint))
                            .map(|a| a.subject.clone())
                    })
                })
                .or_else(|| state.browser_account.clone())
                .or_else(|| state.accounts.first().map(|a| a.subject.clone()));
            let Some(subject) = account.filter(|s| state.account(s).is_some()) else {
                return Response::html(200, "Fake Google: no account to sign in with.");
            };
            let mut granted: Vec<String> = scopes
                .iter()
                .filter(|s| !decision.untick.contains(s))
                .cloned()
                .collect();
            for extra in &decision.extra {
                if !granted.contains(extra) {
                    granted.push(extra.clone());
                }
            }
            let code = format!("4/0fake-code-{}", state.next_secret("code"));
            state.codes.push(Code {
                code: code.clone(),
                client_id: client_id.to_string(),
                redirect_uri: redirect_uri.to_string(),
                challenge: challenge.to_string(),
                scopes: granted.clone(),
                nonce: param("nonce").map(str::to_string),
                subject,
                ends_ms: state.now_ms + CODE_LIFETIME_MS,
                used: false,
            });
            vec![
                ("state".into(), oauth_state.into()),
                ("iss".into(), ISSUER.into()),
                ("code".into(), code),
                ("scope".into(), spell_scopes(state.scope_spelling, &granted)),
                ("authuser".into(), "0".into()),
                ("prompt".into(), "consent".into()),
            ]
        }
    };
    let query = encode_pairs(pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())));
    let location = format!("{target}?{query}");
    Response::html(302, "Moved Temporarily").header("Location", &location)
}

fn known_scope(scope: &str) -> bool {
    matches!(scope, "openid" | "email" | "profile")
        || scope
            .strip_prefix("https://www.googleapis.com/auth/")
            .is_some_and(|rest| !rest.is_empty())
}

fn spell_scopes(spelling: ScopeSpelling, scopes: &[String]) -> String {
    scopes
        .iter()
        .map(|s| match (spelling, s.as_str()) {
            (ScopeSpelling::Google, SCOPE_EMAIL) => SCOPE_USERINFO_EMAIL,
            (ScopeSpelling::Google, SCOPE_PROFILE) => SCOPE_USERINFO_PROFILE,
            _ => s.as_str(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn token_answer(value: &Value) -> Response {
    Response::json(200, value)
        .header(
            "Cache-Control",
            "no-cache, no-store, max-age=0, must-revalidate",
        )
        .header("Pragma", "no-cache")
}

/// `POST /token`: the code exchange (3.4) and the refresh (3.8).
pub(crate) fn token(state: &mut State, request: &Request, plan: &Plan) -> Response {
    let Some(form) = request.form() else {
        return Response::oauth_error(400, "invalid_request", "Bad Request");
    };
    let mut seen: Vec<&str> = Vec::new();
    for (name, _) in &form {
        if seen.contains(&name.as_str()) {
            return Response::oauth_error(400, "invalid_request", "Duplicate parameter.");
        }
        seen.push(name);
    }
    let field = |name: &str| form.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
    let Some(client) = field("client_id").and_then(|id| state.client(&id).cloned()) else {
        return Response::oauth_error(401, "invalid_client", "The OAuth client was not found.");
    };
    match field("client_secret") {
        Some(secret) if secret != client.secret => {
            return Response::oauth_error(401, "invalid_client", "Unauthorized");
        }
        None if client.secret_required => {
            return Response::oauth_error(400, "invalid_request", "client_secret is missing.");
        }
        _ => {}
    }
    match field("grant_type").as_deref() {
        Some("authorization_code") => exchange_code(state, &client.id, &field, plan),
        Some("refresh_token") => refresh(state, &client.id, &field, plan),
        Some(_) => Response::oauth_error(400, "unsupported_grant_type", "Invalid grant_type."),
        None => Response::oauth_error(
            400,
            "invalid_request",
            "Missing required parameter: grant_type",
        ),
    }
}

fn exchange_code(
    state: &mut State,
    client_id: &str,
    field: &dyn Fn(&str) -> Option<String>,
    plan: &Plan,
) -> Response {
    let Some(presented) = field("code") else {
        return Response::oauth_error(400, "invalid_request", "Missing required parameter: code");
    };
    let now = state.now_ms;
    let Some(code) = state.codes.iter_mut().find(|c| c.code == presented) else {
        return Response::oauth_error(400, "invalid_grant", "Malformed auth code.");
    };
    // A code is used by its first presentation, whatever comes of it.
    let first_use = !code.used;
    code.used = true;
    if !first_use || code.ends_ms <= now || code.client_id != client_id {
        return Response::oauth_error(400, "invalid_grant", "Bad Request");
    }
    if field("redirect_uri").as_deref() != Some(code.redirect_uri.as_str()) {
        return Response::oauth_error(400, "redirect_uri_mismatch", "Bad Request");
    }
    let Some(verifier) = field("code_verifier") else {
        return Response::oauth_error(400, "invalid_grant", "Missing code verifier.");
    };
    if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != code.challenge {
        return Response::oauth_error(400, "invalid_grant", "Invalid code verifier.");
    }
    let subject = code.subject.clone();
    let scopes = code.scopes.clone();
    let nonce = code.nonce.clone();
    issue(
        state,
        client_id,
        &subject,
        scopes,
        nonce.as_deref(),
        None,
        plan,
    )
}

fn refresh(
    state: &mut State,
    client_id: &str,
    field: &dyn Fn(&str) -> Option<String>,
    plan: &Plan,
) -> Response {
    let Some(presented) = field("refresh_token") else {
        return Response::oauth_error(
            400,
            "invalid_request",
            "Missing required parameter: refresh_token",
        );
    };
    let Some(index) = state
        .refresh_tokens
        .iter()
        .position(|t| t.token == presented && state.refresh_usable(t))
    else {
        return Response::oauth_error(400, "invalid_grant", "Token has been expired or revoked.");
    };
    if state.refresh_tokens[index].client_id != client_id {
        return Response::oauth_error(400, "invalid_grant", "Unauthorized");
    }
    let subject = state.refresh_tokens[index].subject.clone();
    let scopes = state.refresh_tokens[index].scopes.clone();
    issue(state, client_id, &subject, scopes, None, Some(index), plan)
}

/// Issues the tokens of a token answer: a new refresh token for a code exchange (`refreshing`
/// is `None`), or the access token of a refresh, with the token faults of `plan` applied.
fn issue(
    state: &mut State,
    client_id: &str,
    subject: &str,
    scopes: Vec<String>,
    nonce: Option<&str>,
    refreshing: Option<usize>,
    plan: &Plan,
) -> Response {
    let mut access_scopes = scopes.clone();
    let mut scope_text = spell_scopes(state.scope_spelling, &scopes);
    let mut expires_in = state.access_token_lifetime_s;
    let mut refresh_lifetime = state.refresh_token_lifetime_s;
    let mut no_refresh = false;
    let mut no_id_token = false;
    let mut rotate = false;
    let mut token_type = "Bearer".to_string();
    for fault in &plan.token {
        match fault {
            TokenFault::Scope(s) => {
                scope_text = s.clone();
                access_scopes = s
                    .split(' ')
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            TokenFault::ExpiresIn(n) => expires_in = *n,
            TokenFault::RefreshTokenExpiresIn(n) => refresh_lifetime = Some(*n),
            TokenFault::NoRefreshToken => no_refresh = true,
            TokenFault::NoIdToken => no_id_token = true,
            TokenFault::RotateRefreshToken => rotate = true,
            TokenFault::TokenType(t) => token_type = t.clone(),
            TokenFault::Member(..) | TokenFault::DropMember(_) => {}
        }
    }
    let access_refs: Vec<&str> = access_scopes.iter().map(String::as_str).collect();
    let access_token = state.issue_access_token(subject, client_id, &access_refs, expires_in);
    let mut answer = Map::new();
    answer.insert("access_token".into(), json!(access_token));
    answer.insert("expires_in".into(), json!(expires_in));
    answer.insert("scope".into(), json!(scope_text));
    answer.insert("token_type".into(), json!(token_type));

    let new_refresh = match refreshing {
        None => !no_refresh,
        Some(_) => rotate && !no_refresh,
    };
    if new_refresh {
        let token = format!("1//fake-refresh-{}", state.next_secret("refresh"));
        state.refresh_tokens.push(RefreshToken {
            token: token.clone(),
            subject: subject.to_string(),
            client_id: client_id.to_string(),
            scopes: access_scopes.clone(),
            ends_ms: refresh_lifetime.map(|s| state.now_ms + s * 1000),
            revoked: false,
        });
        if let Some(old) = refreshing {
            state.refresh_tokens[old].revoked = true;
        }
        answer.insert("refresh_token".into(), json!(token));
        if let Some(s) = refresh_lifetime {
            answer.insert("refresh_token_expires_in".into(), json!(s));
        }
    } else if let Some(index) = refreshing
        && let Some(end) = state.refresh_tokens[index].ends_ms
    {
        answer.insert(
            "refresh_token_expires_in".into(),
            json!((end - state.now_ms).max(0) / 1000),
        );
    }
    let has_openid = scopes.iter().any(|s| s == "openid");
    if has_openid && !no_id_token {
        let with_email = scopes
            .iter()
            .any(|s| s == SCOPE_EMAIL || s == SCOPE_USERINFO_EMAIL);
        let token = id_token(
            state,
            subject,
            client_id,
            nonce,
            &access_token,
            with_email,
            &plan.id_token,
        );
        answer.insert("id_token".into(), json!(token));
    }
    for fault in &plan.token {
        match fault {
            TokenFault::Member(name, value) => {
                answer.insert(name.clone(), value.clone());
            }
            TokenFault::DropMember(name) => {
                answer.remove(name);
            }
            _ => {}
        }
    }
    token_answer(&Value::Object(answer))
}

/// `POST /revoke` (3.9): a known refresh or access token ends the whole grant of its account
/// to its client (FA14) and answers 200; anything else answers 400 `invalid_token`.
pub(crate) fn revoke(state: &mut State, request: &Request) -> Response {
    let from_form = request
        .form()
        .and_then(|f| f.into_iter().find(|(n, _)| n == "token").map(|(_, v)| v));
    let from_query = request.query_all("token").first().map(|s| s.to_string());
    let Some(token) = from_form.or(from_query) else {
        return Response::oauth_error(400, "invalid_request", "Missing required parameter: token");
    };
    let now = state.now_ms;
    let owner = state
        .refresh_tokens
        .iter()
        .find(|t| t.token == token && state.refresh_usable(t))
        .map(|t| (t.subject.clone(), t.client_id.clone()))
        .or_else(|| {
            state
                .access_tokens
                .iter()
                .find(|t| t.token == token && !t.revoked && t.ends_ms > now)
                .map(|t| (t.subject.clone(), t.client_id.clone()))
        });
    match owner {
        Some((subject, client)) => {
            state.revoke_grant(&subject, &client);
            Response::json(200, &json!({}))
        }
        None => Response::oauth_error(400, "invalid_token", "Token expired or revoked"),
    }
}

/// `GET /oauth2/v3/certs`: the keys published for this fetch, with `Cache-Control: max-age`.
pub(crate) fn certs(state: &mut State) -> Response {
    let fetch = state.key_set_fetches;
    state.key_set_fetches += 1;
    let keys: Vec<Value> = state
        .published_keys
        .iter()
        .filter(|(_, from)| *from <= fetch)
        .map(|(k, _)| k.jwk())
        .collect();
    Response::json(200, &json!({"keys": keys})).header(
        "Cache-Control",
        &format!(
            "public, max-age={}, must-revalidate, no-transform",
            state.key_set_max_age_s
        ),
    )
}

enum Signing {
    Rs256,
    Hs256,
    None,
}

/// An identity token for `subject`, signed RS256 with the signing key, with the faults of
/// `faults` applied (3.5, 14.2). Claims: `iss`, `azp`, `aud`, `sub`, `email` and
/// `email_verified` when the address scope was granted, `at_hash`, `nonce` when the
/// authorization request had one, `iat` and `exp` (one hour) on the fake clock.
pub(crate) fn id_token(
    state: &mut State,
    subject: &str,
    client_id: &str,
    nonce: Option<&str>,
    access_token: &str,
    with_email: bool,
    faults: &[IdTokenFault],
) -> String {
    let now = state.now_ms.div_euclid(1000);
    let mut claims = Map::new();
    claims.insert("iss".into(), json!(ISSUER));
    claims.insert("azp".into(), json!(client_id));
    claims.insert("aud".into(), json!(client_id));
    claims.insert("sub".into(), json!(subject));
    if with_email && let Some(account) = state.account(subject) {
        claims.insert("email".into(), json!(account.email));
        claims.insert("email_verified".into(), json!(account.email_verified));
    }
    let digest = Sha256::digest(access_token.as_bytes());
    claims.insert(
        "at_hash".into(),
        json!(URL_SAFE_NO_PAD.encode(&digest[..16])),
    );
    if let Some(n) = nonce {
        claims.insert("nonce".into(), json!(n));
    }
    claims.insert("iat".into(), json!(now));
    claims.insert("exp".into(), json!(now + 3600));

    let mut key = state.signing_key;
    let mut kid: Option<String> = Some(key.kid().to_string());
    let mut header_extra = Map::new();
    let mut signing = Signing::Rs256;
    let mut bad_signature = false;
    let mut doubled_header = false;
    let mut doubled_payload = false;
    for fault in faults {
        match fault {
            IdTokenFault::Issuer(s) => {
                claims.insert("iss".into(), json!(s));
            }
            IdTokenFault::Audience(v) => {
                claims.insert("aud".into(), v.clone());
            }
            IdTokenFault::Azp(s) => {
                claims.insert("azp".into(), json!(s));
            }
            IdTokenFault::Nonce(s) => {
                claims.insert("nonce".into(), json!(s));
            }
            IdTokenFault::NoNonce => {
                claims.remove("nonce");
            }
            IdTokenFault::Expired => {
                claims.insert("iat".into(), json!(now - 300));
                claims.insert("exp".into(), json!(now - 120));
            }
            IdTokenFault::IssuedInFuture => {
                claims.insert("iat".into(), json!(now + 300));
                claims.insert("exp".into(), json!(now + 3900));
            }
            IdTokenFault::TooOld => {
                claims.insert("iat".into(), json!(now - 1200));
                claims.insert("exp".into(), json!(now + 2400));
            }
            IdTokenFault::BadSignature => bad_signature = true,
            IdTokenFault::AlgNone => signing = Signing::None,
            IdTokenFault::AlgHs256 => signing = Signing::Hs256,
            IdTokenFault::UnknownKid => kid = Some("fake-key-unknown".into()),
            IdTokenFault::NoKid => kid = None,
            IdTokenFault::Kid(s) => kid = Some(s.clone()),
            IdTokenFault::Crit => {
                header_extra.insert("crit".into(), json!(["fabric-fake"]));
                header_extra.insert("fabric-fake".into(), json!(true));
            }
            IdTokenFault::JwkHeader => {
                // The attacker's key in the header, the ordinary key's id: a reader that takes
                // the key from the header accepts it, one that uses the key set does not.
                header_extra.insert("jwk".into(), KeyId::Attacker.jwk());
                key = KeyId::Attacker;
            }
            IdTokenFault::DoubledHeaderMember => doubled_header = true,
            IdTokenFault::DoubledPayloadMember => doubled_payload = true,
            IdTokenFault::WeakExponentKey => {
                if !state
                    .published_keys
                    .iter()
                    .any(|(k, _)| *k == KeyId::WeakExponent)
                {
                    state.publish_key(KeyId::WeakExponent, 0);
                }
                key = KeyId::WeakExponent;
                kid = Some(key.kid().into());
            }
            IdTokenFault::ShortModulusKey => {
                if !state
                    .published_keys
                    .iter()
                    .any(|(k, _)| *k == KeyId::ShortModulus)
                {
                    state.publish_key(KeyId::ShortModulus, 0);
                }
                key = KeyId::ShortModulus;
                kid = Some(key.kid().into());
            }
            IdTokenFault::SignWith(k) => {
                key = *k;
                kid = Some(k.kid().into());
            }
            IdTokenFault::EmailUnverified => {
                claims.insert("email_verified".into(), json!(false));
            }
            IdTokenFault::Email(s) => {
                claims.insert("email".into(), json!(s));
            }
            IdTokenFault::Subject(s) => {
                claims.insert("sub".into(), json!(s));
            }
            IdTokenFault::Claim(n, v) => {
                claims.insert(n.clone(), v.clone());
            }
            IdTokenFault::DropClaim(n) => {
                claims.remove(n);
            }
        }
    }
    let mut header = Map::new();
    header.insert(
        "alg".into(),
        json!(match signing {
            Signing::Rs256 => "RS256",
            Signing::Hs256 => "HS256",
            Signing::None => "none",
        }),
    );
    if let Some(kid) = kid {
        header.insert("kid".into(), json!(kid));
    }
    header.insert("typ".into(), json!("JWT"));
    header.extend(header_extra);
    let mut header_text = Value::Object(header).to_string();
    if doubled_header {
        // A strict reader refuses the doubled member; a reader that keeps the last one sees
        // the real `alg`.
        header_text = format!("{{\"alg\":\"HS256\",{}", &header_text[1..]);
    }
    let mut payload_text = Value::Object(claims).to_string();
    if doubled_payload {
        payload_text = format!("{{\"sub\":\"fake-other-subject\",{}", &payload_text[1..]);
    }
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header_text.as_bytes()),
        URL_SAFE_NO_PAD.encode(payload_text.as_bytes())
    );
    let signed = if bad_signature {
        format!("{input}.tampered")
    } else {
        input.clone()
    };
    let signature = match signing {
        Signing::Rs256 => keys::sign_rs256(key, signed.as_bytes()),
        Signing::Hs256 => hmac_sha256(&key.modulus(), signed.as_bytes()).to_vec(),
        Signing::None => Vec::new(),
    };
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = if key.len() > 64 {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    block.resize(64, 0);
    let mut inner = Sha256::new();
    inner.update(block.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
    outer.update(inner);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
