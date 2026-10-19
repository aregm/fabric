//! The relay (section 5.4, task T5.1): FIFO mailboxes that carry frames between the gateways.
//! It is untrusted. It sees only Noise ciphertext, keeps frames in memory for 10 minutes, and
//! logs only each frame's mailbox, size and time.
//!
//! - `POST /v1/mailboxes/{id}`: the body is one frame; returns `{"seq": n}`.
//! - `GET /v1/mailboxes/{id}?after=n&wait_ms=m`: the frames after sequence number `n`, oldest
//!   first, as `{"frames": [{"seq", "at_ms", "frame": "<hex>"}]}`. With no frames yet, it waits
//!   up to `m` milliseconds (at most 25 seconds) for one.
//!
//! Mailboxes are random 128-bit IDs listed in the per-run configuration. Reading never removes
//! a frame; each reader keeps its own position.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use fabric_token::hex;
use fabric_wire::MAX_FRAME_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::{Instant, timeout_at};

use crate::audit::AuditLog;
use crate::clock::{Clock, ClockConfig, DemoClock};
use crate::net::check_loopback;

/// How long the relay keeps a frame (section 5.4).
pub const RETENTION_MS: i64 = 10 * 60 * 1_000;
/// The longest a poll waits for a frame.
pub const MAX_WAIT_MS: u64 = 25_000;
/// The most frames one poll returns.
const MAX_FRAMES_PER_POLL: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    pub listen: SocketAddr,
    /// One inbox per person: 32 lowercase hex digits each.
    pub mailboxes: Vec<String>,
    pub clock: ClockConfig,
    #[serde(default)]
    pub audit_log: Option<PathBuf>,
}

impl RelayConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let config: Self =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        check_loopback(config.listen)?;
        for mailbox in &config.mailboxes {
            check_mailbox(mailbox)?;
        }
        Ok(config)
    }
}

/// A mailbox ID is 128 random bits as 32 lowercase hex digits.
pub fn check_mailbox(id: &str) -> Result<(), String> {
    if hex::decode_array::<16>(id).is_ok() {
        Ok(())
    } else {
        Err(format!("mailbox ID {id:?} is not 32 lowercase hex digits"))
    }
}

/// A new random mailbox ID.
pub fn new_mailbox() -> Result<String, String> {
    fabric_token::random::<16>()
        .map(|bytes| hex::encode(&bytes))
        .map_err(|error| error.to_string())
}

#[derive(Debug, Default)]
struct Mailbox {
    next_seq: u64,
    frames: VecDeque<Stored>,
}

#[derive(Debug)]
struct Stored {
    seq: u64,
    at_ms: i64,
    bytes: Vec<u8>,
}

/// One frame as a reader receives it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayFrame {
    pub seq: u64,
    pub at_ms: i64,
    #[serde(with = "hex_bytes")]
    pub frame: Vec<u8>,
}

mod hex_bytes {
    use fabric_token::hex;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        hex::decode(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

pub struct Relay {
    mailboxes: Mutex<HashMap<String, Mailbox>>,
    arrivals: Notify,
    clock: Arc<dyn Clock>,
    audit: AuditLog,
}

impl Relay {
    #[must_use]
    pub fn new(mailboxes: &[String], clock: Arc<dyn Clock>, audit: AuditLog) -> Arc<Self> {
        Arc::new(Self {
            mailboxes: Mutex::new(
                mailboxes
                    .iter()
                    .map(|id| (id.clone(), Mailbox::default()))
                    .collect(),
            ),
            arrivals: Notify::new(),
            clock,
            audit,
        })
    }

    pub fn from_config(config: &RelayConfig) -> Result<Arc<Self>, String> {
        let clock = DemoClock::from_config(&config.clock)?;
        let audit = match &config.audit_log {
            Some(path) => {
                AuditLog::open(path).map_err(|error| format!("{}: {error}", path.display()))?
            }
            None => AuditLog::disabled(),
        };
        Ok(Self::new(&config.mailboxes, Arc::new(clock), audit))
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Mailbox>> {
        self.mailboxes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends a frame and wakes waiting readers. Returns its sequence number.
    fn append(&self, id: &str, bytes: Vec<u8>) -> Option<u64> {
        let now_ms = self.clock.now_ms();
        let size = bytes.len();
        let seq = {
            let mut mailboxes = self.lock();
            let mailbox = mailboxes.get_mut(id)?;
            expire(mailbox, now_ms);
            mailbox.next_seq += 1;
            let seq = mailbox.next_seq;
            mailbox.frames.push_back(Stored {
                seq,
                at_ms: now_ms,
                bytes,
            });
            seq
        };
        self.audit.record(&json!({
            "at_ms": now_ms,
            "component": "relay",
            "mailbox": id,
            "seq": seq,
            "bytes": size,
        }));
        self.arrivals.notify_waiters();
        Some(seq)
    }

    /// The frames after `after`, oldest first.
    fn frames_after(&self, id: &str, after: u64) -> Option<Vec<RelayFrame>> {
        let now_ms = self.clock.now_ms();
        let mut mailboxes = self.lock();
        let mailbox = mailboxes.get_mut(id)?;
        expire(mailbox, now_ms);
        Some(
            mailbox
                .frames
                .iter()
                .filter(|stored| stored.seq > after)
                .take(MAX_FRAMES_PER_POLL)
                .map(|stored| RelayFrame {
                    seq: stored.seq,
                    at_ms: stored.at_ms,
                    frame: stored.bytes.clone(),
                })
                .collect(),
        )
    }

    async fn poll(&self, id: &str, after: u64, wait_ms: u64) -> Option<Vec<RelayFrame>> {
        let deadline = Instant::now() + Duration::from_millis(wait_ms.min(MAX_WAIT_MS));
        loop {
            // Register for wakeups before looking, so a frame that arrives in between is seen.
            let mut arrival = pin!(self.arrivals.notified());
            arrival.as_mut().enable();
            let frames = self.frames_after(id, after)?;
            if !frames.is_empty() || Instant::now() >= deadline {
                return Some(frames);
            }
            if timeout_at(deadline, arrival).await.is_err() {
                return self.frames_after(id, after);
            }
        }
    }
}

fn expire(mailbox: &mut Mailbox, now_ms: i64) {
    while mailbox
        .frames
        .front()
        .is_some_and(|stored| now_ms - stored.at_ms >= RETENTION_MS)
    {
        mailbox.frames.pop_front();
    }
}

fn unknown_mailbox() -> Response {
    let body = json!({ "error": "unknown_mailbox", "detail": "no such mailbox" });
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}

#[derive(Deserialize)]
struct PollQuery {
    #[serde(default)]
    after: u64,
    #[serde(default)]
    wait_ms: u64,
}

async fn post_frame(
    State(relay): State<Arc<Relay>>,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> Response {
    match relay.append(&id, body.to_vec()) {
        Some(seq) => Json(json!({ "seq": seq })).into_response(),
        None => unknown_mailbox(),
    }
}

async fn poll_frames(
    State(relay): State<Arc<Relay>>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<PollQuery>,
) -> Response {
    match relay.poll(&id, query.after, query.wait_ms).await {
        Some(frames) => Json(json!({ "frames": frames })).into_response(),
        None => unknown_mailbox(),
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "service": "relay" }))
}

pub fn router(relay: Arc<Relay>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/mailboxes/{id}", get(poll_frames).post(post_frame))
        .layer(DefaultBodyLimit::max(MAX_FRAME_BYTES))
        .with_state(relay)
}

pub async fn serve(
    listener: TcpListener,
    relay: Arc<Relay>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    axum::serve(listener, router(relay))
        .with_graceful_shutdown(shutdown)
        .await
}

#[derive(Debug)]
pub enum RelayError {
    Http(reqwest::Error),
    /// The relay answered with an error or with something unreadable.
    Response(String),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(error) => write!(f, "relay: {error}"),
            Self::Response(detail) => write!(f, "relay: {detail}"),
        }
    }
}

impl std::error::Error for RelayError {}

impl From<reqwest::Error> for RelayError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

#[derive(Deserialize)]
struct PostResponse {
    seq: u64,
}

#[derive(Deserialize)]
struct PollResponse {
    frames: Vec<RelayFrame>,
}

/// A client for the relay, as gateways and tests use it.
#[derive(Clone, Debug)]
pub struct RelayClient {
    base: String,
    http: reqwest::Client,
}

impl RelayClient {
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn post(&self, mailbox: &str, frame: &[u8]) -> Result<u64, RelayError> {
        let response = self
            .http
            .post(format!("{}/v1/mailboxes/{mailbox}", self.base))
            .body(frame.to_vec())
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        let response: PostResponse = read(response).await?;
        Ok(response.seq)
    }

    pub async fn poll(
        &self,
        mailbox: &str,
        after: u64,
        wait_ms: u64,
    ) -> Result<Vec<RelayFrame>, RelayError> {
        let response = self
            .http
            .get(format!(
                "{}/v1/mailboxes/{mailbox}?after={after}&wait_ms={wait_ms}",
                self.base
            ))
            .timeout(Duration::from_millis(wait_ms.min(MAX_WAIT_MS) + 10_000))
            .send()
            .await?;
        let response: PollResponse = read(response).await?;
        Ok(response.frames)
    }
}

async fn read<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, RelayError> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(RelayError::Response(format!(
            "status {status}: {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    serde_json::from_slice(&bytes).map_err(|error| RelayError::Response(error.to_string()))
}
