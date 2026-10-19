//! The calendar API as agents and the dashboard call it.

use fabric_policy::Action;
use fabric_token::BindingKey;
use serde::Deserialize;

use super::freebusy::Busy;
use super::store::Event;
pub use crate::client::CallError;
use crate::client::{TIMEOUT, post_action, read};

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
        let url = format!("{}/v1/freebusy", self.base);
        let response: FreeBusyResponse =
            post_action(&self.http, &url, agent, token, action, now_ms).await?;
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
        let url = format!("{}/v1/meetings", self.base);
        let _: serde_json::Value =
            post_action(&self.http, &url, agent, token, action, now_ms).await?;
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
}
