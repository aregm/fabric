//! Protocol messages (section 6). Each is typed, signed with the sender's Ed25519 identity key
//! (D16), and travels inside the Noise session. The signature is a COSE_Sign1 over the
//! message's deterministic CBOR, with the external data `fabric-message-v1`, so a signed
//! message can never pass for a consent record or a token.

use std::fmt;

use fabric_policy::{Action, Answer, Candidate, CloseReason, Decision};
use fabric_token::{SigningKey, VerifyingKey, cbor, cose, hex};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::WireError;

/// The external data of message signatures.
pub const MESSAGE_AAD: &[u8] = b"fabric-message-v1";
/// Longer signed messages are refused before they are parsed.
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;

/// A message from one gateway to the other.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub negotiation: String,
    pub sent_at_ms: i64,
    pub body: Body,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Body {
    /// The coordinator's candidates for one round.
    Propose {
        round: u32,
        candidates: Vec<Candidate>,
        note: String,
    },
    /// The invitee's availability for each received candidate, in order.
    Respond {
        round: u32,
        answers: Vec<Answer>,
        note: String,
    },
    /// The sender's yes or no to each mutual option, with its owner's signed consent records.
    Decide {
        round: u32,
        decisions: Vec<Decision>,
        consents: Vec<SignedRecord>,
        note: String,
    },
    Close {
        reason: CloseReason,
        note: String,
    },
}

/// A signed record carried inside a message, such as a consent record. It travels as a CBOR
/// byte string, and appears as lowercase hex in JSON.
#[derive(Clone, Eq, PartialEq)]
pub struct SignedRecord(pub Vec<u8>);

impl fmt::Debug for SignedRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SignedRecord({} bytes)", self.0.len())
    }
}

impl Serialize for SignedRecord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(&self.0))
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for SignedRecord {
    /// Accepts either form, whatever the deserializer reports: inside an internally tagged
    /// enum, serde's buffering deserializer calls itself human-readable even for CBOR.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecordVisitor;

        impl Visitor<'_> for RecordVisitor {
            type Value = SignedRecord;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a byte string, or lowercase hex")
            }

            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<SignedRecord, E> {
                Ok(SignedRecord(bytes.to_vec()))
            }

            fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<SignedRecord, E> {
                Ok(SignedRecord(bytes))
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<SignedRecord, E> {
                hex::decode(text).map(SignedRecord).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(RecordVisitor)
    }
}

impl Body {
    /// The message type, as in `{"type": "propose", ...}`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Propose { .. } => "propose",
            Self::Respond { .. } => "respond",
            Self::Decide { .. } => "decide",
            Self::Close { .. } => "close",
        }
    }

    #[must_use]
    pub const fn round(&self) -> Option<u32> {
        match self {
            Self::Propose { round, .. }
            | Self::Respond { round, .. }
            | Self::Decide { round, .. } => Some(*round),
            Self::Close { .. } => None,
        }
    }

    #[must_use]
    pub fn note(&self) -> &str {
        match self {
            Self::Propose { note, .. }
            | Self::Respond { note, .. }
            | Self::Decide { note, .. }
            | Self::Close { note, .. } => note,
        }
    }
}

impl Message {
    /// The message that a send action asks for, or `None` for an action that sends nothing.
    /// `consents` go into a `Decide`; they are ignored otherwise.
    #[must_use]
    pub fn for_action(
        action: &Action,
        from: &str,
        to: &str,
        sent_at_ms: i64,
        consents: Vec<SignedRecord>,
    ) -> Option<Self> {
        let (negotiation, body) = match action.clone() {
            Action::SendPropose {
                negotiation,
                round,
                candidates,
                note,
            } => (
                negotiation,
                Body::Propose {
                    round,
                    candidates,
                    note,
                },
            ),
            Action::SendRespond {
                negotiation,
                round,
                answers,
                note,
            } => (
                negotiation,
                Body::Respond {
                    round,
                    answers,
                    note,
                },
            ),
            Action::SendDecide {
                negotiation,
                round,
                decisions,
                note,
            } => (
                negotiation,
                Body::Decide {
                    round,
                    decisions,
                    consents,
                    note,
                },
            ),
            Action::SendClose {
                negotiation,
                reason,
                note,
            } => (negotiation, Body::Close { reason, note }),
            _ => return None,
        };
        Some(Self {
            from: from.to_owned(),
            to: to.to_owned(),
            negotiation,
            sent_at_ms,
            body,
        })
    }

    /// Signs the message with the sender's identity key.
    pub fn sign(&self, identity: &SigningKey) -> Result<Vec<u8>, WireError> {
        Ok(cose::sign(identity, &cbor::to_vec(self)?, MESSAGE_AAD)?)
    }

    /// Checks that `sender` signed `signed`, and returns the message.
    pub fn verify(signed: &[u8], sender: &VerifyingKey) -> Result<Self, WireError> {
        let payload = cose::verify(signed, sender, MESSAGE_AAD, MAX_MESSAGE_BYTES)
            .map_err(WireError::Signature)?;
        cbor::decode(&payload)?
            .deserialized()
            .map_err(|error| WireError::Format(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use fabric_policy::{Action, Candidate};
    use fabric_token::SigningKey;

    use super::{Body, Message, SignedRecord};

    #[test]
    fn messages_round_trip_signed_and_refuse_other_keys() {
        let action = Action::SendPropose {
            negotiation: "n1".to_owned(),
            round: 1,
            candidates: vec![Candidate {
                id: "c1".to_owned(),
                start_ms: 1_799_762_400_000,
                length_ms: 1_800_000,
            }],
            note: "afternoons suit me".to_owned(),
        };
        let message = Message::for_action(&action, "alice", "bob", 5, Vec::new()).unwrap();
        assert_eq!(message.body.kind(), "propose");
        let alice = SigningKey::from_bytes(&[1; 32]);
        let signed = message.sign(&alice).unwrap();
        assert_eq!(
            Message::verify(&signed, &alice.verifying_key()).unwrap(),
            message
        );
        let bob = SigningKey::from_bytes(&[2; 32]);
        assert!(Message::verify(&signed, &bob.verifying_key()).is_err());
        assert!(
            Message::for_action(
                &Action::Handshake {
                    peer: "bob".to_owned()
                },
                "alice",
                "bob",
                5,
                Vec::new()
            )
            .is_none()
        );

        let decide = Message {
            body: Body::Decide {
                round: 1,
                decisions: Vec::new(),
                consents: vec![SignedRecord(vec![1, 2, 3])],
                note: String::new(),
            },
            ..message
        };
        let signed = decide.sign(&alice).unwrap();
        assert_eq!(
            Message::verify(&signed, &alice.verifying_key()).unwrap(),
            decide
        );

        // In JSON, as the agent's deliveries carry it, a signed record is hex.
        let json = serde_json::to_string(&decide).unwrap();
        assert!(json.contains("\"consents\":[\"010203\"]"), "{json}");
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), decide);
    }
}
