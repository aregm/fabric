//! The agent channel's handshake (README section 8): the two methods that come before any
//! tool frame on a connection, the text an agent signs, and the pairing code the owner reads.
//!
//! This crate only builds the text and the code. Signing and verifying with Ed25519 belong to
//! the agent adapter and the broker.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::common::PROTOCOL;
use crate::digest::sha256_hex;
use crate::json::canonical;
use crate::scalar::{Count, Id, Nonce, PairingCode, PositiveCount, PublicKey, Signature};

/// The methods of the agent channel that are not tools, in the order of the schema.
pub const HANDSHAKE_METHODS: [&str; 2] = ["agent_hello", "agent_prove"];

/// `agent#/$defs/params_agent_hello`: the first frame of a connection, with the agent's id,
/// its public key and the adapter's schema version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentHelloParams {
    pub schema_version: Count,
    pub enrollment_id: Id,
    pub public_key: PublicKey,
}

/// `agent#/$defs/result_agent_hello`: the broker's schema version and the nonce it made for
/// this connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentHelloResult {
    pub schema_version: PositiveCount,
    pub broker_nonce: Nonce,
}

/// `agent#/$defs/params_agent_prove`: the Ed25519 signature over the handshake text, made
/// with the key of the hello.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProveParams {
    pub signature: Signature,
}

/// What a proof found: the session has started (`Approved`), or the proved key is now the
/// waiting agent's key and the owner has not approved yet (`Pending`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvedEnrollment {
    Approved,
    Pending,
}

/// `agent#/$defs/result_agent_prove`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProveResult {
    pub enrollment: ProvedEnrollment,
}

/// The text an agent signs: `fabric-desktop/1`, `agent_handshake`, each ended by a line feed,
/// and the canonical form of the four handshake values (README sections 3 and 8). The
/// signature is Ed25519 (RFC 8032) over its UTF-8 bytes.
#[must_use]
pub fn handshake_text(
    schema_version: u64,
    enrollment_id: &Id,
    public_key: &PublicKey,
    broker_nonce: &Nonce,
) -> String {
    let subject = json!({
        "broker_nonce": broker_nonce.as_str(),
        "enrollment_id": enrollment_id.as_str(),
        "public_key": public_key.as_str(),
        "schema_version": schema_version,
    });
    format!("{PROTOCOL}\nagent_handshake\n{}", canonical(&subject))
}

/// The code of a public key: the first 12 hexadecimal digits, in capitals, of
/// SHA-256(`fabric-desktop/1\nagent_key\n` + the key as written), as `XXXX-XXXX-XXXX`.
#[must_use]
pub fn pairing_code(public_key: &PublicKey) -> PairingCode {
    let hex = sha256_hex(&format!("{PROTOCOL}\nagent_key\n{}", public_key.as_str()));
    let digits = hex[..12].to_ascii_uppercase();
    PairingCode::new(format!(
        "{}-{}-{}",
        &digits[0..4],
        &digits[4..8],
        &digits[8..12]
    ))
    .expect("twelve hexadecimal digits in capitals make a pairing code")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pairing_code_of_the_rfc_8032_test_key() {
        let key = PublicKey::new(
            "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        )
        .unwrap();
        assert_eq!(pairing_code(&key).as_str(), "100D-25BE-C78D");
    }

    #[test]
    fn the_handshake_text_is_two_lines_and_a_canonical_object() {
        let text = handshake_text(
            3,
            &Id::new("agent-1").unwrap(),
            &PublicKey::new(format!("ed25519:{}", "0".repeat(64))).unwrap(),
            &Nonce::new("a".repeat(64)).unwrap(),
        );
        assert_eq!(
            text,
            format!(
                "fabric-desktop/1\nagent_handshake\n{{\"broker_nonce\":\"{}\",\"enrollment_id\":\"agent-1\",\"public_key\":\"ed25519:{}\",\"schema_version\":3}}",
                "a".repeat(64),
                "0".repeat(64)
            )
        );
    }
}
