//! The calendar API as agents and the dashboard call it.

use std::fmt;
use std::time::Duration;

use fabric_policy::Action;
use fabric_token::{BindingKey, TokenError, hex};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::freebusy::Busy;
use super::service::TOKEN_SCHEME;
use super::store::Event;

const TIMEOUT: Duration = Duration::from_secs(10);

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

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused {
                status,
                error,
                detail,
            } => write!(f, "refused ({status} {error}): {detail}"),
            Self::Http(error) => write!(f, "calendar service: {error}"),
            Self::Token(error) => error.fmt(f),
            Self::Response(detail) => write!(f, "calendar service: {detail}"),
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
struct FreeBusyResponse {
    busy: Vec<Busy>,
}

#[derive(Deserialize)]
struct EventsResponse {
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct ResetResponse {
    events: usize,
}

#[derive(Deserialize)]
struct RefusalBody {
    error: String,
    detail: String,
}

/// A client for one person's calendar service.
#[derive(Clone, Debug)]
pub struct CalendarClient {
    base: String,
    http: reqwest::Client,
}

impl CalendarClient {
    /// A client for the service at `base_url`, such as `http://127.0.0.1:47001`.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }
    }

    /// Runs a `read_free_busy` action with a token for it, signed by the agent at `now_ms`.
    pub async fn free_busy(
        &self,
        agent: &BindingKey,
        token: &[u8],
        action: &Action,
        now_ms: i64,
    ) -> Result<Vec<Busy>, CallError> {
        let response: FreeBusyResponse = self
            .agent_call("/v1/freebusy", agent, token, action, now_ms)
            .await?;
        Ok(response.busy)
    }

    /// Runs an `add_meeting` action with a token for it, signed by the agent at `now_ms`.
    pub async fn add_meeting(
        &self,
        agent: &BindingKey,
        token: &[u8],
        action: &Action,
        now_ms: i64,
    ) -> Result<(), CallError> {
        let _: serde_json::Value = self
            .agent_call("/v1/meetings", agent, token, action, now_ms)
            .await?;
        Ok(())
    }

    /// The owner's view: every event, with titles.
    pub async fn owner_events(&self, owner_secret: &str) -> Result<Vec<Event>, CallError> {
        let request = self
            .http
            .get(format!("{}/v1/owner/events", self.base))
            .bearer_auth(owner_secret);
        let response: EventsResponse = read(request.timeout(TIMEOUT).send().await?).await?;
        Ok(response.events)
    }

    /// Restores the fixtures and returns how many events they have.
    pub async fn owner_reset(&self, owner_secret: &str) -> Result<usize, CallError> {
        let request = self
            .http
            .post(format!("{}/v1/owner/reset", self.base))
            .bearer_auth(owner_secret);
        let response: ResetResponse = read(request.timeout(TIMEOUT).send().await?).await?;
        Ok(response.events)
    }

    async fn agent_call<T: DeserializeOwned>(
        &self,
        path: &str,
        agent: &BindingKey,
        token: &[u8],
        action: &Action,
        now_ms: i64,
    ) -> Result<T, CallError> {
        let body =
            serde_json::to_vec(action).map_err(|error| CallError::Response(error.to_string()))?;
        let presentation = agent.present(token, &body, now_ms)?;
        let authorization = format!("{TOKEN_SCHEME} {}", hex::encode(&presentation.to_bytes()?));
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .timeout(TIMEOUT)
            .send()
            .await?;
        read(response).await
    }
}

async fn read<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, CallError> {
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
