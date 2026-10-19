//! The trusted host (section 5.2): one per person, beside the calendar service. Phase 5 runs its
//! messaging gateway; the policy gate and the consent signer join it in later phases.
//!
//! Agent API, on loopback:
//!
//! - `POST /v1/gateway/handshake`: the body is a `handshake` action, with a token for it.
//! - `POST /v1/gateway/send`: the body is a send action, with a token for it.
//! - `GET /v1/gateway/inbox?after=n&wait_ms=m`: deliveries after `n`, waiting up to `m`
//!   milliseconds for one. Needs `Authorization: Bearer <agent secret>`.
//! - `GET /v1/gateway/status`: the link and the negotiation. Needs the agent secret.
//!
//! Refusals are `{"error", "detail"}`, as at the calendar service.

pub mod client;
pub mod gateway;
pub mod negotiation;

use std::fs;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use fabric_policy::{Action, Role};
use fabric_token::{KeyDirectory, PersonKeys, SecretStore, hex, random};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;

pub use client::HostClient;
pub use gateway::{Delivery, Event, Gateway, GatewayParts, Status};
pub use negotiation::NegotiationRecord;

use crate::api::{BearerSecret, Failure, check_secret, presentation};
use crate::audit::AuditLog;
use crate::clock::{ClockConfig, DemoClock};
use crate::net::check_loopback;
use crate::relay::{RelayClient, check_mailbox};

const MAX_BODY_BYTES: usize = 16 * 1024;

/// One trusted host's settings, from the per-run configuration the launcher writes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub person: String,
    /// The one person this host's gateway talks to (Q21).
    pub peer: String,
    /// Alice coordinates in the scenario (D18).
    pub role: Role,
    pub listen: SocketAddr,
    /// The launcher's key directory, for the peer's public keys.
    pub keys: PathBuf,
    /// The relay's base URL.
    pub relay: String,
    /// This person's mailbox on the relay, and the peer's.
    pub inbox: String,
    pub peer_inbox: String,
    /// The per-run secret this person's agent presents to read its inbox.
    pub agent_secret: String,
    pub clock: ClockConfig,
    #[serde(default)]
    pub audit_log: Option<PathBuf>,
    /// Where this person's keys live in Windows Credential Manager.
    #[serde(default = "default_credential_prefix")]
    pub credential_prefix: String,
}

fn default_credential_prefix() -> String {
    "fabric-demo".to_owned()
}

impl HostConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let config: Self =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> Result<(), String> {
        for person in [&self.person, &self.peer] {
            fabric_policy::hol::identifier(person).map_err(|error| error.to_string())?;
        }
        if self.person == self.peer {
            return Err("a host cannot be its own peer".to_owned());
        }
        check_loopback(self.listen)?;
        check_mailbox(&self.inbox)?;
        check_mailbox(&self.peer_inbox)?;
        check_secret("agent secret", &self.agent_secret)
    }
}

/// One person's trusted host.
pub struct Host {
    gateway: Arc<Gateway>,
    agent_secret: BearerSecret,
}

impl Host {
    #[must_use]
    pub fn new(gateway: Arc<Gateway>, agent_secret: &str) -> Arc<Self> {
        Arc::new(Self {
            gateway,
            agent_secret: BearerSecret::new(agent_secret),
        })
    }

    /// Loads this person's keys from `secrets` and the peer's from the key directory. A
    /// coordinator opens its negotiation with a fresh ID.
    pub fn from_config(
        config: &HostConfig,
        secrets: &dyn SecretStore,
    ) -> Result<Arc<Self>, String> {
        config.check()?;
        let keys = PersonKeys::load(secrets, &config.person).map_err(|error| error.to_string())?;
        let directory = KeyDirectory::read(&config.keys)
            .map_err(|error| format!("{}: {error}", config.keys.display()))?;
        let own_keys = *directory
            .person(&config.person)
            .map_err(|error| error.to_string())?;
        if own_keys != keys.public() {
            return Err(format!(
                "the key directory's keys for {} are not the ones in the secret store",
                config.person
            ));
        }
        let peer_keys = *directory
            .person(&config.peer)
            .map_err(|error| error.to_string())?;
        let negotiation = match config.role {
            Role::Coordinator => Some(new_negotiation_id()?),
            Role::Invitee => None,
        };
        let audit = match &config.audit_log {
            Some(path) => {
                AuditLog::open(path).map_err(|error| format!("{}: {error}", path.display()))?
            }
            None => AuditLog::disabled(),
        };
        let gateway = Gateway::new(GatewayParts {
            person: config.person.clone(),
            peer: config.peer.clone(),
            role: config.role,
            negotiation,
            identity: keys.identity().clone(),
            noise_secret: keys.noise_secret(),
            own_keys,
            peer_keys,
            relay: RelayClient::new(&config.relay),
            inbox: config.inbox.clone(),
            peer_inbox: config.peer_inbox.clone(),
            clock: Arc::new(DemoClock::from_config(&config.clock)?),
            audit,
        })?;
        Ok(Self::new(gateway, &config.agent_secret))
    }

    #[must_use]
    pub const fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }
}

/// A fresh negotiation ID, such as `n-3f2a9c1d`.
pub fn new_negotiation_id() -> Result<String, String> {
    random::<4>()
        .map(|bytes| format!("n-{}", hex::encode(&bytes)))
        .map_err(|error| error.to_string())
}

fn action(body: &[u8]) -> Result<Action, Failure> {
    serde_json::from_slice(body).map_err(|error| {
        Failure::bad_request(
            "malformed_request",
            format!("the body is not an action: {error}"),
        )
    })
}

async fn handshake(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        let presentation = presentation(&headers)?;
        let action = action(&body)?;
        host.gateway.handshake(&presentation, &action, &body).await
    }
    .await;
    respond(result)
}

async fn send(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        let presentation = presentation(&headers)?;
        let action = action(&body)?;
        host.gateway.send(&presentation, &action, &body).await
    }
    .await;
    respond(result)
}

#[derive(Deserialize)]
struct InboxQuery {
    #[serde(default)]
    after: u64,
    #[serde(default)]
    wait_ms: u64,
}

async fn inbox(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    Query(query): Query<InboxQuery>,
) -> Response {
    if let Err(failure) = host.agent_secret.check(&headers, "agent_unauthorized") {
        return failure.into_response();
    }
    let deliveries = host.gateway.deliveries(query.after, query.wait_ms).await;
    Json(json!({ "deliveries": deliveries })).into_response()
}

async fn status(State(host): State<Arc<Host>>, headers: HeaderMap) -> Response {
    match host.agent_secret.check(&headers, "agent_unauthorized") {
        Ok(()) => Json(host.gateway.status()).into_response(),
        Err(failure) => failure.into_response(),
    }
}

async fn health(State(host): State<Arc<Host>>) -> Json<Value> {
    Json(json!({ "service": host.gateway.component() }))
}

fn respond(result: Result<Value, Failure>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(failure) => failure.into_response(),
    }
}

pub fn router(host: Arc<Host>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/gateway/handshake", post(handshake))
        .route("/v1/gateway/send", post(send))
        .route("/v1/gateway/inbox", get(inbox))
        .route("/v1/gateway/status", get(status))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(host)
}

/// Serves `host` on `listener` until `shutdown` completes. The caller runs the gateway's
/// inbound loop ([`Gateway::run_inbound`]).
pub async fn serve(
    listener: TcpListener,
    host: Arc<Host>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    axum::serve(listener, router(host))
        .with_graceful_shutdown(shutdown)
        .await
}
