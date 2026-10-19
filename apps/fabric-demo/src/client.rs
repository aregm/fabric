//! The client side of the demo's HTTP conventions (see [`crate::api`]), shared by the clients
//! of the calendar service and the trusted host.

use std::fmt;
use std::time::Duration;

use fabric_policy::Action;
use fabric_token::{BindingKey, TokenError};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::api::authorization;

pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum CallError {
    /// The service refused the request. `error` is a token reason from section 9, such as
    /// `token_replayed`, or another error code of the service.
    Refused {
        status: u16,
        error: String,
        detail: String,
    },
    Http(reqwest::Error),
    Token(TokenError),
    /// The service answered with something this client cannot read.
    Response(String),
}

impl CallError {
    /// The service's error code, for a refusal.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused {
                status,
                error,
                detail,
            } => write!(f, "refused ({status} {error}): {detail}"),
            Self::Http(error) => write!(f, "request failed: {error}"),
            Self::Token(error) => error.fmt(f),
            Self::Response(detail) => write!(f, "unreadable answer: {detail}"),
        }
    }
}

impl std::error::Error for CallError {}

impl From<reqwest::Error> for CallError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<TokenError> for CallError {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}

#[derive(Deserialize)]
struct RefusalBody {
    error: String,
    detail: String,
}

/// Posts `action` as the body with a presentation of `token`, signed by `agent` at `now_ms`.
pub async fn post_action<T: DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    agent: &BindingKey,
    token: &[u8],
    action: &Action,
    now_ms: i64,
) -> Result<T, CallError> {
    let body =
        serde_json::to_vec(action).map_err(|error| CallError::Response(error.to_string()))?;
    let presentation = agent.present(token, &body, now_ms)?;
    let response = http
        .post(url)
        .header(AUTHORIZATION, authorization(&presentation)?)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .timeout(TIMEOUT)
        .send()
        .await?;
    read(response).await
}

/// Reads a JSON answer, or the refusal it carries.
pub async fn read<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, CallError> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if status.is_success() {
        return serde_json::from_slice(&bytes)
            .map_err(|error| CallError::Response(error.to_string()));
    }
    let refusal: RefusalBody = serde_json::from_slice(&bytes).map_err(|_| {
        CallError::Response(format!(
            "status {status} with body {:?}",
            String::from_utf8_lossy(&bytes)
        ))
    })?;
    Err(CallError::Refused {
        status: status.as_u16(),
        error: refusal.error,
        detail: refusal.detail,
    })
}
