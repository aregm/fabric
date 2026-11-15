use std::fmt;

use fabric_token::CborError;

#[derive(Debug)]
pub enum WireError {
    Cbor(CborError),
    /// Bytes that are not a frame, message or record of this protocol.
    Format(String),
    /// A signature that does not verify with the expected key.
    Signature(String),
    /// A Noise handshake or transport failure, including a replayed or altered frame.
    Noise(String),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => error.fmt(f),
            Self::Format(detail) => write!(f, "malformed: {detail}"),
            Self::Signature(detail) => write!(f, "bad signature: {detail}"),
            Self::Noise(detail) => write!(f, "noise: {detail}"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<CborError> for WireError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<snow::Error> for WireError {
    fn from(error: snow::Error) -> Self {
        Self::Noise(error.to_string())
    }
}
