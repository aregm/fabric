//! COSE_Sign1 (RFC 9052) in one fixed profile, shared by tokens, protocol messages and consent
//! records. The protected header is exactly `{1: -19}` (Ed25519, RFC 9864), and the unprotected
//! header is empty. The external data, which the signature covers but the message does not
//! carry, separates the uses of one key. Messages are deterministic CBOR, and any other
//! encoding is refused.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::cbor::{self, CborError, Value};

/// The CBOR tag of a COSE_Sign1 message.
const COSE_SIGN1_TAG: u64 = 18;
/// The protected header `{1: -19}`.
const PROTECTED: [u8; 3] = [0xa1, 0x01, 0x32];

/// Signs `payload` as a tagged COSE_Sign1 whose signature also covers `external_aad`.
pub fn sign(key: &SigningKey, payload: &[u8], external_aad: &[u8]) -> Result<Vec<u8>, CborError> {
    let signature = key.sign(&sig_structure(payload, external_aad)?);
    cbor::encode(&Value::Tag(
        COSE_SIGN1_TAG,
        Box::new(Value::Array(vec![
            Value::Bytes(PROTECTED.to_vec()),
            Value::Map(Vec::new()),
            Value::Bytes(payload.to_vec()),
            Value::Bytes(signature.to_bytes().to_vec()),
        ])),
    ))
}

/// Checks that `key` signed `message` with `external_aad`, and returns the payload.
pub fn verify(
    message: &[u8],
    key: &VerifyingKey,
    external_aad: &[u8],
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let (payload, signature) = envelope(message, max_bytes)?;
    let signed = sig_structure(&payload, external_aad).map_err(|error| error.to_string())?;
    key.verify_strict(&signed, &signature)
        .map_err(|_| "the signature does not verify with the expected key".to_owned())?;
    Ok(payload)
}

/// The payload, after checking the message's shape but not its signature.
pub fn read_payload(message: &[u8], max_bytes: usize) -> Result<Vec<u8>, String> {
    envelope(message, max_bytes).map(|(payload, _)| payload)
}

/// The bytes a COSE_Sign1 signature covers (RFC 9052 section 4.4).
fn sig_structure(payload: &[u8], external_aad: &[u8]) -> Result<Vec<u8>, CborError> {
    cbor::encode(&Value::Array(vec![
        Value::Text("Signature1".to_owned()),
        Value::Bytes(PROTECTED.to_vec()),
        Value::Bytes(external_aad.to_vec()),
        Value::Bytes(payload.to_vec()),
    ]))
}

/// The payload and signature, after checking the message's exact shape.
fn envelope(message: &[u8], max_bytes: usize) -> Result<(Vec<u8>, Signature), String> {
    if message.len() > max_bytes {
        return Err(format!(
            "the message has {} bytes; at most {max_bytes}",
            message.len()
        ));
    }
    let value = cbor::decode(message).map_err(|error| format!("COSE_Sign1: {error}"))?;
    let Value::Tag(COSE_SIGN1_TAG, inner) = value else {
        return Err("the message is not a tagged COSE_Sign1".to_owned());
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
    let (Value::Bytes(payload), Value::Bytes(signature)) = (payload, signature) else {
        return Err("the payload and signature must be byte strings".to_owned());
    };
    let signature = <[u8; 64]>::try_from(signature.as_slice())
        .map_err(|_| format!("the signature has {} bytes; expected 64", signature.len()))?;
    Ok((payload, Signature::from_bytes(&signature)))
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::{read_payload, sign, verify};

    #[test]
    fn external_data_separates_uses_of_one_key() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let message = sign(&key, b"payload", b"fabric-message-v1").unwrap();
        let public = key.verifying_key();
        assert_eq!(
            verify(&message, &public, b"fabric-message-v1", 1_024).unwrap(),
            b"payload"
        );
        assert!(verify(&message, &public, b"fabric-consent-v1", 1_024).is_err());
        assert!(verify(&message, &public, b"fabric-message-v1", 10).is_err());
        let other = SigningKey::from_bytes(&[4; 32]).verifying_key();
        assert!(verify(&message, &other, b"fabric-message-v1", 1_024).is_err());
        assert_eq!(read_payload(&message, 1_024).unwrap(), b"payload");
    }
}
