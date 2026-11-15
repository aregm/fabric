//! The trusted host's API as this person's agent calls it.

use std::time::Duration;

use fabric_policy::Action;
use fabric_token::{BindingKey, VerifyingKey, hex};
use serde::Deserialize;
use serde_json::{Value, json};

use super::gateway::{Delivery, MAX_WAIT_MS, Status};
use super::{AgentState, AuthorizeAnswer, ScreenAnswer};
use crate::agent::Faults;
use crate::client::{CallError, TIMEOUT, post_action, read};

#[derive(Deserialize)]
struct InboxResponse {
    deliveries: Vec<Delivery>,
}

/// A client for one person's trusted host.
#[derive(Clone, Debug)]
pub struct HostClient {
    base: String,
    agent_secret: String,
    http: reqwest::Client,
}

impl HostClient {
    /// A client for the host at `base_url`, with the run's agent secret.
    #[must_use]
    pub fn new(base_url: &str, agent_secret: &str) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_owned(),
            agent_secret: agent_secret.to_owned(),
            http: reqwest::Client::new(),
        }
    }

    /// Runs a `handshake` action. Returns `{"link": "calling"}` or `{"link": "connected"}`.
    pub async fn handshake(
        &self,
        agent: &BindingKey,
        token: &[u8],
        action: &Action,
        now_ms: i64,
    ) -> Result<Value, CallError> {
        let url = format!("{}/v1/gateway/handshake", self.base);
        post_action(&self.http, &url, agent, token, action, now_ms).await
    }

    /// Runs a send action. Returns `{"sent": "<type>"}`.
    pub async fn send(
        &self,
        agent: &BindingKey,
        token: &[u8],
        action: &Action,
        now_ms: i64,
    ) -> Result<Value, CallError> {
        let url = format!("{}/v1/gateway/send", self.base);
        post_action(&self.http, &url, agent, token, action, now_ms).await
    }

    /// The deliveries after `after`, waiting up to `wait_ms` for one.
    pub async fn inbox(&self, after: u64, wait_ms: u64) -> Result<Vec<Delivery>, CallError> {
        let wait_ms = wait_ms.min(MAX_WAIT_MS);
        let request = self
            .http
            .get(format!(
                "{}/v1/gateway/inbox?after={after}&wait_ms={wait_ms}",
                self.base
            ))
            .bearer_auth(&self.agent_secret)
            .timeout(TIMEOUT + Duration::from_millis(wait_ms));
        let response: InboxResponse = read(request.send().await?).await?;
        Ok(response.deliveries)
    }

    pub async fn status(&self) -> Result<Status, CallError> {
        let request = self
            .http
            .get(format!("{}/v1/gateway/status", self.base))
            .bearer_auth(&self.agent_secret)
            .timeout(TIMEOUT);
        read(request.send().await?).await
    }

    /// The fault switches the dashboard armed, which this call takes (Q24).
    pub async fn take_faults(&self) -> Result<Faults, CallError> {
        let request = self
            .http
            .post(format!("{}/v1/agent/faults", self.base))
            .bearer_auth(&self.agent_secret)
            .timeout(TIMEOUT);
        read(request.send().await?).await
    }

    /// Registers the agent's token-binding key.
    pub async fn register(&self, binding_key: &VerifyingKey) -> Result<(), CallError> {
        let request = self
            .http
            .post(format!("{}/v1/agent/register", self.base))
            .bearer_auth(&self.agent_secret)
            .json_body(&json!({ "binding_key": hex::encode(binding_key.as_bytes()) }))
            .timeout(TIMEOUT);
        let _: Value = read(request.send().await?).await?;
        Ok(())
    }

    pub async fn state(&self) -> Result<AgentState, CallError> {
        let request = self
            .http
            .get(format!("{}/v1/agent/state", self.base))
            .bearer_auth(&self.agent_secret)
            .timeout(TIMEOUT);
        read(request.send().await?).await
    }

    /// Screens actions; Isabelle checks them all in one theory.
    pub async fn screen(&self, actions: &[Action]) -> Result<ScreenAnswer, CallError> {
        let request = self
            .http
            .post(format!("{}/v1/gate/screen", self.base))
            .bearer_auth(&self.agent_secret)
            .json_body(&json!({ "actions": actions }))
            .timeout(CHECK_TIMEOUT);
        read(request.send().await?).await
    }

    /// The execution-time check of one action, with its token when permitted.
    pub async fn authorize(&self, action: &Action) -> Result<AuthorizeAnswer, CallError> {
        let request = self
            .http
            .post(format!("{}/v1/gate/authorize", self.base))
            .bearer_auth(&self.agent_secret)
            .json_body(&json!({ "action": action }))
            .timeout(CHECK_TIMEOUT);
        read(request.send().await?).await
    }
}

/// Isabelle's watchdog is 30 seconds per check (section 8.3); the call allows a little more.
const CHECK_TIMEOUT: Duration = Duration::from_secs(45);

trait JsonBody {
    fn json_body(self, value: &Value) -> Self;
}

impl JsonBody for reqwest::RequestBuilder {
    fn json_body(self, value: &Value) -> Self {
        self.header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(value.to_string())
    }
}
