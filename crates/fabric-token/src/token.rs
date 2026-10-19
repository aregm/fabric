//! CBOR Web Tokens (RFC 8392) in COSE_Sign1 envelopes (RFC 9052), signed with Ed25519.
//!
//! The policy gate mints one token per permitted action with [`Minter::mint`]; the calendar
//! service or the messaging gateway checks it with [`Verifier`](crate::Verifier). Section 9 of
//! docs/isabelle-calendar-demo.md describes the claims and the checks.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use fabric_policy::{Action, CheckKind, CheckReport};
use serde::{Deserialize, Serialize};

use crate::cbor::{self, CborError, Value, int};
use crate::digest::action_digest;
use crate::error::TokenError;
use crate::hex;
use crate::keys::random;

/// Seconds from a token's issue to its expiry (section 8.3).
pub const LIFETIME_SECONDS: i64 = 60;
/// Longer tokens are refused before they are parsed.
pub const MAX_TOKEN_BYTES: usize = 4_096;

/// The CBOR tag of a COSE_Sign1 message.
const COSE_SIGN1_TAG: u64 = 18;
/// The protected header `{1: -19}`: algorithm Ed25519 (RFC 9864). The unprotected header is
/// always empty, and the signature covers no external data.
const PROTECTED: [u8; 3] = [0xa1, 0x01, 0x32];

// Registered claim keys (RFC 8392 section 4; RFC 8747 section 3.1).
const ISS: i64 = 1;
const AUD: i64 = 3;
const EXP: i64 = 4;
const IAT: i64 = 6;
const CTI: i64 = 7;
const CNF: i64 = 8;
// Private-use claim keys, which are the integers below -65536 (RFC 8392 section 9.1).
const ACTION: i64 = -65_537;
const ARGS: i64 = -65_538;
const NEGOTIATION: i64 = -65_539;
const PROOF: i64 = -65_540;
// `cnf` holds a COSE_Key (RFC 8747 section 3.2) for an Ed25519 public key (RFC 9053 section 7.2).
const CNF_COSE_KEY: i64 = 1;
const KTY: i64 = 1;
const KTY_OKP: i64 = 1;
const CRV: i64 = -1;
const CRV_ED25519: i64 = 6;
const X: i64 = -2;

/// What a token says. Byte fields appear as lowercase hex in JSON.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    /// `iss`: the minting gate, `gate:<person>`.
    pub issuer: String,
    /// `aud`: the one service that may accept the token.
    pub audience: String,
    /// `cnf`: the agent's token-binding public key.
    #[serde(with = "hex::array")]
    pub holder: [u8; 32],
    /// `iat`, in seconds since the Unix epoch.
    pub issued_at: i64,
    /// `exp`, in seconds since the Unix epoch.
    pub expires_at: i64,
    /// `cti`: the single-use nonce.
    #[serde(with = "hex::array")]
    pub nonce: [u8; 16],
    /// The action kind, as [`Action::kind`] names it.
    pub action: String,
    /// SHA-256 of the action's deterministic CBOR ([`action_digest`]).
    #[serde(with = "hex::array")]
    pub args: [u8; 32],
    /// The negotiation ID, for actions that belong to one.
    pub negotiation: Option<String>,
    /// The Isabelle proof ID of the check that permitted the action.
    #[serde(with = "hex::array")]
    pub proof: [u8; 32],
}

impl Claims {
    fn to_value(&self) -> Value {
        let holder = Value::Map(vec![
            (int(KTY), int(KTY_OKP)),
            (int(CRV), int(CRV_ED25519)),
            (int(X), Value::Bytes(self.holder.to_vec())),
        ]);
        let mut claims = vec![
            (int(ISS), Value::Text(self.issuer.clone())),
            (int(AUD), Value::Text(self.audience.clone())),
            (int(EXP), int(self.expires_at)),
            (int(IAT), int(self.issued_at)),
            (int(CTI), Value::Bytes(self.nonce.to_vec())),
            (int(CNF), Value::Map(vec![(int(CNF_COSE_KEY), holder)])),
            (int(ACTION), Value::Text(self.action.clone())),
            (int(ARGS), Value::Bytes(self.args.to_vec())),
            (int(PROOF), Value::Bytes(self.proof.to_vec())),
        ];
        if let Some(negotiation) = &self.negotiation {
            claims.push((int(NEGOTIATION), Value::Text(negotiation.clone())));
        }
        Value::Map(claims)
    }

    fn from_value(value: Value) -> Result<Self, String> {
        let Value::Map(entries) = value else {
            return Err("the claims are not a map".to_owned());
        };
        let mut claims = BTreeMap::new();
        for (key, value) in entries {
            let key = integer(key, "a claim key")?;
            if ![
                ISS,
                AUD,
                EXP,
                IAT,
                CTI,
                CNF,
                ACTION,
                ARGS,
                NEGOTIATION,
                PROOF,
            ]
            .contains(&key)
            {
                return Err(format!("unexpected claim {key}"));
            }
            claims.insert(key, value);
        }
        let mut take = |key: i64, name: &str| {
            claims
                .remove(&key)
                .ok_or_else(|| format!("the {name} claim is missing"))
        };
        Ok(Self {
            issuer: text(take(ISS, "iss")?, "iss")?,
            audience: text(take(AUD, "aud")?, "aud")?,
            holder: holder_key(take(CNF, "cnf")?)?,
            issued_at: integer(take(IAT, "iat")?, "iat")?,
            expires_at: integer(take(EXP, "exp")?, "exp")?,
            nonce: bytes(take(CTI, "cti")?, "cti")?,
            action: text(take(ACTION, "action")?, "action")?,
            args: bytes(take(ARGS, "args")?, "args")?,
            negotiation: take(NEGOTIATION, "negotiation")
                .ok()
                .map(|value| text(value, "negotiation"))
                .transpose()?,
            proof: bytes(take(PROOF, "proof")?, "proof")?,
        })
    }
}

fn integer(value: Value, name: &str) -> Result<i64, String> {
    match value {
        Value::Integer(value) => {
            i64::try_from(value).map_err(|_| format!("{name} is out of range"))
        }
        _ => Err(format!("{name} is not an integer")),
    }
}

fn text(value: Value, name: &str) -> Result<String, String> {
    match value {
        Value::Text(text) => Ok(text),
        _ => Err(format!("{name} is not a text string")),
    }
}

fn bytes<const N: usize>(value: Value, name: &str) -> Result<[u8; N], String> {
    match value {
        Value::Bytes(bytes) => <[u8; N]>::try_from(bytes.as_slice())
            .map_err(|_| format!("{name} has {} bytes; expected {N}", bytes.len())),
        _ => Err(format!("{name} is not a byte string")),
    }
}

/// Reads `{1: {1: 1, -1: 6, -2: x}}`. Decoding was deterministic, so the parameters arrive in
/// key order.
fn holder_key(value: Value) -> Result<[u8; 32], String> {
    let invalid = || "cnf must be an Ed25519 COSE_Key".to_owned();
    let Value::Map(cnf) = value else {
        return Err(invalid());
    };
    let [(member, key)] = <[(Value, Value); 1]>::try_from(cnf).map_err(|_| invalid())?;
    if member != int(CNF_COSE_KEY) {
        return Err(invalid());
    }
    let Value::Map(parameters) = key else {
        return Err(invalid());
    };
    let [(kty, okp), (crv, ed25519), (x, public)] =
        <[(Value, Value); 3]>::try_from(parameters).map_err(|_| invalid())?;
    if kty != int(KTY)
        || okp != int(KTY_OKP)
        || crv != int(CRV)
        || ed25519 != int(CRV_ED25519)
        || x != int(X)
    {
        return Err(invalid());
    }
    bytes(public, "the cnf key")
}

/// The services that execute actions, and so accept tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    /// The person's calendar service: `ReadFreeBusy` and `AddMeeting`.
    Calendar,
    /// The person's messaging gateway: `Handshake` and the `Send…` actions.
    Gateway,
}

impl Service {
    /// The service that executes `action`. Nothing executes `ReadEventDetails`.
    #[must_use]
    pub const fn for_action(action: &Action) -> Option<Self> {
        match action {
            Action::ReadFreeBusy { .. } | Action::AddMeeting { .. } => Some(Self::Calendar),
            Action::Handshake { .. }
            | Action::SendPropose { .. }
            | Action::SendRespond { .. }
            | Action::SendDecide { .. }
            | Action::SendClose { .. } => Some(Self::Gateway),
            Action::ReadEventDetails { .. } => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::Gateway => "gateway",
        }
    }

    /// The `aud` claim for this service of `person`: `calendar:<person>` or `gateway:<person>`.
    #[must_use]
    pub fn audience(self, person: &str) -> String {
        format!("{}:{person}", self.name())
    }
}

/// The `iss` claim of `person`'s policy gate: `gate:<person>`.
#[must_use]
pub fn issuer(person: &str) -> String {
    format!("gate:{person}")
}

/// Checks a person's key ID with the rule for HOL identifiers, since the same ID appears in
/// check theories, and so audiences such as `calendar:<person>` cannot be ambiguous.
pub(crate) fn check_person(person: &str) -> Result<(), TokenError> {
    fabric_policy::hol::identifier(person)
        .map(|_| ())
        .map_err(|error| TokenError::Key(error.to_string()))
}

/// The policy gate's signing half: turns a permitting authorize check into a token.
pub struct Minter {
    person: String,
    key: SigningKey,
}

impl Minter {
    /// A minter for `person`'s gate.
    pub fn new(person: &str, key: SigningKey) -> Result<Self, TokenError> {
        check_person(person)?;
        Ok(Self::trusted(person, key))
    }

    /// For a person whose ID was already checked.
    pub(crate) fn trusted(person: &str, key: SigningKey) -> Self {
        Self {
            person: person.to_owned(),
            key,
        }
    }

    #[must_use]
    pub fn person(&self) -> &str {
        &self.person
    }

    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    /// Mints a token for the one action `report` permitted, bound to the agent key `holder`.
    /// Refuses unless `report` is an authorize check whose single action Isabelle permitted.
    pub fn mint(
        &self,
        report: &CheckReport,
        holder: &VerifyingKey,
        now_ms: i64,
    ) -> Result<Minted, TokenError> {
        self.mint_with_nonce(report, holder, now_ms, random()?)
    }

    fn mint_with_nonce(
        &self,
        report: &CheckReport,
        holder: &VerifyingKey,
        now_ms: i64,
        nonce: [u8; 16],
    ) -> Result<Minted, TokenError> {
        if report.kind != CheckKind::Authorize {
            return Err(TokenError::Mint(
                "tokens come only from an authorize check, not from screening".to_owned(),
            ));
        }
        let [checked] = report.verdicts.as_slice() else {
            return Err(TokenError::Mint(format!(
                "an authorize check covers one action, not {}",
                report.verdicts.len()
            )));
        };
        let action = &checked.action;
        if !checked.verdict.is_permit() {
            return Err(TokenError::Mint(format!(
                "{} was not permitted ({})",
                action.kind(),
                checked.verdict.summary()
            )));
        }
        let service = Service::for_action(action)
            .ok_or_else(|| TokenError::Mint(format!("no service executes {}", action.kind())))?;
        let proof = hex::decode_array(&report.proof_id)
            .map_err(|detail| TokenError::Mint(format!("proof ID: {detail}")))?;
        let issued_at = now_ms.div_euclid(1_000);
        let claims = Claims {
            issuer: issuer(&self.person),
            audience: service.audience(&self.person),
            holder: holder.to_bytes(),
            issued_at,
            expires_at: issued_at + LIFETIME_SECONDS,
            nonce,
            action: action.kind().to_owned(),
            args: action_digest(action)?,
            negotiation: action.negotiation().map(str::to_owned),
            proof,
        };
        let bytes = seal(&self.key, &claims)?;
        Ok(Minted { bytes, claims })
    }
}

impl fmt::Debug for Minter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Minter")
            .field("person", &self.person)
            .field("gate_key", &hex::encode(self.verifying_key().as_bytes()))
            .finish_non_exhaustive()
    }
}

/// A freshly minted token and what it says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Minted {
    pub bytes: Vec<u8>,
    pub claims: Claims,
}

/// Signs `claims` as a tagged COSE_Sign1.
fn seal(key: &SigningKey, claims: &Claims) -> Result<Vec<u8>, CborError> {
    let payload = cbor::encode(&claims.to_value())?;
    let signature = key.sign(&sig_structure(&payload)?);
    cbor::encode(&Value::Tag(
        COSE_SIGN1_TAG,
        Box::new(Value::Array(vec![
            Value::Bytes(PROTECTED.to_vec()),
            Value::Map(Vec::new()),
            Value::Bytes(payload),
            Value::Bytes(signature.to_bytes().to_vec()),
        ])),
    ))
}

/// The bytes a COSE_Sign1 signature covers (RFC 9052 section 4.4).
fn sig_structure(payload: &[u8]) -> Result<Vec<u8>, CborError> {
    cbor::encode(&Value::Array(vec![
        Value::Text("Signature1".to_owned()),
        Value::Bytes(PROTECTED.to_vec()),
        Value::Bytes(Vec::new()),
        Value::Bytes(payload.to_vec()),
    ]))
}

/// A token's payload and signature, after checking the envelope's exact shape.
fn envelope(token: &[u8]) -> Result<(Vec<u8>, Signature), String> {
    if token.len() > MAX_TOKEN_BYTES {
        return Err(format!(
            "the token has {} bytes; at most {MAX_TOKEN_BYTES}",
            token.len()
        ));
    }
    let value = cbor::decode(token).map_err(|error| format!("token: {error}"))?;
    let Value::Tag(COSE_SIGN1_TAG, inner) = value else {
        return Err("the token is not a tagged COSE_Sign1".to_owned());
    };
    let Value::Array(items) = *inner else {
        return Err("the COSE_Sign1 is not an array".to_owned());
    };
    let [protected, unprotected, payload, signature] = <[Value; 4]>::try_from(items)
        .map_err(|_| "the COSE_Sign1 does not have four items".to_owned())?;
    if protected != Value::Bytes(PROTECTED.to_vec()) {
        return Err("the protected header must be exactly {1: -19} (Ed25519)".to_owned());
    }
    if unprotected != Value::Map(Vec::new()) {
        return Err("the unprotected header must be empty".to_owned());
    }
    let Value::Bytes(payload) = payload else {
        return Err("the payload is not a byte string".to_owned());
    };
    let signature: [u8; 64] = bytes(signature, "the signature")?;
    Ok((payload, Signature::from_bytes(&signature)))
}

fn claims_of(payload: &[u8]) -> Result<Claims, String> {
    let value = cbor::decode(payload).map_err(|error| format!("claims: {error}"))?;
    Claims::from_value(value)
}

/// Checks the gate's signature and returns the claims.
pub(crate) fn open(token: &[u8], gate_key: &VerifyingKey) -> Result<Claims, String> {
    let (payload, signature) = envelope(token)?;
    let message = sig_structure(&payload).map_err(|error| error.to_string())?;
    gate_key
        .verify_strict(&message, &signature)
        .map_err(|_| "the signature does not verify with this gate's key".to_owned())?;
    claims_of(&payload)
}

/// Reads a token's claims without checking its signature: for the agent that holds it, and
/// for display.
pub fn read_unverified(token: &[u8]) -> Result<Claims, TokenError> {
    let (payload, _) = envelope(token).map_err(TokenError::Format)?;
    claims_of(&payload).map_err(TokenError::Format)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signature, SigningKey};

    use super::{Claims, open, seal};
    use crate::cbor;

    fn from_hex(text: &str) -> Vec<u8> {
        crate::hex::decode(text).unwrap()
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    fn sample() -> Claims {
        Claims {
            issuer: "gate:alice".to_owned(),
            audience: "calendar:alice".to_owned(),
            holder: [0x11; 32],
            issued_at: 1_799_654_400,
            expires_at: 1_799_654_460,
            nonce: [0x22; 16],
            action: "read_free_busy".to_owned(),
            args: [0x33; 32],
            negotiation: None,
            proof: [0x44; 32],
        }
    }

    /// The claims and envelope bytes, assembled by hand from RFC 8392, RFC 8747 and RFC 9052.
    #[test]
    fn tokens_have_the_specified_layout() {
        // Claim keys in deterministic order: 1 iss, 3 aud, 4 exp, 6 iat, 7 cti, 8 cnf, then the
        // private claims -65537 action, -65538 args and -65540 proof.
        let payload = cat(&[
            &from_hex("a9016a"),
            b"gate:alice",
            &from_hex("036e"),
            b"calendar:alice",
            &from_hex("041a6b448c3c"), // exp 1799654460
            &from_hex("061a6b448c00"), // iat 1799654400
            &from_hex("0750"),
            &[0x22; 16],
            &from_hex("08a101a301012006215820"), // {1: {1: 1, -1: 6, -2: h'11..'}}
            &[0x11; 32],
            &from_hex("3a000100006e"),
            b"read_free_busy",
            &from_hex("3a000100015820"),
            &[0x33; 32],
            &from_hex("3a000100035820"),
            &[0x44; 32],
        ]);
        assert_eq!(cbor::encode(&sample().to_value()).unwrap(), payload);

        // ["Signature1", h'a10132', h'', payload]
        let length = u8::try_from(payload.len()).unwrap();
        let signed = cat(&[
            &from_hex("846a"),
            b"Signature1",
            &from_hex("43a1013240"),
            &[0x58, length],
            &payload,
        ]);

        // RFC 8032 section 7.1, test 1.
        let key = SigningKey::from_bytes(
            &crate::hex::decode_array(
                "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            )
            .unwrap(),
        );
        let token = seal(&key, &sample()).unwrap();
        // 18([h'a10132', {}, payload, signature])
        let expected = cat(&[
            &from_hex("d28443a10132a0"),
            &[0x58, length],
            &payload,
            &from_hex("5840"),
        ]);
        assert_eq!(token[..expected.len()], expected);
        assert_eq!(token.len(), expected.len() + 64);

        let signature = Signature::from_slice(&token[expected.len()..]).unwrap();
        key.verifying_key()
            .verify_strict(&signed, &signature)
            .expect("the signature covers the Sig_structure");
        assert_eq!(open(&token, &key.verifying_key()).unwrap(), sample());
    }
}
