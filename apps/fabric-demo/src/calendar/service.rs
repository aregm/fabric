//! The calendar service's HTTP API (section 5.1). It listens on loopback only.
//!
//! Agent API, a token on every call:
//!
//! - `POST /v1/freebusy`: the body is a `read_free_busy` action. Returns
//!   `{"busy": [{"start_ms", "end_ms"}, ...]}`, clipped to the window and merged.
//! - `POST /v1/meetings`: the body is an `add_meeting` action. Writes one event to this person's
//!   calendar and returns the meeting's `start_ms` and `length_ms`.
//!
//! The token travels as `Authorization: FabricToken <hex of the presentation>`. No agent
//! response carries an event title or ID. A refusal is `{"error", "detail"}`, where `error` is a
//! token reason from section 9 or one of `malformed_request`, `wrong_endpoint` and
//! `store_failed`.
//!
//! Owner API, for the dashboard, with `Authorization: Bearer <owner secret>`: `GET
//! /v1/owner/events` lists events with titles, and `POST /v1/owner/reset` restores the
//! fixtures. `GET /v1/health` names the service.

use std::fs;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use fabric_policy::Action;
use fabric_token::{
    KeyDirectory, Presentation, Reason, Rejection, ReplayCache, SecretStore, Service, Verifier,
    VerifyingKey, hex, random, sha256,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::fixtures;
use super::freebusy::free_busy;
use super::store::{AddedBy, Event, Store, StoreKey};
use crate::audit::AuditLog;
use crate::clock::{Clock, ClockConfig, DemoClock};

/// The `Authorization` scheme that carries a token presentation.
pub const TOKEN_SCHEME: &str = "FabricToken";
/// Request bodies are actions, which are small.
const MAX_BODY_BYTES: usize = 16 * 1024;
/// The shortest owner secret accepted, in characters.
const MIN_OWNER_SECRET: usize = 32;

/// One calendar service's settings, from the per-run configuration the launcher writes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    pub person: String,
    /// How meeting titles name the other person: "Meeting with <peer_name> (Fabric)".
    pub peer_name: String,
    /// A loopback address. Port 0 picks a free port.
    pub listen: SocketAddr,
    pub store: PathBuf,
    pub fixtures: PathBuf,
    /// The launcher's key directory, for this person's gate key.
    pub keys: PathBuf,
    /// The per-run secret that the dashboard presents to the owner API.
    pub owner_secret: String,
    pub clock: ClockConfig,
    #[serde(default)]
    pub audit_log: Option<PathBuf>,
    /// Where the store key lives in Windows Credential Manager; tests use their own prefix.
    #[serde(default = "default_credential_prefix")]
    pub credential_prefix: String,
}

fn default_credential_prefix() -> String {
    "fabric-demo".to_owned()
}

impl CalendarConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let config: Self =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> Result<(), String> {
        fabric_policy::hol::identifier(&self.person).map_err(|error| error.to_string())?;
        if !self.listen.ip().is_loopback() {
            return Err(format!(
                "the calendar service listens on loopback only, not {}",
                self.listen
            ));
        }
        if self.owner_secret.len() < MIN_OWNER_SECRET {
            return Err(format!(
                "the owner secret needs at least {MIN_OWNER_SECRET} characters"
            ));
        }
        Ok(())
    }
}

/// Everything a calendar service needs, already loaded.
pub struct CalendarParts {
    pub person: String,
    pub peer_name: String,
    /// This person's policy-gate key, which every token must be signed with.
    pub gate_key: VerifyingKey,
    pub owner_secret: String,
    pub store: Store,
    /// What `POST /v1/owner/reset` restores.
    pub fixtures: Vec<Event>,
    pub clock: Arc<dyn Clock>,
    pub audit: AuditLog,
}

/// One person's calendar service.
pub struct Calendar {
    person: String,
    peer_name: String,
    verifier: Verifier,
    replay: ReplayCache,
    store: Mutex<Store>,
    fixtures: Vec<Event>,
    owner_secret_sha256: [u8; 32],
    clock: Arc<dyn Clock>,
    audit: AuditLog,
}

impl Calendar {
    #[must_use]
    pub fn new(parts: CalendarParts) -> Arc<Self> {
        Arc::new(Self {
            verifier: Verifier::new(Service::Calendar, &parts.person, parts.gate_key),
            person: parts.person,
            peer_name: parts.peer_name,
            replay: ReplayCache::new(),
            store: Mutex::new(parts.store),
            fixtures: parts.fixtures,
            owner_secret_sha256: sha256(parts.owner_secret.as_bytes()),
            clock: parts.clock,
            audit: parts.audit,
        })
    }

    /// Loads the gate key, the fixtures and the store (creating it from the fixtures on the
    /// first run), with the store key from `secrets`.
    pub fn from_config(
        config: &CalendarConfig,
        secrets: &dyn SecretStore,
    ) -> Result<Arc<Self>, String> {
        config.check()?;
        let gate_key = KeyDirectory::read(&config.keys)
            .and_then(|directory| directory.person(&config.person)?.gate())
            .map_err(|error| format!("{}: {error}", config.keys.display()))?;
        let fixtures = fixtures::load(&config.fixtures, &config.person)?;
        let key =
            StoreKey::load_or_create(secrets, &config.person).map_err(|error| error.to_string())?;
        if let Some(parent) = config.store.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        let store = Store::open_or_create(&config.store, &config.person, key, &fixtures)
            .map_err(|error| error.to_string())?;
        let clock = DemoClock::from_config(&config.clock)?;
        let audit = match &config.audit_log {
            Some(path) => {
                AuditLog::open(path).map_err(|error| format!("{}: {error}", path.display()))?
            }
            None => AuditLog::disabled(),
        };
        Ok(Self::new(CalendarParts {
            person: config.person.clone(),
            peer_name: config.peer_name.clone(),
            gate_key,
            owner_secret: config.owner_secret.clone(),
            store,
            fixtures,
            clock: Arc::new(clock),
            audit,
        }))
    }

    /// The audience this service accepts and the name it logs under: `calendar:<person>`.
    #[must_use]
    pub fn component(&self) -> &str {
        self.verifier.audience()
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn agent_request(&self, endpoint: Endpoint, headers: &HeaderMap, body: &[u8]) -> Response {
        let now_ms = self.clock.now_ms();
        let mut record = json!({
            "at_ms": now_ms,
            "component": self.component(),
            "endpoint": endpoint.path(),
        });
        let response = match self.run(endpoint, headers, body, now_ms, &mut record) {
            Ok(response) => {
                record["outcome"] = "executed".into();
                response
            }
            Err(failure) => {
                record["outcome"] = "refused".into();
                record["error"] = failure.error.into();
                record["detail"] = failure.detail.clone().into();
                failure.into_response()
            }
        };
        self.audit.record(&record);
        response
    }

    fn run(
        &self,
        endpoint: Endpoint,
        headers: &HeaderMap,
        body: &[u8],
        now_ms: i64,
        record: &mut Value,
    ) -> Result<Response, Failure> {
        let presentation = presentation(headers)?;
        let action: Action = serde_json::from_slice(body).map_err(|error| {
            Failure::new(
                StatusCode::BAD_REQUEST,
                "malformed_request",
                format!("the body is not an action: {error}"),
            )
        })?;
        record["action"] = action.kind().into();
        if action.kind() != endpoint.kind() {
            return Err(Failure::new(
                StatusCode::BAD_REQUEST,
                "wrong_endpoint",
                format!(
                    "{} runs {}, not {}",
                    endpoint.path(),
                    endpoint.kind(),
                    action.kind()
                ),
            ));
        }
        let accepted = self
            .verifier
            .check(&presentation, &action, body, now_ms, &self.replay)?;
        record["token"] = serde_json::to_value(&accepted.claims).unwrap_or(Value::Null);

        match action {
            Action::ReadFreeBusy { start_ms, end_ms } => {
                let busy = free_busy(self.store().events(), start_ms, end_ms);
                accepted.commit();
                record["busy_intervals"] = busy.len().into();
                Ok(Json(json!({ "busy": busy })).into_response())
            }
            Action::AddMeeting {
                negotiation,
                start_ms,
                length_ms,
            } => {
                let event = Event {
                    id: event_id()?,
                    start_ms,
                    length_ms,
                    title: format!("Meeting with {} (Fabric)", self.peer_name),
                    busy: true,
                    added_by: Some(AddedBy {
                        negotiation,
                        proof: hex::encode(&accepted.claims.proof),
                        token: hex::encode(&accepted.claims.nonce),
                    }),
                };
                // On failure `accepted` is dropped uncommitted, which releases the token.
                self.store().add(event).map_err(store_failed)?;
                accepted.commit();
                let meeting = json!({ "start_ms": start_ms, "length_ms": length_ms });
                Ok((StatusCode::CREATED, Json(meeting)).into_response())
            }
            other => Err(Failure::new(
                StatusCode::BAD_REQUEST,
                "wrong_endpoint",
                format!("no endpoint runs {}", other.kind()),
            )),
        }
    }

    fn owner_request(
        &self,
        endpoint: &str,
        headers: &HeaderMap,
        run: impl FnOnce(&Self) -> Result<Response, Failure>,
    ) -> Response {
        let result = self.check_owner(headers).and_then(|()| run(self));
        let mut record = json!({
            "at_ms": self.clock.now_ms(),
            "component": self.component(),
            "endpoint": endpoint,
        });
        let response = match result {
            Ok(response) => {
                record["outcome"] = "executed".into();
                response
            }
            Err(failure) => {
                record["outcome"] = "refused".into();
                record["error"] = failure.error.into();
                failure.into_response()
            }
        };
        self.audit.record(&record);
        response
    }

    fn check_owner(&self, headers: &HeaderMap) -> Result<(), Failure> {
        let given = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        // Comparing digests keeps the comparison time independent of the secret.
        if sha256(given.as_bytes()) == self.owner_secret_sha256 {
            Ok(())
        } else {
            Err(Failure::new(
                StatusCode::UNAUTHORIZED,
                "owner_unauthorized",
                "the owner API needs this run's owner secret",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Endpoint {
    FreeBusy,
    Meetings,
}

impl Endpoint {
    const fn path(self) -> &'static str {
        match self {
            Self::FreeBusy => "/v1/freebusy",
            Self::Meetings => "/v1/meetings",
        }
    }

    const fn kind(self) -> &'static str {
        match self {
            Self::FreeBusy => "read_free_busy",
            Self::Meetings => "add_meeting",
        }
    }
}

/// A refused request: an HTTP status and `{"error", "detail"}`.
#[derive(Debug)]
struct Failure {
    status: StatusCode,
    error: &'static str,
    detail: String,
}

impl Failure {
    fn new(status: StatusCode, error: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            error,
            detail: detail.into(),
        }
    }
}

impl From<Rejection> for Failure {
    /// 401 when the token cannot be trusted at all, 403 when it is genuine but does not allow
    /// this request.
    fn from(rejection: Rejection) -> Self {
        let status = match rejection.reason {
            Reason::BadSignature | Reason::Expired | Reason::PopInvalid => StatusCode::UNAUTHORIZED,
            Reason::WrongAudience | Reason::TokenReplayed | Reason::TokenArgumentMismatch => {
                StatusCode::FORBIDDEN
            }
        };
        Self::new(status, rejection.reason.name(), rejection.detail)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.error, "detail": self.detail });
        (self.status, Json(body)).into_response()
    }
}

fn store_failed(error: impl std::fmt::Display) -> Failure {
    Failure::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "store_failed",
        error.to_string(),
    )
}

/// Reads `Authorization: FabricToken <hex>`. A missing or unreadable token is a
/// `bad_signature` (section 9).
fn presentation(headers: &HeaderMap) -> Result<Presentation, Failure> {
    let unreadable = |detail: &str| {
        Failure::new(
            StatusCode::UNAUTHORIZED,
            Reason::BadSignature.name(),
            detail.to_owned(),
        )
    };
    let value = headers
        .get(AUTHORIZATION)
        .ok_or_else(|| unreadable("the request carries no token"))?
        .to_str()
        .map_err(|_| unreadable("the Authorization header is not ASCII"))?;
    let encoded = value
        .strip_prefix(TOKEN_SCHEME)
        .and_then(|rest| rest.strip_prefix(' '))
        .ok_or_else(|| unreadable("the Authorization scheme is not FabricToken"))?;
    let bytes =
        hex::decode(encoded.trim()).map_err(|_| unreadable("the token is not lowercase hex"))?;
    Presentation::from_bytes(&bytes).map_err(|error| unreadable(&error.to_string()))
}

fn event_id() -> Result<String, Failure> {
    let bytes = random::<8>().map_err(store_failed)?;
    Ok(format!("ev-{}", hex::encode(&bytes)))
}

/// The service's routes.
pub fn router(calendar: Arc<Calendar>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/freebusy", post(read_free_busy))
        .route("/v1/meetings", post(add_meeting))
        .route("/v1/owner/events", get(owner_events))
        .route("/v1/owner/reset", post(owner_reset))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(calendar)
}

async fn health(State(calendar): State<Arc<Calendar>>) -> Json<Value> {
    Json(json!({ "service": calendar.component() }))
}

async fn read_free_busy(
    State(calendar): State<Arc<Calendar>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    calendar.agent_request(Endpoint::FreeBusy, &headers, &body)
}

async fn add_meeting(
    State(calendar): State<Arc<Calendar>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    calendar.agent_request(Endpoint::Meetings, &headers, &body)
}

async fn owner_events(State(calendar): State<Arc<Calendar>>, headers: HeaderMap) -> Response {
    calendar.owner_request("/v1/owner/events", &headers, |calendar| {
        let store = calendar.store();
        let body = json!({ "person": calendar.person, "events": store.events() });
        Ok(Json(body).into_response())
    })
}

async fn owner_reset(State(calendar): State<Arc<Calendar>>, headers: HeaderMap) -> Response {
    calendar.owner_request("/v1/owner/reset", &headers, |calendar| {
        calendar
            .store()
            .replace(calendar.fixtures.clone())
            .map_err(store_failed)?;
        Ok(Json(json!({ "events": calendar.fixtures.len() })).into_response())
    })
}

/// Binds a loopback address; any other address is refused.
pub async fn bind(address: SocketAddr) -> io::Result<TcpListener> {
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("the calendar service listens on loopback only, not {address}"),
        ));
    }
    TcpListener::bind(address).await
}

/// Serves `calendar` on `listener` until `shutdown` completes.
pub async fn serve(
    listener: TcpListener,
    calendar: Arc<Calendar>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    axum::serve(listener, router(calendar))
        .with_graceful_shutdown(shutdown)
        .await
}
