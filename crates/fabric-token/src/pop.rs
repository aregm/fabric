//! Proof of possession: the agent signs every use of a token with the key named in its `cnf`
//! claim (RFC 8747), over the token's nonce, the SHA-256 of the request body, and a timestamp.

use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::cbor::{self, CborError, Value, int};
use crate::digest::sha256;
use crate::error::TokenError;
use crate::hex;
use crate::keys::random;
use crate::token::{Claims, MAX_TOKEN_BYTES, read_unverified};

/// How old a proof of possession may be when the service checks it.
pub const POP_MAX_AGE_MS: i64 = 10_000;
/// How far ahead of the service's clock a proof of possession may be dated.
pub const POP_MAX_LEAD_MS: i64 = 2_000;

/// Separates these signatures from any other use of the agent's key.
const CONTEXT: &str = "fabric-pop-v1";

/// An agent's token-binding key. The agent makes a new one each time it starts (section 5.3).
pub struct BindingKey(SigningKey);

impl BindingKey {
    pub fn generate() -> Result<Self, TokenError> {
        Ok(Self(SigningKey::from_bytes(&random()?)))
    }

    #[must_use]
    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    /// The public half, which the launcher registers with the trusted host.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.0.verifying_key()
    }

    /// Presents `token` with a request whose body is `body`, signed at `now_ms`.
    pub fn present(
        &self,
        token: &[u8],
        body: &[u8],
        now_ms: i64,
    ) -> Result<Presentation, TokenError> {
        let claims = read_unverified(token)?;
        let signature = self.0.sign(&message(&claims.nonce, body, now_ms)?);
        Ok(Presentation {
            token: token.to_vec(),
            timestamp_ms: now_ms,
            signature: signature.to_bytes(),
        })
    }
}

impl fmt::Debug for BindingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BindingKey")
            .field(&hex::encode(self.verifying_key().as_bytes()))
            .finish()
    }
}

/// A token and its proof of possession, as an agent sends them with one request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Presentation {
    pub token: Vec<u8>,
    pub timestamp_ms: i64,
    pub signature: [u8; 64],
}

impl Presentation {
    /// Deterministic CBOR: `[token, timestamp_ms, signature]`.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TokenError> {
        Ok(cbor::encode(&Value::Array(vec![
            Value::Bytes(self.token.clone()),
            int(self.timestamp_ms),
            Value::Bytes(self.signature.to_vec()),
        ]))?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TokenError> {
        let invalid = || TokenError::Format("not a token presentation".to_owned());
        if bytes.len() > MAX_TOKEN_BYTES + 128 {
            return Err(TokenError::Format(format!(
                "the presentation has {} bytes; at most {}",
                bytes.len(),
                MAX_TOKEN_BYTES + 128
            )));
        }
        let Value::Array(items) = cbor::decode(bytes)? else {
            return Err(invalid());
        };
        let [token, timestamp, signature] = <[Value; 3]>::try_from(items).map_err(|_| invalid())?;
        let (Value::Bytes(token), Value::Integer(timestamp), Value::Bytes(signature)) =
            (token, timestamp, signature)
        else {
            return Err(invalid());
        };
        Ok(Self {
            token,
            timestamp_ms: i64::try_from(timestamp).map_err(|_| invalid())?,
            signature: <[u8; 64]>::try_from(signature.as_slice()).map_err(|_| invalid())?,
        })
    }
}

/// What the agent signs: `["fabric-pop-v1", nonce, SHA-256(body), timestamp_ms]`.
fn message(nonce: &[u8; 16], body: &[u8], timestamp_ms: i64) -> Result<Vec<u8>, CborError> {
    cbor::encode(&Value::Array(vec![
        Value::Text(CONTEXT.to_owned()),
        Value::Bytes(nonce.to_vec()),
        Value::Bytes(sha256(body).to_vec()),
        int(timestamp_ms),
    ]))
}

/// Checks the proof of possession against the token's `cnf` key.
pub(crate) fn verify(
    claims: &Claims,
    presentation: &Presentation,
    body: &[u8],
    now_ms: i64,
) -> Result<(), String> {
    let age = now_ms.saturating_sub(presentation.timestamp_ms);
    if age > POP_MAX_AGE_MS {
        return Err(format!(
            "signed {age} ms before the check; at most {POP_MAX_AGE_MS} ms"
        ));
    }
    if age < -POP_MAX_LEAD_MS {
        return Err(format!(
            "dated {} ms ahead of this service's clock; at most {POP_MAX_LEAD_MS} ms",
            age.saturating_neg()
        ));
    }
    let holder = VerifyingKey::from_bytes(&claims.holder)
        .map_err(|_| "the token's cnf key is not an Ed25519 public key".to_owned())?;
    let message = message(&claims.nonce, body, presentation.timestamp_ms)
        .map_err(|error| error.to_string())?;
    holder
        .verify_strict(&message, &Signature::from_bytes(&presentation.signature))
        .map_err(|_| {
            "not signed with the token's cnf key over this request body and time".to_owned()
        })
}
