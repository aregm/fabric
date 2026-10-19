use std::error::Error;
use std::fmt;
use std::io;

/// Failures of the policy gate. Any of them means "deny": the gate never permits an action
/// it could not check.
#[derive(Debug)]
pub enum PolicyError {
    /// A fact or action could not be rendered safely into HOL.
    Render(String),
    /// The Isabelle server spoke an unexpected protocol message.
    Protocol(String),
    /// Isabelle reported a failure, such as a session that would not start.
    Isabelle(String),
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(message) => write!(formatter, "cannot render for Isabelle: {message}"),
            Self::Protocol(message) => write!(formatter, "Isabelle server protocol: {message}"),
            Self::Isabelle(message) => write!(formatter, "Isabelle: {message}"),
            Self::Io(error) => write!(formatter, "I/O: {error}"),
            Self::Json(error) => write!(formatter, "JSON: {error}"),
        }
    }
}

impl Error for PolicyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PolicyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PolicyError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
