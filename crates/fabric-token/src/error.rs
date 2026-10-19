use std::fmt;

use crate::cbor::CborError;

/// Errors from minting, encoding, and key handling. A token a service refuses is a
/// [`Rejection`](crate::Rejection) instead.
#[derive(Debug)]
pub enum TokenError {
    Cbor(CborError),
    /// The gate refused to mint: the check was not an authorize permit for one action.
    Mint(String),
    /// A token or presentation could not be read.
    Format(String),
    /// A key is missing or malformed.
    Key(String),
    /// The secret store failed.
    Store(String),
    /// The operating system's random number generator failed.
    Random(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => error.fmt(f),
            Self::Mint(detail) => write!(f, "no token: {detail}"),
            Self::Format(detail) => write!(f, "unreadable token: {detail}"),
            Self::Key(detail) => write!(f, "key: {detail}"),
            Self::Store(detail) => write!(f, "secret store: {detail}"),
            Self::Random(detail) => write!(f, "random number generator: {detail}"),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TokenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cbor(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CborError> for TokenError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<std::io::Error> for TokenError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for TokenError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
