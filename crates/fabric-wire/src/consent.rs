//! Consent records (section 12, task T7.1). When an owner says yes or no to a mutual option,
//! their host's consent signer signs a record with the owner's identity key. The record travels
//! inside `decide`, so the other side's gate can check the owner's answer without trusting
//! either agent (D16). The signature is a COSE_Sign1 with the external data
//! `fabric-consent-v1`, so a consent record can never pass for a message or a token.

use fabric_policy::Consent;
use fabric_token::{SigningKey, VerifyingKey, cbor, cose};
use serde::{Deserialize, Serialize};

use crate::error::WireError;
use crate::message::SignedRecord;

/// The external data of consent signatures.
pub const CONSENT_AAD: &[u8] = b"fabric-consent-v1";
const MAX_RECORD_BYTES: usize = 4_096;

/// One owner's yes or no to one meeting (section 6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentRecord {
    pub signer: String,
    pub negotiation: String,
    pub start_ms: i64,
    pub length_ms: i64,
    /// Both people, in alphabetical order.
    pub participants: Vec<String>,
    pub yes: bool,
    pub issued_at_ms: i64,
    pub expires_ms: i64,
}

impl ConsentRecord {
    pub fn sign(&self, identity: &SigningKey) -> Result<SignedRecord, WireError> {
        Ok(SignedRecord(cose::sign(
            identity,
            &cbor::to_vec(self)?,
            CONSENT_AAD,
        )?))
    }

    /// Checks that `signer` signed `record`, and returns it.
    pub fn verify(record: &SignedRecord, signer: &VerifyingKey) -> Result<Self, WireError> {
        let payload = cose::verify(&record.0, signer, CONSENT_AAD, MAX_RECORD_BYTES)
            .map_err(WireError::Signature)?;
        cbor::decode(&payload)?
            .deserialized()
            .map_err(|error| WireError::Format(error.to_string()))
    }

    /// The fact the policy gate reasons about, once the signature has been checked.
    #[must_use]
    pub fn fact(&self) -> Consent {
        Consent {
            signer: self.signer.clone(),
            negotiation: self.negotiation.clone(),
            start_ms: self.start_ms,
            length_ms: self.length_ms,
            yes: self.yes,
            expires_ms: self.expires_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use fabric_token::SigningKey;

    use super::ConsentRecord;
    use crate::message::Message;

    fn record() -> ConsentRecord {
        ConsentRecord {
            signer: "bob".to_owned(),
            negotiation: "n-1".to_owned(),
            start_ms: 1_799_937_000_000,
            length_ms: 1_800_000,
            participants: vec!["alice".to_owned(), "bob".to_owned()],
            yes: true,
            issued_at_ms: 1_799_656_000_000,
            expires_ms: 1_799_740_800_000,
        }
    }

    #[test]
    fn records_verify_only_with_the_owners_key_and_never_as_messages() {
        let bob = SigningKey::from_bytes(&[2; 32]);
        let signed = record().sign(&bob).unwrap();
        assert_eq!(
            ConsentRecord::verify(&signed, &bob.verifying_key()).unwrap(),
            record()
        );
        let mallory = SigningKey::from_bytes(&[3; 32]);
        assert!(ConsentRecord::verify(&signed, &mallory.verifying_key()).is_err());
        assert!(Message::verify(&signed.0, &bob.verifying_key()).is_err());
        assert!(record().fact().yes);
    }
}
