//! The dashboard (section 13, task T8.1): one local page that shows both owners' views and both
//! agents' traces (D24). It serves plain HTML, CSS and JS with no build step, makes no external
//! requests, and pushes every component's audit records over Server-Sent Events.
//!
//! Components append audit records to per-run JSON-lines files; the dashboard tails them. Owner
//! actions go to each person's host and calendar service with that owner's per-run secret.
//!
//! - `GET /`: the page. It reads its key from the URL fragment, `#key=<secret>`.
//! - `GET /v1/overview`: both owners' requests, negotiations and calendars, and relay frames.
//! - `GET /v1/stream?key=…&after=n`: audit records after `n`, as Server-Sent Events.
//! - `GET /v1/proof?id=<proof ID>`: the check report behind a verdict.
//! - `POST /v1/people/{name}/request`, `/consent`, `/faults` and `/reset`: owner actions.
//!
//! Every API call needs the key, in `x-dashboard-key` or the `key` query parameter.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fs;
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HOST};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use fabric_policy::time::format_utc;
use futures_util::StreamExt as _;
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::api::{BearerSecret, Failure, check_secret};
use crate::clock::{Clock, ClockConfig, DemoClock};
use crate::host::RequestForm;
use crate::net::check_loopback;

const PAGE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../demo/dashboard/index.html"
));
const SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../demo/dashboard/app.js"
));
const STYLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../demo/dashboard/style.css"
));
/// Audit records the dashboard keeps in memory for pages that connect later.
const KEEP: usize = 5_000;
const CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// One person as the dashboard reaches them.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersonLinks {
    pub name: String,
    /// The name the page shows, such as "Alice".
    pub display: String,
    pub host: String,
    pub calendar: String,
    pub owner_secret: String,
    /// The request form, pre-filled with the scenario (D23).
    pub request: RequestForm,
    /// This person's mailbox on the relay.
    pub mailbox: String,
    /// `claude`, `ollama:<model>` or `scripted`.
    pub backend: String,
}

/// An audit log to tail.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogSource {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    pub listen: SocketAddr,
    /// The key the page presents.
    pub secret: String,
    pub people: Vec<PersonLinks>,
    pub relay: String,
    pub logs: Vec<LogSource>,
    /// The policy gates' check logs, for "View proof".
    pub checks: Vec<PathBuf>,
    pub clock: ClockConfig,
}

impl DashboardConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let config: Self =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        check_loopback(config.listen)?;
        check_secret("dashboard secret", &config.secret)?;
        Ok(config)
    }
}

#[derive(Default)]
struct Feed {
    next: u64,
    records: VecDeque<(u64, Value)>,
}

pub struct Dashboard {
    config: DashboardConfig,
    key: BearerSecret,
    http: reqwest::Client,
    clock: DemoClock,
    feed: Mutex<Feed>,
    updates: broadcast::Sender<(u64, Value)>,
    /// The `Host` header values the page may use; anything else is refused (DNS rebinding).
    hosts: Mutex<Vec<String>>,
}

impl Dashboard {
    pub fn new(config: DashboardConfig) -> Result<Arc<Self>, String> {
        let clock = DemoClock::from_config(&config.clock)?;
        Ok(Arc::new(Self {
            key: BearerSecret::new(&config.secret),
            http: reqwest::Client::new(),
            clock,
            feed: Mutex::new(Feed::default()),
            updates: broadcast::channel(1_024).0,
            hosts: Mutex::new(Vec::new()),
            config,
        }))
    }

    fn feed(&self) -> MutexGuard<'_, Feed> {
        self.feed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Accepts requests that name this address in `Host`.
    pub fn serve_at(&self, address: SocketAddr) {
        let port = address.port();
        *self.hosts.lock().unwrap_or_else(PoisonError::into_inner) = vec![
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ];
    }

    fn check(&self, headers: &HeaderMap, query_key: Option<&str>) -> Result<(), Failure> {
        let host = headers
            .get(HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        if !hosts.is_empty() && !hosts.iter().any(|allowed| allowed == host) {
            return Err(Failure::new(
                StatusCode::FORBIDDEN,
                "wrong_host",
                "unexpected Host header",
            ));
        }
        drop(hosts);
        let mut auth = HeaderMap::new();
        let given = headers
            .get("x-dashboard-key")
            .and_then(|value| value.to_str().ok())
            .or(query_key)
            .unwrap_or_default();
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {given}")) {
            auth.insert(axum::http::header::AUTHORIZATION, value);
        }
        self.key.check(&auth, "dashboard_unauthorized")
    }

    fn publish(&self, record: Value) {
        let mut feed = self.feed();
        feed.next += 1;
        let seq = feed.next;
        feed.records.push_back((seq, record.clone()));
        if feed.records.len() > KEEP {
            feed.records.pop_front();
        }
        drop(feed);
        let _ = self.updates.send((seq, record));
    }

    /// Tails every audit log for as long as the dashboard runs.
    pub async fn tail_logs(self: Arc<Self>) {
        let mut offsets = vec![0_u64; self.config.logs.len()];
        loop {
            for (index, source) in self.config.logs.iter().enumerate() {
                let Ok(text) = read_from(&source.path, &mut offsets[index]) else {
                    continue;
                };
                for line in text.lines() {
                    if let Ok(record) = serde_json::from_str::<Value>(line.trim())
                        && !own_read(&record)
                    {
                        self.publish(json!({ "source": source.name, "record": record }));
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    fn person(&self, name: &str) -> Result<&PersonLinks, Failure> {
        self.config
            .people
            .iter()
            .find(|person| person.name == name)
            .ok_or_else(|| Failure::new(StatusCode::NOT_FOUND, "unknown_person", name.to_owned()))
    }

    async fn call(&self, request: reqwest::RequestBuilder) -> Value {
        match request.timeout(CALL_TIMEOUT).send().await {
            Ok(response) => {
                let status = response.status();
                let value = match response.bytes().await {
                    Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                    Err(_) => Value::Null,
                };
                if status.is_success() {
                    value
                } else {
                    json!({ "error": value["error"], "detail": value["detail"], "status": status.as_u16() })
                }
            }
            Err(error) => json!({ "error": "unreachable", "detail": error.to_string() }),
        }
    }

    async fn overview(&self) -> Value {
        let mut people = Vec::new();
        let mut frames = Vec::new();
        for person in &self.config.people {
            let status = self
                .call(
                    self.http
                        .get(format!("{}/v1/owner/status", person.host))
                        .bearer_auth(&person.owner_secret),
                )
                .await;
            let calendar = self
                .call(
                    self.http
                        .get(format!("{}/v1/owner/events", person.calendar))
                        .bearer_auth(&person.owner_secret),
                )
                .await;
            people.push(json!({
                "name": person.name,
                "display": person.display,
                "backend": person.backend,
                "request_form": person.request,
                "status": status,
                "events": calendar["events"],
                "calendar_error": calendar["error"],
            }));
            let polled = self
                .call(self.http.get(format!(
                    "{}/v1/mailboxes/{}?after=0",
                    self.config.relay, person.mailbox
                )))
                .await;
            for frame in polled["frames"].as_array().into_iter().flatten() {
                frames.push(json!({
                    "to": person.name,
                    "mailbox": &person.mailbox[..8],
                    "seq": frame["seq"],
                    "at": frame["at_ms"].as_i64().map(format_utc),
                    "bytes": frame["frame"].as_str().map_or(0, |hex| hex.len() / 2),
                }));
            }
        }
        frames.sort_by_key(|frame| frame["at"].as_str().unwrap_or_default().to_owned());
        let now_ms = self.clock.now_ms();
        json!({
            "now_ms": now_ms,
            "now": format_utc(now_ms),
            "people": people,
            "relay": frames,
        })
    }

    fn proof(&self, id: &str) -> Option<Value> {
        for path in &self.config.checks {
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            for line in text.lines().rev() {
                if let Ok(report) = serde_json::from_str::<Value>(line)
                    && report["proof_id"] == id
                {
                    return Some(report);
                }
            }
        }
        None
    }

    async fn owner_action(&self, name: &str, action: &str, body: &[u8]) -> Result<Value, Failure> {
        let person = self.person(name)?;
        let (url, payload) = match action {
            "request" => {
                let form: RequestForm = if body.is_empty() {
                    person.request.clone()
                } else {
                    serde_json::from_slice(body).map_err(|error| {
                        Failure::bad_request("malformed_request", error.to_string())
                    })?
                };
                (format!("{}/v1/owner/request", person.host), json!(form))
            }
            "consent" | "faults" => (
                format!("{}/v1/owner/{action}", person.host),
                serde_json::from_slice(body).map_err(|error| {
                    Failure::bad_request("malformed_request", error.to_string())
                })?,
            ),
            "reset" => (format!("{}/v1/owner/reset", person.calendar), json!({})),
            _ => {
                return Err(Failure::new(
                    StatusCode::NOT_FOUND,
                    "unknown_action",
                    action.to_owned(),
                ));
            }
        };
        let answer = self
            .call(
                self.http
                    .post(url)
                    .bearer_auth(&person.owner_secret)
                    .header(CONTENT_TYPE, "application/json")
                    .body(payload.to_string()),
            )
            .await;
        self.publish(json!({
            "source": "dashboard",
            "record": { "kind": "owner_action", "person": name, "action": action, "answer": answer },
        }));
        Ok(answer)
    }
}

/// Reads the whole lines appended to `path` since `offset`. A line still being written stays
/// for the next read, so a character is never split.
fn read_from(path: &Path, offset: &mut u64) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    if length < *offset {
        *offset = 0;
    }
    file.seek(SeekFrom::Start(*offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let complete = bytes
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |end| end + 1);
    bytes.truncate(complete);
    *offset += complete as u64;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The dashboard's own polling of each owner's calendar. The calendar audits these reads like
/// any other owner request (section 5.1); the dashboard leaves them out of its feed.
fn own_read(record: &Value) -> bool {
    record["endpoint"] == "/v1/owner/events" && record["outcome"] == "executed"
}

#[derive(Deserialize)]
struct KeyQuery {
    key: Option<String>,
    #[serde(default)]
    after: u64,
    id: Option<String>,
}

fn static_file(content_type: &'static str, body: &'static str) -> Response {
    (
        [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-store")],
        body,
    )
        .into_response()
}

async fn page() -> Response {
    static_file("text/html; charset=utf-8", PAGE)
}

async fn script() -> Response {
    static_file("text/javascript; charset=utf-8", SCRIPT)
}

async fn style() -> Response {
    static_file("text/css; charset=utf-8", STYLE)
}

async fn overview(
    State(dashboard): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Query(query): Query<KeyQuery>,
) -> Response {
    if let Err(failure) = dashboard.check(&headers, query.key.as_deref()) {
        return failure.into_response();
    }
    Json(dashboard.overview().await).into_response()
}

async fn proof(
    State(dashboard): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Query(query): Query<KeyQuery>,
) -> Response {
    if let Err(failure) = dashboard.check(&headers, query.key.as_deref()) {
        return failure.into_response();
    }
    match query.id.as_deref().and_then(|id| dashboard.proof(id)) {
        Some(report) => Json(report).into_response(),
        None => {
            Failure::new(StatusCode::NOT_FOUND, "unknown_proof", "no such check").into_response()
        }
    }
}

async fn feed_stream(
    State(dashboard): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Query(query): Query<KeyQuery>,
) -> Response {
    if let Err(failure) = dashboard.check(&headers, query.key.as_deref()) {
        return failure.into_response();
    }
    // Subscribe before copying the backlog; the page drops records it has already seen.
    let updates = dashboard.updates.subscribe();
    let backlog: Vec<(u64, Value)> = dashboard
        .feed()
        .records
        .iter()
        .filter(|(seq, _)| *seq > query.after)
        .cloned()
        .collect();
    let event = |(seq, record): (u64, Value)| {
        Ok::<_, Infallible>(
            Event::default()
                .id(seq.to_string())
                .data(json!({ "seq": seq, "item": record }).to_string()),
        )
    };
    let live = stream::unfold(updates, |mut updates| async move {
        loop {
            match updates.recv().await {
                Ok(item) => return Some((item, updates)),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let events = stream::iter(backlog).chain(live).map(event);
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn person_action(
    State(dashboard): State<Arc<Dashboard>>,
    headers: HeaderMap,
    Query(query): Query<KeyQuery>,
    UrlPath((name, action)): UrlPath<(String, String)>,
    body: Bytes,
) -> Response {
    if let Err(failure) = dashboard.check(&headers, query.key.as_deref()) {
        return failure.into_response();
    }
    match dashboard.owner_action(&name, &action, &body).await {
        Ok(answer) => Json(answer).into_response(),
        Err(failure) => failure.into_response(),
    }
}

pub fn router(dashboard: Arc<Dashboard>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/app.js", get(script))
        .route("/style.css", get(style))
        .route("/v1/overview", get(overview))
        .route("/v1/proof", get(proof))
        .route("/v1/stream", get(feed_stream))
        .route("/v1/people/{name}/{action}", post(person_action))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(dashboard)
}

/// Serves the dashboard and tails the logs until `shutdown` completes.
pub async fn serve(
    listener: TcpListener,
    dashboard: Arc<Dashboard>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    dashboard.serve_at(listener.local_addr()?);
    let tail = tokio::spawn(dashboard.clone().tail_logs());
    let served = axum::serve(listener, router(dashboard))
        .with_graceful_shutdown(shutdown)
        .await;
    tail.abort();
    served
}
