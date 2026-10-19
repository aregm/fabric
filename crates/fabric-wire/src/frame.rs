//! Relay frames: what the relay carries between two gateways. A frame names its Noise session by
//! a random ID and otherwise carries only Noise output, so the relay learns sizes and timing and
//! nothing else (section 5.4).
//!
//! Encoding, as deterministic CBOR: `[1, session, noise]` for the caller's handshake message,
//! `[2, session, noise]` for the callee's answer, and `[3, session, nonce, noise]` for a
//! transport message.

use fabric_token::cbor::{self, Value};

use crate::error::WireError;

/// Noise messages are at most 65535 bytes; framing adds a few more.
pub const MAX_FRAME_BYTES: usize = 65_535 + 64;

pub type SessionId = [u8; 8];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Frame {
    /// The first IK handshake message, from the caller.
    Hello { session: SessionId, noise: Vec<u8> },
    /// The second IK handshake message, from the callee.
    Welcome { session: SessionId, noise: Vec<u8> },
    /// A transport message. The explicit nonce lets the receiver refuse replays and survive a
    /// frame that never arrived.
    Message {
        session: SessionId,
        nonce: u64,
        noise: Vec<u8>,
    },
}

impl Frame {
    #[must_use]
    pub const fn session(&self) -> SessionId {
        match self {
            Self::Hello { session, .. }
            | Self::Welcome { session, .. }
            | Self::Message { session, .. } => *session,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let items = match self {
            Self::Hello { session, noise } => vec![
                cbor::int(1),
                Value::Bytes(session.to_vec()),
                Value::Bytes(noise.clone()),
            ],
            Self::Welcome { session, noise } => vec![
                cbor::int(2),
                Value::Bytes(session.to_vec()),
                Value::Bytes(noise.clone()),
            ],
            Self::Message {
                session,
                nonce,
                noise,
            } => vec![
                cbor::int(3),
                Value::Bytes(session.to_vec()),
                Value::Integer((*nonce).into()),
                Value::Bytes(noise.clone()),
            ],
        };
        Ok(cbor::encode(&Value::Array(items))?)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let invalid = |detail: &str| WireError::Format(format!("frame: {detail}"));
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(invalid("too long"));
        }
        let Value::Array(items) = cbor::decode(bytes)? else {
            return Err(invalid("not an array"));
        };
        let mut items = items.into_iter();
        let kind = match items.next() {
            Some(Value::Integer(kind)) => u8::try_from(kind).map_err(|_| invalid("bad type"))?,
            _ => return Err(invalid("no type")),
        };
        let session: SessionId = match items.next() {
            Some(Value::Bytes(session)) => {
                <SessionId>::try_from(session.as_slice()).map_err(|_| invalid("bad session"))?
            }
            _ => return Err(invalid("no session")),
        };
        let rest: Vec<Value> = items.collect();
        match (kind, rest.as_slice()) {
            (1, [Value::Bytes(noise)]) => Ok(Self::Hello {
                session,
                noise: noise.clone(),
            }),
            (2, [Value::Bytes(noise)]) => Ok(Self::Welcome {
                session,
                noise: noise.clone(),
            }),
            (3, [Value::Integer(nonce), Value::Bytes(noise)]) => Ok(Self::Message {
                session,
                nonce: u64::try_from(*nonce).map_err(|_| invalid("bad nonce"))?,
                noise: noise.clone(),
            }),
            _ => Err(invalid("unknown layout")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Frame;

    #[test]
    fn frames_round_trip() {
        for frame in [
            Frame::Hello {
                session: [1; 8],
                noise: vec![9; 96],
            },
            Frame::Welcome {
                session: [2; 8],
                noise: vec![8; 48],
            },
            Frame::Message {
                session: [3; 8],
                nonce: 7,
                noise: vec![7; 20],
            },
        ] {
            let bytes = frame.encode().unwrap();
            assert_eq!(Frame::decode(&bytes).unwrap(), frame);
        }
        assert!(Frame::decode(&[0x80]).is_err());
        assert!(Frame::decode(b"plain text").is_err());
    }
}
