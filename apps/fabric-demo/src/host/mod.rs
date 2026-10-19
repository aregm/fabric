//! The trusted host (section 5.2): one per person, beside the calendar service. It runs the
//! person's messaging gateway (Phase 5) and policy gate (Phase 6); the consent signer joins in
//! Phase 7. The agent supplies only proposed actions; every fact Isabelle reasons about comes
//! from the host's own records (D14).
//!
//! Agent API, on loopback. All but the token-gated gateway actions need
//! `Authorization: Bearer <agent secret>`:
//!
//! - `POST /v1/agent/register`: `{"binding_key": "<hex>"}`, the agent's token-binding key.
//! - `GET /v1/agent/state`: the owner's request, the link and the negotiation.
//! - `POST /v1/gate/screen`: `{"actions": [...]}`; a verdict for each action.
//! - `POST /v1/gate/authorize`: `{"action": ...}`; the verdict, and a token if permitted.
//! - `POST /v1/gateway/handshake` and `POST /v1/gateway/send`: an action, with its token.
//! - `GET /v1/gateway/inbox?after=n&wait_ms=m` and `GET /v1/gateway/status`.
//!
//! Owner API, for the dashboard, with `Authorization: Bearer <owner secret>`:
//!
//! - `POST /v1/owner/request`: the owner's request (D18, D23).
//! - `POST /v1/owner/consent`: `{"candidate_id", "yes"}`, the owner's answer on a mutual option.
//! - `GET /v1/owner/status`: the request, the link and the negotiation.
//!
//! Refusals are `{"error", "detail"}`, as at the calendar service.

pub mod client;
pub mod gateway;
pub mod negotiation;
pub mod policy;

use std::fs;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use fabric_policy::time::parse_utc;
use fabric_policy::{Action, CheckReport, Context, Decision, Request, Role, Verdict};
use fabric_token::{KeyDirectory, PersonKeys, SecretStore, VerifyingKey, hex, random};
use fabric_wire::ConsentRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;

pub use client::HostClient;
pub use gateway::{Delivery, Event, Gateway, GatewayParts, Status};
pub use negotiation::NegotiationRecord;
pub use policy::{PolicyConfig, PolicyGate};

use crate::api::{BearerSecret, Failure, check_secret, presentation};
use crate::audit::AuditLog;
use crate::clock::{Clock, ClockConfig, DemoClock};
use crate::net::check_loopback;
use crate::relay::{RelayClient, check_mailbox};

const MAX_BODY_BYTES: usize = 64 * 1024;

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
    /// The per-run secret of this person's agent.
    pub agent_secret: String,
    /// The per-run secret of the dashboard's owner view.
    pub owner_secret: String,
    /// The policy gate; without it, screening and authorizing are refused.
    #[serde(default)]
    pub policy: Option<PolicyConfig>,
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
        check_secret("agent secret", &self.agent_secret)?;
        check_secret("owner secret", &self.owner_secret)
    }
}

/// The owner's request as the host keeps it: the typed request the policy gate reasons about,
/// and the preference text that only the owner's own agent reads (section 6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnerRequest {
    pub request: Request,
    pub preference: String,
}

/// What an owner files (D23). Times are ISO 8601 UTC.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestForm {
    pub peer: String,
    pub duration_minutes: i64,
    pub window_start: String,
    pub window_end: String,
    /// When the negotiation expires.
    pub expires_at: String,
    #[serde(default)]
    pub preference: String,
}

/// What the agent sees of its host's records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub person: String,
    pub peer: String,
    pub now_ms: i64,
    pub request: Option<OwnerRequest>,
    pub link: String,
    pub negotiation: NegotiationRecord,
    /// The owner's yes or no to each mutual option, from the consent signer (Phase 7).
    #[serde(default)]
    pub owner_decisions: Vec<Decision>,
}

/// The answer to a screening request: one verdict per action, in order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenAnswer {
    pub theory: String,
    pub proof_id: String,
    pub seconds: f64,
    pub verdicts: Vec<Verdict>,
}

/// The answer to an authorize request, with the token when the action was permitted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeAnswer {
    pub theory: String,
    pub proof_id: String,
    pub seconds: f64,
    pub verdict: Verdict,
    /// The token, as lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Everything a host needs, already loaded.
pub struct HostParts {
    pub gateway: Arc<Gateway>,
    pub policy: Option<PolicyGate>,
    pub agent_secret: String,
    pub owner_secret: String,
    pub clock: Arc<dyn Clock>,
    pub audit: AuditLog,
}

#[derive(Default)]
struct Records {
    request: Option<OwnerRequest>,
    binding_key: Option<VerifyingKey>,
}

/// One person's trusted host.
pub struct Host {
    gateway: Arc<Gateway>,
    policy: Option<PolicyGate>,
    agent_secret: BearerSecret,
    owner_secret: BearerSecret,
    clock: Arc<dyn Clock>,
    audit: AuditLog,
    records: Mutex<Records>,
}

impl Host {
    #[must_use]
    pub fn new(parts: HostParts) -> Arc<Self> {
        Arc::new(Self {
            gateway: parts.gateway,
            policy: parts.policy,
            agent_secret: BearerSecret::new(&parts.agent_secret),
            owner_secret: BearerSecret::new(&parts.owner_secret),
            clock: parts.clock,
            audit: parts.audit,
            records: Mutex::new(Records::default()),
        })
    }

    /// Loads this person's keys from `secrets` and the peer's from the key directory, and
    /// opens the policy gate's session on the shared Isabelle server. A coordinator opens its
    /// negotiation with a fresh ID.
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
        let audit = || match &config.audit_log {
            Some(path) => {
                AuditLog::open(path).map_err(|error| format!("{}: {error}", path.display()))
            }
            None => Ok(AuditLog::disabled()),
        };
        let clock: Arc<dyn Clock> = Arc::new(DemoClock::from_config(&config.clock)?);
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
            clock: clock.clone(),
            audit: audit()?,
        })?;
        let policy = config
            .policy
            .as_ref()
            .map(|policy| PolicyGate::connect(policy, keys.minter()))
            .transpose()?;
        Ok(Self::new(HostParts {
            gateway,
            policy,
            agent_secret: config.agent_secret.clone(),
            owner_secret: config.owner_secret.clone(),
            clock,
            audit: audit()?,
        }))
    }

    #[must_use]
    pub const fn gateway(&self) -> &Arc<Gateway> {
        &self.gateway
    }

    /// Stops the policy gate's session.
    pub fn stop(&self) {
        if let Some(policy) = &self.policy {
            policy.stop();
        }
    }

    fn records(&self) -> MutexGuard<'_, Records> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, mut record: Value) {
        record["at_ms"] = self.clock.now_ms().into();
        record["component"] = format!("host:{}", self.gateway.person()).into();
        self.audit.record(&record);
    }

    /// Files the owner's request. The negotiation opens now.
    pub fn file_request(&self, form: &RequestForm) -> Result<OwnerRequest, Failure> {
        if form.peer != self.gateway.peer() {
            return Err(Failure::bad_request(
                "unknown_peer",
                format!("this host negotiates only with {}", self.gateway.peer()),
            ));
        }
        let time = |text: &str| {
            parse_utc(text).ok_or_else(|| {
                Failure::bad_request("malformed_request", format!("{text:?} is not a UTC time"))
            })
        };
        let (window_start_ms, window_end_ms, expires_at_ms) = (
            time(&form.window_start)?,
            time(&form.window_end)?,
            time(&form.expires_at)?,
        );
        if form.duration_minutes <= 0 || window_start_ms >= window_end_ms {
            return Err(Failure::bad_request(
                "malformed_request",
                "the duration must be positive and the window must not be empty",
            ));
        }
        let request = OwnerRequest {
            request: Request {
                peer: form.peer.clone(),
                duration_ms: form.duration_minutes * 60 * 1_000,
                window_start_ms,
                window_end_ms,
                opened_at_ms: self.clock.now_ms(),
                expires_at_ms,
            },
            preference: form.preference.clone(),
        };
        self.records().request = Some(request.clone());
        self.record(json!({ "kind": "owner_request", "request": request.request }));
        Ok(request)
    }

    #[must_use]
    pub fn agent_state(&self) -> AgentState {
        let status = self.gateway.status();
        AgentState {
            person: self.gateway.person().to_owned(),
            peer: self.gateway.peer().to_owned(),
            now_ms: self.clock.now_ms(),
            request: self.records().request.clone(),
            link: status.link,
            negotiation: status.negotiation,
            owner_decisions: self.gateway.owner_decisions(),
        }
    }

    /// The owner's yes or no to a mutual option (section 12). The record expires with the
    /// owner's request.
    pub fn consent(&self, candidate_id: &str, yes: bool) -> Result<ConsentRecord, Failure> {
        let expires_ms = self
            .records()
            .request
            .as_ref()
            .map(|request| request.request.expires_at_ms)
            .ok_or_else(|| Failure::conflict("no_request", "the owner has not filed a request"))?;
        self.gateway.record_consent(candidate_id, yes, expires_ms)
    }

    /// Registers the agent's token-binding key; later tokens are bound to it.
    pub fn register_agent(&self, key: VerifyingKey) {
        self.records().binding_key = Some(key);
        self.record(
            json!({ "kind": "agent_registered", "binding_key": hex::encode(key.as_bytes()) }),
        );
    }

    /// The context for a check, from the host's own records (D14).
    fn context(&self) -> Result<Context, Failure> {
        let request = self
            .records()
            .request
            .as_ref()
            .map(|request| request.request.clone())
            .ok_or_else(|| Failure::conflict("no_request", "the owner has not filed a request"))?;
        Ok(Context {
            owner: self.gateway.person().to_owned(),
            now_ms: self.clock.now_ms(),
            request,
            negotiation: self.gateway.policy_negotiation(),
            consents: self.gateway.consent_facts(),
        })
    }

    fn policy(&self) -> Result<&PolicyGate, Failure> {
        self.policy.as_ref().ok_or_else(|| {
            Failure::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no_policy_gate",
                "this host runs without a policy gate",
            )
        })
    }

    pub async fn screen(&self, actions: Vec<Action>) -> Result<ScreenAnswer, Failure> {
        let policy = self.policy()?;
        let context = self.context()?;
        let report = policy.screen(context, actions).await.map_err(|error| {
            Failure::new(StatusCode::SERVICE_UNAVAILABLE, "policy_failed", error)
        })?;
        self.record(json!({
            "kind": "screen",
            "theory": report.theory,
            "proof_id": report.proof_id,
            "seconds": report.seconds,
            "verdicts": report.verdicts,
        }));
        Ok(ScreenAnswer {
            verdicts: report
                .verdicts
                .iter()
                .map(|verdict| verdict.verdict.clone())
                .collect(),
            theory: report.theory,
            proof_id: report.proof_id,
            seconds: report.seconds,
        })
    }

    pub async fn authorize(&self, action: Action) -> Result<AuthorizeAnswer, Failure> {
        let policy = self.policy()?;
        let holder = self.records().binding_key.ok_or_else(|| {
            Failure::conflict("agent_not_registered", "no agent key is registered")
        })?;
        let context = self.context()?;
        let report: CheckReport = policy.authorize(context, action).await.map_err(|error| {
            Failure::new(StatusCode::SERVICE_UNAVAILABLE, "policy_failed", error)
        })?;
        let verdict = report
            .verdicts
            .first()
            .map(|verdict| verdict.verdict.clone())
            .ok_or_else(|| Failure::internal("policy_failed", "the check returned no verdict"))?;
        let minted = if verdict.is_permit() {
            Some(
                policy
                    .mint(&report, &holder, self.clock.now_ms())
                    .map_err(|error| Failure::internal("mint_failed", error))?,
            )
        } else {
            None
        };
        self.record(json!({
            "kind": "authorize",
            "theory": report.theory,
            "proof_id": report.proof_id,
            "seconds": report.seconds,
            "verdicts": report.verdicts,
            "token": minted.as_ref().map(|minted| &minted.claims),
        }));
        Ok(AuthorizeAnswer {
            theory: report.theory,
            proof_id: report.proof_id,
            seconds: report.seconds,
            verdict,
            token: minted.map(|minted| hex::encode(&minted.bytes)),
        })
    }
}

/// A fresh negotiation ID, such as `n-3f2a9c1d`.
pub fn new_negotiation_id() -> Result<String, String> {
    random::<4>()
        .map(|bytes| format!("n-{}", hex::encode(&bytes)))
        .map_err(|error| error.to_string())
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8], what: &str) -> Result<T, Failure> {
    serde_json::from_slice(body).map_err(|error| {
        Failure::bad_request(
            "malformed_request",
            format!("the body is not {what}: {error}"),
        )
    })
}

fn respond<T: Serialize>(result: Result<T, Failure>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(failure) => failure.into_response(),
    }
}

async fn handshake(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        let presentation = presentation(&headers)?;
        let action = parse(&body, "an action")?;
        host.gateway.handshake(&presentation, &action, &body).await
    }
    .await;
    respond(result)
}

async fn send(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        let presentation = presentation(&headers)?;
        let action = parse(&body, "an action")?;
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
    respond(
        host.agent_secret
            .check(&headers, "agent_unauthorized")
            .map(|()| host.gateway.status()),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    binding_key: String,
}

async fn register(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = (|| {
        host.agent_secret.check(&headers, "agent_unauthorized")?;
        let registration: Registration = parse(&body, "a registration")?;
        let key = hex::decode_array::<32>(&registration.binding_key)
            .ok()
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .ok_or_else(|| {
                Failure::bad_request("malformed_request", "the binding key is not an Ed25519 key")
            })?;
        host.register_agent(key);
        Ok(json!({ "registered": true }))
    })();
    respond(result)
}

async fn agent_state(State(host): State<Arc<Host>>, headers: HeaderMap) -> Response {
    respond(
        host.agent_secret
            .check(&headers, "agent_unauthorized")
            .map(|()| host.agent_state()),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenRequest {
    actions: Vec<Action>,
}

async fn screen(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        host.agent_secret.check(&headers, "agent_unauthorized")?;
        let request: ScreenRequest = parse(&body, "a screening request")?;
        host.screen(request.actions).await
    }
    .await;
    respond(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizeRequest {
    action: Action,
}

async fn authorize(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        host.agent_secret.check(&headers, "agent_unauthorized")?;
        let request: AuthorizeRequest = parse(&body, "an authorize request")?;
        host.authorize(request.action).await
    }
    .await;
    respond(result)
}

async fn owner_request(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = (|| {
        host.owner_secret.check(&headers, "owner_unauthorized")?;
        let form: RequestForm = parse(&body, "a request")?;
        host.file_request(&form)
            .map(|request| json!({ "request": request }))
    })();
    respond(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsentForm {
    candidate_id: String,
    yes: bool,
}

async fn owner_consent(State(host): State<Arc<Host>>, headers: HeaderMap, body: Bytes) -> Response {
    let result = (|| {
        host.owner_secret.check(&headers, "owner_unauthorized")?;
        let form: ConsentForm = parse(&body, "a consent")?;
        host.consent(&form.candidate_id, form.yes)
            .map(|record| json!({ "consent": record }))
    })();
    respond(result)
}

async fn owner_status(State(host): State<Arc<Host>>, headers: HeaderMap) -> Response {
    respond(
        host.owner_secret
            .check(&headers, "owner_unauthorized")
            .map(|()| host.agent_state()),
    )
}

async fn health(State(host): State<Arc<Host>>) -> Json<Value> {
    Json(json!({ "service": format!("host:{}", host.gateway.person()) }))
}

pub fn router(host: Arc<Host>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/agent/register", post(register))
        .route("/v1/agent/state", get(agent_state))
        .route("/v1/gate/screen", post(screen))
        .route("/v1/gate/authorize", post(authorize))
        .route("/v1/gateway/handshake", post(handshake))
        .route("/v1/gateway/send", post(send))
        .route("/v1/gateway/inbox", get(inbox))
        .route("/v1/gateway/status", get(status))
        .route("/v1/owner/request", post(owner_request))
        .route("/v1/owner/consent", post(owner_consent))
        .route("/v1/owner/status", get(owner_status))
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
