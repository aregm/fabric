//! The identity token (3.5, PD4) and Google's key set.
//!
//! The ten checks of 3.5, each failure a fixed [`IdentityProblem`], never a part of the token:
//!
//! 1. Three base64url parts, at most [`MAX_TOKEN_LEN`] bytes; header and payload are JSON
//!    objects read by the contract's strict reader, which refuses a member that appears twice
//!    ([`decode`]).
//! 2. Header `alg` is exactly `RS256`, checked before any key is looked at; a header with
//!    `crit` is refused. `jku`, `jwk`, `x5u` and `x5c` are never read ([`check_header`]).
//! 3. Header `kid` names a usable key of Google's key set ([`KeyCache`], [`parse_key_set`]):
//!    `kty` `RSA`, `alg` absent or `RS256`, `use` absent or `sig`, a modulus of 2048 to 8192
//!    bits, the exponent 65537. The key set is kept for the `max-age` of its `Cache-Control`
//!    held to 5 minutes..24 hours ([`cache_lifetime_ms`]), and fetched once more when a `kid` is
//!    unknown, at most once a minute.
//! 4. The signature over `header.payload` verifies with that key: RSASSA-PKCS1-v1_5 with
//!    SHA-256 and its DigestInfo prefix ([`verify_signature`]).
//! 5. to 10. The claims ([`check_claims`]): issuer, audience and `azp`, times, nonce (in
//!    constant time), subject, verified address that fits the contract's `email`.
//!
//! `at_hash` is not checked (3.5). The key set is the only source of a key. No error of the
//! JSON, base64 or RSA libraries is kept or formatted (PD27).

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fabric_desktop_contracts::json::parse_strict;
use fabric_desktop_contracts::scalar::Email;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::sha2::{Digest, Sha256};
use rsa::{BoxedUint, PublicKey};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::http::{BodyBound, Exchange, Method, Request, Target, Transport};
use crate::secret::Secret;

use crate::secret::wipe::WipedJson;

/// The longest identity token read, in bytes (3.5 item 1).
pub const MAX_TOKEN_LEN: usize = 16 * 1024;

/// The issuers 3.5 item 5 accepts.
pub const ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

/// The shortest and longest time the key set is kept, in seconds (3.5 item 3).
pub const KEY_SET_LIFETIME_BOUNDS_S: (u64, u64) = (300, 86_400);

/// The least time between two fetches made because a `kid` was unknown, in ms (3.5 item 3).
pub const UNKNOWN_KID_REFETCH_MS: u64 = 60_000;

/// The modulus sizes accepted, in bits (3.5 item 3).
pub const MODULUS_BITS: (usize, usize) = (2048, 8192);

/// Why an identity token was refused (3.5). Fixed labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityProblem {
    /// The token answer had no `id_token` (3.4).
    NoIdToken,
    /// `openid` was not granted, so there is no usable identity token (3.6).
    NoOpenId,
    /// Longer than [`MAX_TOKEN_LEN`] (item 1).
    TooLong,
    /// Not three parts of base64url without padding (item 1).
    NotThreeParts,
    /// The header is not a strict JSON object (item 1).
    Header,
    /// The payload is not a strict JSON object (item 1).
    Payload,
    /// `alg` is not exactly `RS256` (item 2).
    Algorithm,
    /// The header has `crit` (item 2).
    Critical,
    /// No `kid`, or one that is not a short string (item 3).
    NoKeyId,
    /// The `kid` names no key of the key set, after the one refetch allowed (item 3).
    UnknownKey,
    /// The `kid` names a key that is not usable: another type, algorithm or use, a modulus
    /// outside 2048..8192 bits, an exponent other than 65537, or a doubled `kid` (item 3).
    KeyNotUsable,
    /// The signature does not verify (item 4).
    Signature,
    /// `iss` is not Google's (item 5).
    Issuer,
    /// `aud` is not the client id (item 6).
    Audience,
    /// `azp` is present and not the client id (item 6).
    AuthorizedParty,
    /// `exp` or `iat` missing, of the wrong type, or outside the bounds (item 7).
    Times,
    /// `nonce` missing or not the flow's (item 8).
    Nonce,
    /// `sub` missing or not 1 to 255 printable ASCII characters (item 9).
    Subject,
    /// `email_verified` is not `true` (item 10).
    EmailNotVerified,
    /// `email` missing or not one the contract's `email` can hold (item 10).
    EmailNotUsable,
}

impl IdentityProblem {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            IdentityProblem::NoIdToken => "the token answer has no identity token",
            IdentityProblem::NoOpenId => "openid was not granted",
            IdentityProblem::TooLong => "the identity token is too long",
            IdentityProblem::NotThreeParts => "the identity token is not three base64url parts",
            IdentityProblem::Header => "the identity token's header is not a strict JSON object",
            IdentityProblem::Payload => "the identity token's payload is not a strict JSON object",
            IdentityProblem::Algorithm => "the identity token's algorithm is not RS256",
            IdentityProblem::Critical => "the identity token's header has crit",
            IdentityProblem::NoKeyId => "the identity token names no key",
            IdentityProblem::UnknownKey => "the identity token's key is not in Google's key set",
            IdentityProblem::KeyNotUsable => "the identity token's key is not usable",
            IdentityProblem::Signature => "the identity token's signature does not verify",
            IdentityProblem::Issuer => "the identity token's issuer is not Google",
            IdentityProblem::Audience => "the identity token is not for this client",
            IdentityProblem::AuthorizedParty => "the identity token's azp is not this client",
            IdentityProblem::Times => "the identity token's times are not acceptable",
            IdentityProblem::Nonce => "the identity token's nonce is not this sign-in's",
            IdentityProblem::Subject => "the identity token's subject is not usable",
            IdentityProblem::EmailNotVerified => "the account's address is not verified",
            IdentityProblem::EmailNotUsable => "the account's address cannot be held",
        }
    }
}

impl fmt::Display for IdentityProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why the identity check did not pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckFailure {
    /// A check failed (`Cancelled(IdentityCheckFailed)`).
    Identity(IdentityProblem),
    /// The key set could not be fetched (`Cancelled(Network)`).
    KeysUnavailable,
}

/// Who signed in, as the identity token says (3.5 items 9 and 10).
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    /// `sub`.
    pub subject: String,
    /// `email`, its domain in lower case.
    pub email: Email,
}

/// Prints nothing of the subject or the address.
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Identity([redacted])")
    }
}

/// A token split and read (item 1).
pub struct Decoded {
    /// The header object.
    header: WipedJson,
    /// The payload object.
    payload: WipedJson,
    /// `header.payload` as it appeared, the signed bytes.
    signing_input: Zeroizing<String>,
    /// The signature bytes.
    signature: Vec<u8>,
}

/// Prints nothing.
impl fmt::Debug for Decoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Decoded([redacted])")
    }
}

fn base64url(part: &str) -> Option<Zeroizing<Vec<u8>>> {
    URL_SAFE_NO_PAD.decode(part).ok().map(Zeroizing::new)
}

/// Item 1: splits and reads the token.
pub fn decode(token: &str) -> Result<Decoded, IdentityProblem> {
    if token.len() > MAX_TOKEN_LEN {
        return Err(IdentityProblem::TooLong);
    }
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(IdentityProblem::NotThreeParts);
    };
    // An empty signature is let through to item 2, so that `alg: none` is refused as an
    // algorithm; it can never verify.
    if h.is_empty() || p.is_empty() {
        return Err(IdentityProblem::NotThreeParts);
    }
    let header_bytes = base64url(h).ok_or(IdentityProblem::NotThreeParts)?;
    let payload_bytes = base64url(p).ok_or(IdentityProblem::NotThreeParts)?;
    let signature = base64url(s).ok_or(IdentityProblem::NotThreeParts)?;
    let header = parse_strict(&header_bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or(IdentityProblem::Header)?;
    let header = WipedJson(header);
    let payload = parse_strict(&payload_bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or(IdentityProblem::Payload)?;
    let payload = WipedJson(payload);
    Ok(Decoded {
        header,
        payload,
        signing_input: Zeroizing::new(format!("{h}.{p}")),
        signature: signature.to_vec(),
    })
}

/// Items 2 and the first half of 3: `alg` is exactly `RS256`, no `crit`, and the `kid`.
pub fn check_header(header: &Value) -> Result<String, IdentityProblem> {
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(IdentityProblem::Algorithm);
    }
    if header.get("crit").is_some() {
        return Err(IdentityProblem::Critical);
    }
    match header.get("kid").and_then(Value::as_str) {
        Some(kid) if !kid.is_empty() && kid.len() <= 256 => Ok(kid.to_owned()),
        _ => Err(IdentityProblem::NoKeyId),
    }
}

/// Item 4: RSASSA-PKCS1-v1_5 with SHA-256 and its DigestInfo prefix.
pub fn verify_signature(
    key: &PublicKey,
    signing_input: &[u8],
    signature: &[u8],
) -> Result<(), IdentityProblem> {
    let digest = Sha256::digest(signing_input);
    key.verify(Pkcs1v15Sign::new::<Sha256>(), &digest, signature)
        .map_err(|_| IdentityProblem::Signature)
}

/// What the claims must match (items 6 to 8).
pub struct Expected<'a> {
    /// The client id (item 6).
    pub client_id: &'a str,
    /// The flow's nonce (item 8).
    pub nonce: &'a Secret,
    /// Now, in Unix ms on the crate's clock (item 7).
    pub now_ms: u64,
}

/// Items 5 to 10.
pub fn check_claims(payload: &Value, expected: &Expected<'_>) -> Result<Identity, IdentityProblem> {
    let text = |name: &str| payload.get(name).and_then(Value::as_str);
    if !text("iss").is_some_and(|iss| ISSUERS.contains(&iss)) {
        return Err(IdentityProblem::Issuer);
    }
    let audience_ok = match payload.get("aud") {
        Some(Value::String(aud)) => aud == expected.client_id,
        Some(Value::Array(items)) => {
            items.len() == 1 && items[0].as_str() == Some(expected.client_id)
        }
        _ => false,
    };
    if !audience_ok {
        return Err(IdentityProblem::Audience);
    }
    if let Some(azp) = payload.get("azp")
        && azp.as_str() != Some(expected.client_id)
    {
        return Err(IdentityProblem::AuthorizedParty);
    }
    let (Some(exp), Some(iat)) = (
        payload.get("exp").and_then(Value::as_i64),
        payload.get("iat").and_then(Value::as_i64),
    ) else {
        return Err(IdentityProblem::Times);
    };
    let now_s = i64::try_from(expected.now_ms / 1000).unwrap_or(i64::MAX);
    if exp <= iat || exp <= now_s - 60 || iat > now_s + 60 || iat < now_s - 600 {
        return Err(IdentityProblem::Times);
    }
    if !text("nonce").is_some_and(|nonce| expected.nonce.ct_eq(nonce)) {
        return Err(IdentityProblem::Nonce);
    }
    let subject = text("sub")
        .filter(|sub| {
            (1..=255).contains(&sub.len()) && sub.bytes().all(|b| (b'!'..=b'~').contains(&b))
        })
        .ok_or(IdentityProblem::Subject)?;
    if payload.get("email_verified") != Some(&Value::Bool(true)) {
        return Err(IdentityProblem::EmailNotVerified);
    }
    let email = text("email")
        .and_then(|email| {
            let (local, domain) = email.split_once('@')?;
            let candidate = format!("{local}@{}", domain.to_ascii_lowercase());
            Email::is_valid(&candidate).then_some(candidate)
        })
        .and_then(|candidate| Email::new(candidate).ok())
        .ok_or(IdentityProblem::EmailNotUsable)?;
    Ok(Identity {
        subject: subject.to_owned(),
        email,
    })
}

/// How long a fetched key set is kept: its `max-age` held to 5 minutes..24 hours; 5 minutes
/// when there is none (3.5 item 3).
#[must_use]
pub fn cache_lifetime_ms(max_age_s: Option<u64>) -> u64 {
    let (low, high) = KEY_SET_LIFETIME_BOUNDS_S;
    max_age_s.unwrap_or(low).clamp(low, high) * 1000
}

/// The keys of one key set: the usable ones by `kid`, and the `kid`s of the refused ones.
#[derive(Default)]
pub struct KeySet {
    usable: Vec<(String, PublicKey)>,
    refused: Vec<String>,
}

impl KeySet {
    /// The usable key with this `kid`.
    #[must_use]
    pub fn find(&self, kid: &str) -> Option<&PublicKey> {
        self.usable
            .iter()
            .find(|(k, _)| k == kid)
            .map(|(_, key)| key)
    }

    /// Whether the set lists this `kid` but its key is not usable.
    #[must_use]
    pub fn refuses(&self, kid: &str) -> bool {
        self.refused.iter().any(|k| k == kid)
    }

    /// How many usable keys.
    #[must_use]
    pub fn usable_len(&self) -> usize {
        self.usable.len()
    }
}

/// Prints counts only.
impl fmt::Debug for KeySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "KeySet({} usable, {} refused)",
            self.usable.len(),
            self.refused.len()
        )
    }
}

/// Strips leading zero bytes.
fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    &bytes[start..]
}

/// The usable key of one key-set entry (3.5 item 3), or `None`.
fn usable_key(entry: &Value) -> Option<PublicKey> {
    let text = |name: &str| entry.get(name).and_then(Value::as_str);
    if text("kty") != Some("RSA") {
        return None;
    }
    if entry.get("alg").is_some() && text("alg") != Some("RS256") {
        return None;
    }
    if entry.get("use").is_some() && text("use") != Some("sig") {
        return None;
    }
    let n = URL_SAFE_NO_PAD.decode(text("n")?).ok()?;
    let e = URL_SAFE_NO_PAD.decode(text("e")?).ok()?;
    if trim(&e) != [1, 0, 1] {
        return None;
    }
    let n = trim(&n);
    let first = *n.first()?;
    let bits = (n.len() - 1) * 8 + (8 - first.leading_zeros() as usize);
    let (low, high) = MODULUS_BITS;
    if !(low..=high).contains(&bits) {
        return None;
    }
    PublicKey::new_with_size_limits(
        BoxedUint::from_be_slice_vartime(n),
        BoxedUint::from_be_slice_vartime(&[1, 0, 1]),
        low..=high,
    )
    .ok()
}

/// Reads a key set (`{"keys": [...]}`). `None` when it is not of that shape. A `kid` that
/// appears twice is refused for both entries.
#[must_use]
pub fn parse_key_set(value: &Value) -> Option<KeySet> {
    let entries = value.get("keys")?.as_array()?;
    let mut set = KeySet::default();
    for entry in entries {
        let Some(kid) = entry.get("kid").and_then(Value::as_str) else {
            continue;
        };
        let kid = kid.to_owned();
        if set.refuses(&kid) {
            continue;
        }
        if set.find(&kid).is_some() {
            set.usable.retain(|(k, _)| *k != kid);
            set.refused.push(kid);
            continue;
        }
        match usable_key(entry) {
            Some(key) => set.usable.push((kid, key)),
            None => set.refused.push(kid),
        }
    }
    Some(set)
}

/// Fetches the key set once (3.5 item 3). Gives the set and how long to keep it.
pub fn fetch_key_set(transport: &Transport, deadline_ms: u64) -> Option<(KeySet, u64)> {
    let request = Request::new(Method::Get, Target::Keys, BodyBound::Medium);
    let Exchange::Answer(answer) = transport.send(request, deadline_ms) else {
        return None;
    };
    if !answer.is_success() {
        return None;
    }
    let value = answer.json().ok()?;
    let set = parse_key_set(&value)?;
    Some((set, cache_lifetime_ms(answer.max_age_seconds())))
}

/// Google's key set in memory (3.5 item 3). One per [`super::Flow`], shared by its sign-ins.
#[derive(Default)]
pub struct KeyCache {
    set: KeySet,
    /// Until when the set is kept; 0 when there is none.
    valid_until_ms: u64,
    /// When the last fetch made for an unknown `kid` happened.
    last_unknown_refetch_ms: Option<u64>,
    /// How many fetches were made.
    fetches: u64,
}

impl KeyCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> KeyCache {
        KeyCache::default()
    }

    /// How many times the key set was fetched.
    #[must_use]
    pub fn fetches(&self) -> u64 {
        self.fetches
    }

    fn refetch(
        &mut self,
        now_ms: u64,
        fetch: &mut dyn FnMut() -> Option<(KeySet, u64)>,
    ) -> Result<(), CheckFailure> {
        self.fetches += 1;
        let (set, lifetime_ms) = fetch().ok_or(CheckFailure::KeysUnavailable)?;
        self.set = set;
        self.valid_until_ms = now_ms.saturating_add(lifetime_ms);
        Ok(())
    }

    /// The key with this `kid`, fetching the set when it is not kept, and once more when the
    /// `kid` is unknown and no such refetch happened in the last minute (3.5 item 3). A set
    /// fetched in this call because the kept one had expired is not fetched again.
    pub fn key(
        &mut self,
        kid: &str,
        now_ms: u64,
        fetch: &mut dyn FnMut() -> Option<(KeySet, u64)>,
    ) -> Result<&PublicKey, CheckFailure> {
        // A set fetched in this call because the kept one had expired is as fresh as a
        // refetch would be: an unknown `kid` is not fetched for a second time (3.5 item 3).
        let fetched_now = now_ms >= self.valid_until_ms;
        if fetched_now {
            self.refetch(now_ms, fetch)?;
        }
        if !fetched_now && self.set.find(kid).is_none() && !self.set.refuses(kid) {
            let allowed = self
                .last_unknown_refetch_ms
                .is_none_or(|last| now_ms.saturating_sub(last) >= UNKNOWN_KID_REFETCH_MS);
            if allowed {
                self.last_unknown_refetch_ms = Some(now_ms);
                self.refetch(now_ms, fetch)?;
            }
        }
        if let Some(key) = self.set.find(kid) {
            return Ok(key);
        }
        Err(CheckFailure::Identity(if self.set.refuses(kid) {
            IdentityProblem::KeyNotUsable
        } else {
            IdentityProblem::UnknownKey
        }))
    }
}

/// Prints counts and times only.
impl fmt::Debug for KeyCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyCache")
            .field("set", &self.set)
            .field("valid_until_ms", &self.valid_until_ms)
            .field("fetches", &self.fetches)
            .finish()
    }
}

/// All ten checks of 3.5, in order. `fetch` fetches the key set when the cache needs it.
pub fn check(
    token: &Secret,
    expected: &Expected<'_>,
    keys: &mut KeyCache,
    fetch: &mut dyn FnMut() -> Option<(KeySet, u64)>,
) -> Result<Identity, CheckFailure> {
    let decoded = decode(token.expose()).map_err(CheckFailure::Identity)?;
    let kid = check_header(decoded.header.value()).map_err(CheckFailure::Identity)?;
    let key = keys.key(&kid, expected.now_ms, fetch)?;
    verify_signature(key, decoded.signing_input.as_bytes(), &decoded.signature)
        .map_err(CheckFailure::Identity)?;
    check_claims(decoded.payload.value(), expected).map_err(CheckFailure::Identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_google_fake::keys::{KeyId, sign_rs256};
    use serde_json::json;

    const CLIENT: &str = "client-1.apps.googleusercontent.com";
    const NOW_MS: u64 = 1_794_906_000_000;
    const NOW_S: i64 = 1_794_906_000;

    fn b64(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn claims() -> Value {
        json!({
            "iss": "https://accounts.google.com", "aud": CLIENT, "azp": CLIENT,
            "sub": "1234567890", "email": "Ann@Example.COM", "email_verified": true,
            "nonce": "nonce-1", "iat": NOW_S, "exp": NOW_S + 3600, "at_hash": "x"
        })
    }

    fn token(header: &str, payload: &str, key: KeyId) -> Secret {
        let input = format!("{}.{}", b64(header.as_bytes()), b64(payload.as_bytes()));
        let signature = sign_rs256(key, input.as_bytes());
        Secret::new(format!("{input}.{}", b64(&signature)))
    }

    fn header(kid: &str) -> String {
        format!("{{\"alg\":\"RS256\",\"kid\":\"{kid}\",\"typ\":\"JWT\"}}")
    }

    fn key_set(keys: &[KeyId]) -> KeySet {
        let value = json!({"keys": keys.iter().map(|k| k.jwk()).collect::<Vec<_>>()});
        parse_key_set(&value).expect("a key set")
    }

    fn expected(nonce: &Secret) -> Expected<'_> {
        Expected {
            client_id: CLIENT,
            nonce,
            now_ms: NOW_MS,
        }
    }

    fn run(token: &Secret, cache: &mut KeyCache) -> Result<Identity, CheckFailure> {
        let nonce = Secret::from("nonce-1");
        check(token, &expected(&nonce), cache, &mut || {
            Some((key_set(&[KeyId::A, KeyId::B]), 300_000))
        })
    }

    #[test]
    fn a_good_token_passes_every_check_and_lowers_the_domain() {
        let t = token(&header(KeyId::A.kid()), &claims().to_string(), KeyId::A);
        let identity = run(&t, &mut KeyCache::new()).expect("valid");
        assert_eq!(identity.subject, "1234567890");
        assert_eq!(identity.email.as_str(), "Ann@example.com");
        assert_eq!(format!("{identity:?}"), "Identity([redacted])");
    }

    #[test]
    fn the_shape_checks_of_item_1() {
        let fail = |t: &str| decode(t).err();
        assert_eq!(fail("a.b"), Some(IdentityProblem::NotThreeParts));
        assert_eq!(fail("a.b.c.d"), Some(IdentityProblem::NotThreeParts));
        assert_eq!(fail("..x"), Some(IdentityProblem::NotThreeParts));
        assert_eq!(fail("e30=.e30.AA"), Some(IdentityProblem::NotThreeParts));
        assert_eq!(fail("e30.e30.A+A"), Some(IdentityProblem::NotThreeParts));
        assert_eq!(
            fail(&"a".repeat(MAX_TOKEN_LEN + 1)),
            Some(IdentityProblem::TooLong)
        );
        let doubled = format!("{}.e30.AA", b64(br#"{"alg":"HS256","alg":"RS256"}"#));
        assert_eq!(fail(&doubled), Some(IdentityProblem::Header));
        let array = format!("{}.e30.AA", b64(b"[]"));
        assert_eq!(fail(&array), Some(IdentityProblem::Header));
        let doubled_sub = format!("e30.{}.AA", b64(br#"{"sub":"a","sub":"b"}"#));
        assert_eq!(fail(&doubled_sub), Some(IdentityProblem::Payload));
        let float = format!("e30.{}.AA", b64(br#"{"exp":1.5}"#));
        assert_eq!(fail(&float), Some(IdentityProblem::Payload));
        assert!(decode("e30.e30.AA").is_ok());
    }

    #[test]
    fn the_header_checks_of_items_2_and_3() {
        let check = |h: Value| check_header(&h);
        assert_eq!(
            check(json!({"alg": "RS256", "kid": "k"})),
            Ok("k".to_owned())
        );
        for alg in [
            json!("none"),
            json!("HS256"),
            json!("rs256"),
            json!("RS384"),
            json!(null),
        ] {
            assert_eq!(
                check(json!({"alg": alg, "kid": "k"})),
                Err(IdentityProblem::Algorithm)
            );
        }
        assert_eq!(check(json!({"kid": "k"})), Err(IdentityProblem::Algorithm));
        assert_eq!(
            check(json!({"alg": "RS256", "kid": "k", "crit": ["x"]})),
            Err(IdentityProblem::Critical)
        );
        assert_eq!(
            check(json!({"alg": "RS256"})),
            Err(IdentityProblem::NoKeyId)
        );
        assert_eq!(
            check(json!({"alg": "RS256", "kid": ""})),
            Err(IdentityProblem::NoKeyId)
        );
        assert_eq!(
            check(json!({"alg": "RS256", "kid": 3})),
            Err(IdentityProblem::NoKeyId)
        );
        // A jwk header is never read: the key still comes from the key set.
        assert_eq!(
            check(json!({"alg": "RS256", "kid": "k", "jwk": {"kty": "RSA"}})),
            Ok("k".to_owned())
        );
    }

    #[test]
    fn keys_that_item_3_refuses() {
        let set = key_set(&KeyId::ALL);
        assert!(set.find(KeyId::A.kid()).is_some());
        assert!(set.find(KeyId::B.kid()).is_some());
        assert!(set.find(KeyId::Attacker.kid()).is_some());
        assert!(set.refuses(KeyId::WeakExponent.kid()));
        assert!(set.refuses(KeyId::ShortModulus.kid()));
        let mut jwk = KeyId::A.jwk();
        let with = |name: &str, value: Value| {
            let mut entry = KeyId::A.jwk();
            entry[name] = value;
            parse_key_set(&json!({"keys": [entry]})).expect("a set")
        };
        for (name, value) in [
            ("kty", json!("EC")),
            ("alg", json!("RS512")),
            ("alg", json!(null)),
            ("use", json!("enc")),
            ("e", json!("Aw")),
            ("n", json!("not base64!")),
        ] {
            assert!(
                with(name, value.clone()).refuses(KeyId::A.kid()),
                "{name} {value}"
            );
        }
        jwk.as_object_mut().expect("object").remove("alg");
        jwk.as_object_mut().expect("object").remove("use");
        let bare = parse_key_set(&json!({"keys": [jwk]})).expect("a set");
        assert!(bare.find(KeyId::A.kid()).is_some());
        let twice = parse_key_set(&json!({"keys": [KeyId::A.jwk(), KeyId::A.jwk()]})).expect("set");
        assert!(twice.refuses(KeyId::A.kid()) && twice.usable_len() == 0);
        assert!(parse_key_set(&json!({"nokeys": []})).is_none());
    }

    #[test]
    fn the_signature_must_verify_with_the_named_key() {
        let payload = claims().to_string();
        let by_b = token(&header(KeyId::A.kid()), &payload, KeyId::B);
        assert_eq!(
            run(&by_b, &mut KeyCache::new()).err(),
            Some(CheckFailure::Identity(IdentityProblem::Signature))
        );
        let good = token(&header(KeyId::A.kid()), &payload, KeyId::A);
        let mut parts: Vec<String> = good.expose().split('.').map(str::to_owned).collect();
        parts[1] = b64(claims()
            .to_string()
            .replace("1234567890", "1234567891")
            .as_bytes());
        let swapped = Secret::new(parts.join("."));
        assert_eq!(
            run(&swapped, &mut KeyCache::new()).err(),
            Some(CheckFailure::Identity(IdentityProblem::Signature))
        );
    }

    #[test]
    fn the_claim_checks_of_items_5_to_10() {
        let nonce = Secret::from("nonce-1");
        let fails = |change: &dyn Fn(&mut Value), problem: IdentityProblem| {
            let mut c = claims();
            change(&mut c);
            assert_eq!(
                check_claims(&c, &expected(&nonce)).err(),
                Some(problem),
                "{c}"
            );
        };
        assert!(check_claims(&claims(), &expected(&nonce)).is_ok());
        let mut short_iss = claims();
        short_iss["iss"] = json!("accounts.google.com");
        assert!(check_claims(&short_iss, &expected(&nonce)).is_ok());
        let mut array_aud = claims();
        array_aud["aud"] = json!([CLIENT]);
        array_aud.as_object_mut().expect("o").remove("azp");
        assert!(check_claims(&array_aud, &expected(&nonce)).is_ok());

        fails(
            &|c| c["iss"] = json!("https://evil.example"),
            IdentityProblem::Issuer,
        );
        fails(
            &|c| {
                c.as_object_mut().expect("o").remove("iss");
            },
            IdentityProblem::Issuer,
        );
        fails(&|c| c["aud"] = json!("other"), IdentityProblem::Audience);
        fails(
            &|c| c["aud"] = json!([CLIENT, "other"]),
            IdentityProblem::Audience,
        );
        fails(&|c| c["aud"] = json!([]), IdentityProblem::Audience);
        fails(
            &|c| c["azp"] = json!("other"),
            IdentityProblem::AuthorizedParty,
        );
        fails(&|c| c["azp"] = json!(1), IdentityProblem::AuthorizedParty);
        fails(
            &|c| {
                c["iat"] = json!(NOW_S - 300);
                c["exp"] = json!(NOW_S - 61);
            },
            IdentityProblem::Times,
        );
        fails(&|c| c["exp"] = json!(NOW_S), IdentityProblem::Times);
        fails(&|c| c["iat"] = json!(NOW_S + 61), IdentityProblem::Times);
        fails(&|c| c["iat"] = json!(NOW_S - 601), IdentityProblem::Times);
        fails(
            &|c| c["exp"] = json!(c["iat"].clone()),
            IdentityProblem::Times,
        );
        fails(&|c| c["exp"] = json!("later"), IdentityProblem::Times);
        fails(&|c| c["nonce"] = json!("nonce-2"), IdentityProblem::Nonce);
        fails(
            &|c| {
                c.as_object_mut().expect("o").remove("nonce");
            },
            IdentityProblem::Nonce,
        );
        fails(&|c| c["sub"] = json!(""), IdentityProblem::Subject);
        fails(&|c| c["sub"] = json!("a b"), IdentityProblem::Subject);
        fails(
            &|c| c["sub"] = json!("x".repeat(256)),
            IdentityProblem::Subject,
        );
        fails(&|c| c["sub"] = json!(12), IdentityProblem::Subject);
        fails(
            &|c| c["email_verified"] = json!(false),
            IdentityProblem::EmailNotVerified,
        );
        fails(
            &|c| c["email_verified"] = json!("true"),
            IdentityProblem::EmailNotVerified,
        );
        fails(
            &|c| c["email"] = json!("no-at-sign"),
            IdentityProblem::EmailNotUsable,
        );
        fails(
            &|c| c["email"] = json!("\u{e9}@example.com"),
            IdentityProblem::EmailNotUsable,
        );
        fails(
            &|c| c["email"] = json!("a@localhost"),
            IdentityProblem::EmailNotUsable,
        );
        fails(
            &|c| {
                c.as_object_mut().expect("o").remove("email");
            },
            IdentityProblem::EmailNotUsable,
        );
        // Edges that pass.
        let mut edge = claims();
        edge["iat"] = json!(NOW_S + 60);
        edge["exp"] = json!(NOW_S + 61);
        assert!(check_claims(&edge, &expected(&nonce)).is_ok());
        edge["iat"] = json!(NOW_S - 600);
        edge["exp"] = json!(NOW_S - 59);
        assert!(check_claims(&edge, &expected(&nonce)).is_ok());
    }

    #[test]
    fn the_cache_lifetime_is_held_to_5_minutes_and_24_hours() {
        assert_eq!(cache_lifetime_ms(None), 300_000);
        assert_eq!(cache_lifetime_ms(Some(0)), 300_000);
        assert_eq!(cache_lifetime_ms(Some(10)), 300_000);
        assert_eq!(cache_lifetime_ms(Some(21_600)), 21_600_000);
        assert_eq!(cache_lifetime_ms(Some(86_400)), 86_400_000);
        assert_eq!(cache_lifetime_ms(Some(u64::MAX)), 86_400_000);
    }

    #[test]
    fn the_cache_refetches_on_expiry_and_once_a_minute_for_an_unknown_kid() {
        let mut cache = KeyCache::new();
        let mut fetches = 0u32;
        let mut fetch = || {
            fetches += 1;
            Some((key_set(&[KeyId::A]), 300_000))
        };
        assert!(cache.key(KeyId::A.kid(), 0, &mut fetch).is_ok());
        assert!(cache.key(KeyId::A.kid(), 299_999, &mut fetch).is_ok());
        assert_eq!(cache.fetches(), 1);
        // An unknown kid: one refetch, then none for a minute.
        assert_eq!(
            cache.key("unknown", 1_000, &mut fetch).err(),
            Some(CheckFailure::Identity(IdentityProblem::UnknownKey))
        );
        assert_eq!(cache.fetches(), 2);
        assert!(cache.key("unknown", 60_999, &mut fetch).is_err());
        assert_eq!(cache.fetches(), 2);
        assert!(cache.key("unknown", 61_000, &mut fetch).is_err());
        assert_eq!(cache.fetches(), 3);
        // Expiry.
        assert!(cache.key(KeyId::A.kid(), 361_000, &mut fetch).is_ok());
        assert_eq!(cache.fetches(), 4);
        // A set fetched in this call because the kept one expired is not fetched again for an
        // unknown kid, even when no refetch for an unknown kid happened in the last minute.
        let mut fresh = KeyCache::new();
        let mut fresh_fetches = 0u32;
        let mut fresh_fetch = || {
            fresh_fetches += 1;
            Some((key_set(&[KeyId::A]), 300_000))
        };
        assert_eq!(
            fresh.key("unknown", 0, &mut fresh_fetch).err(),
            Some(CheckFailure::Identity(IdentityProblem::UnknownKey))
        );
        assert_eq!(fresh.fetches(), 1, "the first fetch only");
        assert_eq!(
            fresh.key("unknown", 400_000, &mut fresh_fetch).err(),
            Some(CheckFailure::Identity(IdentityProblem::UnknownKey))
        );
        assert_eq!(fresh.fetches(), 2, "the expiry fetch only");
        let _ = fresh_fetches;
        // A refused key is not refetched for.
        let mut weak = KeyCache::new();
        let mut weak_fetch = || Some((key_set(&[KeyId::WeakExponent]), 300_000));
        assert_eq!(
            weak.key(KeyId::WeakExponent.kid(), 0, &mut weak_fetch)
                .err(),
            Some(CheckFailure::Identity(IdentityProblem::KeyNotUsable))
        );
        assert_eq!(weak.fetches(), 1);
        // No key set at all.
        let mut none = KeyCache::new();
        assert_eq!(
            none.key("k", 0, &mut || None).err(),
            Some(CheckFailure::KeysUnavailable)
        );
        let _ = fetches;
    }
}
