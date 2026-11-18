//! The provisional handshake of W06 4.4, until the contract crate has its types (V3-1).
//!
//! ```text
//! bridge -> broker   {"protocol", "id": "hello-<32 hex>", "method": "agent_hello",
//!                     "params": {"schema_version", "enrollment_id", "public_key"}}
//! broker -> bridge   result {"schema_version", "broker_nonce"}
//!                    or error unsupported_schema_version | internal
//! bridge -> broker   {"protocol", "id": "prove-<32 hex>", "method": "agent_prove",
//!                     "params": {"signature": "<128 hex>"}}
//! broker -> bridge   result {"enrollment": "approved" | "pending"}
//!                    or error not_permitted | internal
//! ```
//!
//! These frames are not contract frames: `decode` refuses their methods. They are built with
//! `serde_json` and the contract's canonical form, and the answers are read with the contract's
//! strict JSON reader and, for an error, the contract's agent error type. The answers are read
//! strictly; there is no negotiation.

use fabric_desktop_contracts::error::{AgentError, ErrorCode};
use fabric_desktop_contracts::scalar::Id;
use fabric_desktop_contracts::typed::from_value;
use fabric_desktop_contracts::{PROTOCOL, SCHEMA_VERSION, canonical, parse_strict};
use serde_json::{Map, Value, json};

use super::key::AgentKey;

/// The method of the first step.
pub const HELLO: &str = "agent_hello";
/// The method of the second step.
pub const PROVE: &str = "agent_prove";
/// The two leading lines of the signed text: the contract's digest prefix with the domain
/// `agent_handshake`.
pub const SIGNED_PREFIX: &str = "fabric-desktop/1\nagent_handshake\n";

/// The bytes the bridge signs: [`SIGNED_PREFIX`] and the canonical form of the four members.
#[must_use]
pub fn signed_text(
    broker_nonce: &str,
    enrollment_id: &Id,
    public_key: &str,
    schema_version: u64,
) -> String {
    let object = json!({
        "broker_nonce": broker_nonce,
        "enrollment_id": enrollment_id.as_str(),
        "public_key": public_key,
        "schema_version": schema_version,
    });
    format!("{SIGNED_PREFIX}{}", canonical(&object))
}

/// The signature of the handshake, 128 lowercase hexadecimal digits.
#[must_use]
pub fn signature(key: &AgentKey, broker_nonce: &str, enrollment_id: &Id) -> String {
    let text = signed_text(
        broker_nonce,
        enrollment_id,
        &key.public_key_text(),
        SCHEMA_VERSION,
    );
    key.sign_hex(text.as_bytes())
}

/// The `agent_hello` line, with its line feed.
#[must_use]
pub fn hello_line(frame_id: &str, enrollment_id: &Id, public_key: &str) -> String {
    let frame = json!({
        "protocol": PROTOCOL,
        "id": frame_id,
        "method": HELLO,
        "params": {
            "schema_version": SCHEMA_VERSION,
            "enrollment_id": enrollment_id.as_str(),
            "public_key": public_key,
        },
    });
    format!("{}\n", canonical(&frame))
}

/// The `agent_prove` line, with its line feed.
#[must_use]
pub fn prove_line(frame_id: &str, signature: &str) -> String {
    let frame = json!({
        "protocol": PROTOCOL,
        "id": frame_id,
        "method": PROVE,
        "params": {"signature": signature},
    });
    format!("{}\n", canonical(&frame))
}

/// The broker's answer to `agent_hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelloAnswer {
    /// The broker's nonce for this connection.
    Nonce(String),
    /// A well-formed result whose schema version is not the bridge's own; it carries the
    /// version received (W06 4.4: another one is `unsupported_schema_version`).
    SchemaVersion(u64),
    /// `unsupported_schema_version` or `internal`.
    Error(AgentError),
}

/// The broker's answer to `agent_prove`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProveAnswer {
    /// The session has started.
    Approved,
    /// The proven key is now the pending agent's key; the owner has not approved yet.
    Pending,
    /// `not_permitted` or `internal`.
    Error(AgentError),
}

/// An answer that is not one: not strict JSON, not an object, a wrong protocol, id or method,
/// an unknown member, a malformed result, or an error code this step does not answer with.
/// Nothing of the answer is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

impl Malformed {
    /// The fixed words for stderr.
    pub const REASON: &'static str = "a handshake answer was malformed";
}

/// The members of an answer, a response frame as the contract writes them (W06 4.4, CR 1):
/// `protocol`, the request's `id` and `method`, and exactly one of `result` and `error`. A
/// result carries the request's method; `method: null` is allowed only with an error.
enum Body {
    Result(Map<String, Value>),
    Error(AgentError),
}

fn read_body(bytes: &[u8], frame_id: &str, method: &str) -> Result<Body, Malformed> {
    let Ok(Value::Object(mut members)) = parse_strict(bytes) else {
        return Err(Malformed);
    };
    if members.remove("protocol") != Some(json!(PROTOCOL)) {
        return Err(Malformed);
    }
    if members.remove("id") != Some(json!(frame_id)) {
        return Err(Malformed);
    }
    let named = match members.remove("method") {
        Some(Value::String(m)) if m == method => true,
        Some(Value::Null) => false,
        _ => return Err(Malformed),
    };
    let body = match (members.remove("result"), members.remove("error")) {
        (Some(Value::Object(result)), None) if named => Body::Result(result),
        (None, Some(error)) => Body::Error(from_value(&error).map_err(|_| Malformed)?),
        _ => return Err(Malformed),
    };
    if members.is_empty() {
        Ok(body)
    } else {
        Err(Malformed)
    }
}

/// Whether `text` is exactly 64 lowercase hexadecimal digits.
#[must_use]
pub fn is_nonce(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Reads the answer to the `agent_hello` with id `frame_id`.
pub fn read_hello_answer(bytes: &[u8], frame_id: &str) -> Result<HelloAnswer, Malformed> {
    match read_body(bytes, frame_id, HELLO)? {
        Body::Error(error) => match error.code() {
            ErrorCode::UnsupportedSchemaVersion | ErrorCode::Internal => {
                Ok(HelloAnswer::Error(error))
            }
            _ => Err(Malformed),
        },
        Body::Result(mut result) => {
            let version = result.remove("schema_version");
            let nonce = result.remove("broker_nonce");
            if !result.is_empty() {
                return Err(Malformed);
            }
            let Some(version) = version.as_ref().and_then(Value::as_u64) else {
                return Err(Malformed);
            };
            if version != SCHEMA_VERSION {
                return Ok(HelloAnswer::SchemaVersion(version));
            }
            match nonce {
                Some(Value::String(nonce)) if is_nonce(&nonce) => Ok(HelloAnswer::Nonce(nonce)),
                _ => Err(Malformed),
            }
        }
    }
}

/// Reads the answer to the `agent_prove` with id `frame_id`.
pub fn read_prove_answer(bytes: &[u8], frame_id: &str) -> Result<ProveAnswer, Malformed> {
    match read_body(bytes, frame_id, PROVE)? {
        Body::Error(error) => match error.code() {
            ErrorCode::NotPermitted | ErrorCode::Internal => Ok(ProveAnswer::Error(error)),
            _ => Err(Malformed),
        },
        Body::Result(mut result) => {
            let enrollment = result.remove("enrollment");
            if !result.is_empty() {
                return Err(Malformed);
            }
            match enrollment.as_ref().and_then(Value::as_str) {
                Some("approved") => Ok(ProveAnswer::Approved),
                Some("pending") => Ok(ProveAnswer::Pending),
                _ => Err(Malformed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, VerifyingKey};

    const NONCE: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    // k01: the signed text and the signature for a fixed seed and nonce; a changed nonce
    // changes it; an answer with a wrong nonce form, another schema version or another id is
    // refused.
    #[test]
    fn k01_handshake() {
        let key = AgentKey::from_seed(&[1u8; 32]);
        let id = Id::new("agent-test").unwrap();
        let public = key.public_key_text();
        let text = signed_text(NONCE, &id, &public, 2);
        assert_eq!(
            text,
            format!(
                "fabric-desktop/1\nagent_handshake\n{{\"broker_nonce\":\"{NONCE}\",\"enrollment_id\":\"agent-test\",\"public_key\":\"{public}\",\"schema_version\":2}}"
            )
        );
        let sig = signature(&key, NONCE, &id);
        assert_eq!(sig.len(), 128);
        assert!(
            sig.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
        // Ed25519 is deterministic: the same seed and nonce give the same signature.
        assert_eq!(sig, signature(&AgentKey::from_seed(&[1u8; 32]), NONCE, &id));
        // It verifies against the public key over exactly the signed text.
        let verifying = VerifyingKey::from_bytes(&key.public_key_bytes()).unwrap();
        let parsed = Signature::from_bytes(&unhex::<64>(&sig));
        assert!(
            verifying
                .verify_strict(
                    signed_text(NONCE, &id, &public, SCHEMA_VERSION).as_bytes(),
                    &parsed
                )
                .is_ok()
        );
        let other_nonce = NONCE.replace("00", "01");
        assert_ne!(sig, signature(&key, &other_nonce, &id));

        // The frames.
        let hello: Value =
            parse_strict(hello_line("hello-1", &id, &public).trim_end().as_bytes()).unwrap();
        assert_eq!(hello["method"], json!("agent_hello"));
        assert_eq!(hello["params"]["schema_version"], json!(SCHEMA_VERSION));
        let prove: Value = parse_strict(prove_line("prove-1", &sig).trim_end().as_bytes()).unwrap();
        assert_eq!(prove["params"]["signature"], json!(sig));

        // The answers.
        let ok = format!(
            r#"{{"protocol":"fabric-desktop/1","id":"hello-1","method":"agent_hello","result":{{"schema_version":{SCHEMA_VERSION},"broker_nonce":"{NONCE}"}}}}"#
        );
        assert_eq!(
            read_hello_answer(ok.as_bytes(), "hello-1"),
            Ok(HelloAnswer::Nonce(NONCE.to_owned()))
        );
        assert_eq!(read_hello_answer(ok.as_bytes(), "hello-2"), Err(Malformed));
        for bad_nonce in [&NONCE[..63], &NONCE.to_uppercase(), &format!("{NONCE}0")] {
            let a = ok.replace(NONCE, bad_nonce);
            assert_eq!(read_hello_answer(a.as_bytes(), "hello-1"), Err(Malformed));
        }
        let other_version = ok.replace(
            &format!("\"schema_version\":{SCHEMA_VERSION}"),
            "\"schema_version\":9",
        );
        assert_eq!(
            read_hello_answer(other_version.as_bytes(), "hello-1"),
            Ok(HelloAnswer::SchemaVersion(9))
        );
        let extra = ok.replace("\"result\"", "\"extra\":1,\"result\"");
        assert_eq!(
            read_hello_answer(extra.as_bytes(), "hello-1"),
            Err(Malformed)
        );
        // A result must name its method: a missing or null method, or another, is refused.
        for method in ["", r#""method":null,"#, r#""method":"agent_prove","#] {
            let a = ok.replace(r#""method":"agent_hello","#, method);
            assert_eq!(
                read_hello_answer(a.as_bytes(), "hello-1"),
                Err(Malformed),
                "{method}"
            );
        }
        let error = r#"{"protocol":"fabric-desktop/1","id":"prove-1","method":"agent_prove","error":{"code":"not_permitted","message":"no","retryable":false,"recovery":"none","correlation_id":null,"detail":null}}"#;
        assert!(matches!(
            read_prove_answer(error.as_bytes(), "prove-1"),
            Ok(ProveAnswer::Error(_))
        ));
        // An error may carry a null method, but not a missing one.
        let null_method = error.replace(r#""method":"agent_prove""#, r#""method":null"#);
        assert!(matches!(
            read_prove_answer(null_method.as_bytes(), "prove-1"),
            Ok(ProveAnswer::Error(_))
        ));
        let no_method = error.replace(r#""method":"agent_prove","#, "");
        assert_eq!(
            read_prove_answer(no_method.as_bytes(), "prove-1"),
            Err(Malformed)
        );
        // not_permitted is not an answer to agent_hello.
        let hello_error = error
            .replace("prove-1", "hello-1")
            .replace("agent_prove", "agent_hello");
        assert_eq!(
            read_hello_answer(hello_error.as_bytes(), "hello-1"),
            Err(Malformed)
        );
        for (answer, expected) in [
            ("approved", Ok(ProveAnswer::Approved)),
            ("pending", Ok(ProveAnswer::Pending)),
            ("removed", Err(Malformed)),
        ] {
            let a = format!(
                r#"{{"protocol":"fabric-desktop/1","id":"prove-1","method":"agent_prove","result":{{"enrollment":"{answer}"}}}}"#
            );
            assert_eq!(read_prove_answer(a.as_bytes(), "prove-1"), expected);
        }
        let wrong_method = r#"{"protocol":"fabric-desktop/1","id":"prove-1","method":"agent_hello","result":{"enrollment":"approved"}}"#;
        assert_eq!(
            read_prove_answer(wrong_method.as_bytes(), "prove-1"),
            Err(Malformed)
        );
    }

    fn unhex<const N: usize>(text: &str) -> [u8; N] {
        assert_eq!(text.len(), 2 * N);
        let mut out = [0u8; N];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    // k04: a cross-check against the contract session's independent fixture, schema version 3
    // draft, pull request 36, `contracts/desktop/v1/fixtures/handshake.json`: for each vector,
    // the public key text from the seed, the pairing code, the signed text byte for byte, and
    // the signature of that text.
    #[test]
    fn k04_contract_fixture() {
        let vectors = [
            (
                "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
                3,
                "agent-claude-desktop",
                "aa0cd311e9453e1fb9b9e14fda1ab4aa2d5858c2a9488a95be640274f3f8afa2",
                "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
                "100D-25BE-C78D",
                "c383528ffd1bc133d8d9501443bebdf77662283fde39b099e5888a99b7ccf268909b38c22e252c2006894be09c8dd1c25f8711268ba44104e8777c0a5c32ad05",
            ),
            (
                "ba4c2cafa66cd16ed5b68653cad1ba510f4cfd7a04ab89f653486aa3691c3992",
                3,
                "agent-0123456789abcdef0123456789abcdef",
                "b79397ae9847146259d1600abb8bdaa0075e9bec5fd00fd2792b0f24f7484543",
                "ed25519:095acc33714f1c29e75bc9445d03e517ee55a0ebeb01b7a420e96e505abf5429",
                "4E7A-9852-EC06",
                "d5ba4c39e5a5c2ca8df6b302cc6515cd4b0213691a9146e59922cf042791c2967625fa44fe6d402e34419b967b3e74595b98101ba6466040344b62360896f207",
            ),
        ];
        for (seed, version, enrollment, nonce, public, code, sig) in vectors {
            let key = AgentKey::from_seed(&unhex::<32>(seed));
            assert_eq!(key.public_key_text(), public, "public key of {enrollment}");
            assert_eq!(key.pairing_code(), code, "pairing code of {enrollment}");
            let text = format!(
                "fabric-desktop/1\nagent_handshake\n{{\"broker_nonce\":\"{nonce}\",\"enrollment_id\":\"{enrollment}\",\"public_key\":\"{public}\",\"schema_version\":{version}}}"
            );
            let id = Id::new(enrollment).unwrap();
            assert_eq!(
                signed_text(nonce, &id, public, version),
                text,
                "text of {enrollment}"
            );
            assert_eq!(
                key.sign_hex(text.as_bytes()),
                sig,
                "signature of {enrollment}"
            );
        }
    }
}
