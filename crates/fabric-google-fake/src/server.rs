//! The HTTP/1.1 server (14.1): `std::net` and threads, one request per connection,
//! `Content-Length` bodies, `Connection: close`.
//!
//! Every connection gets a thread of its own. The head may be at most 16 KiB and the body at
//! most 1 MiB; a connection has 10 s to send its head and 10 s more for its body. A request the
//! server cannot read is logged with its problem and answered 4xx. A handler that panics is
//! counted and its connection closed without an answer; the server goes on. Dropping the
//! `FakeGoogle` stops the accept loop, wakes every delayed answer, joins every thread and frees
//! the port.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::faults::{AfterApply, BodyChange, Instead, LateTrigger, Plan};
use crate::log::LoggedRequest;
use crate::state::{LateChange, State};
use crate::url::{parse_pairs, percent_decode};
use crate::{calendar, oauth};

/// The most a request head may have.
pub(crate) const MAX_HEAD: usize = 16 * 1024;
/// The most a request body may have.
pub(crate) const MAX_BODY: usize = 1024 * 1024;
/// How long a connection has for its head, and again for its body.
const READ_WINDOW: Duration = Duration::from_secs(10);
/// How often a waiting read looks at the stop flag.
const POLL: Duration = Duration::from_millis(50);

/// A request as the server read it.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) connection: u64,
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) raw_path: String,
    pub(crate) path: String,
    pub(crate) segments: Vec<String>,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn query_all(&self, name: &str) -> Vec<&str> {
        self.query
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub(crate) fn form(&self) -> Option<Vec<(String, String)>> {
        let ct = self.header("content-type")?;
        if !media_type(ct).eq_ignore_ascii_case("application/x-www-form-urlencoded") {
            return None;
        }
        parse_pairs(std::str::from_utf8(&self.body).ok()?)
    }
}

/// The media type of a `Content-Type` value, without parameters.
pub(crate) fn media_type(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

/// An answer to write.
#[derive(Clone, Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn new(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn json(status: u16, value: &Value) -> Response {
        Response::new(status)
            .header("Content-Type", "application/json; charset=UTF-8")
            .body(serde_json::to_vec_pretty(value).unwrap_or_default())
    }

    pub(crate) fn html(status: u16, text: &str) -> Response {
        Response::new(status)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(format!("<!DOCTYPE html><html><body><p>{text}</p></body></html>").into_bytes())
    }

    pub(crate) fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub(crate) fn body(mut self, body: Vec<u8>) -> Response {
        self.body = body;
        self
    }

    /// Google's error shape for the Calendar API (`error.errors[0].reason`).
    pub(crate) fn google_error(status: u16, reason: &str, message: &str) -> Response {
        let domain = match reason {
            "rateLimitExceeded"
            | "userRateLimitExceeded"
            | "quotaExceeded"
            | "dailyLimitExceeded" => "usageLimits",
            _ => "global",
        };
        let mut error = json!({"domain": domain, "reason": reason, "message": message});
        if matches!(reason, "authError" | "required") {
            error["location"] = json!("Authorization");
            error["locationType"] = json!("header");
        }
        if reason == "conditionNotMet" {
            error["location"] = json!("If-Match");
            error["locationType"] = json!("header");
        }
        let mut body = json!({"error": {"errors": [error], "code": status, "message": message}});
        if let Some(s) = status_name(status) {
            body["error"]["status"] = json!(s);
        }
        let mut response = Response::json(status, &body);
        if status == 401 {
            response = response.header(
                "WWW-Authenticate",
                "Bearer realm=\"https://accounts.google.com/\", error=\"invalid_token\"",
            );
        }
        response
    }

    /// The OAuth error shape of the token and revocation endpoints.
    pub(crate) fn oauth_error(status: u16, error: &str, description: &str) -> Response {
        Response::json(
            status,
            &json!({"error": error, "error_description": description}),
        )
        .header(
            "Cache-Control",
            "no-cache, no-store, max-age=0, must-revalidate",
        )
        .header("Pragma", "no-cache")
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\n",
            self.status,
            reason_phrase(self.status)
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.body.len()
        ));
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

fn status_name(status: u16) -> Option<&'static str> {
    Some(match status {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        409 => "ABORTED",
        412 => "FAILED_PRECONDITION",
        429 => "RESOURCE_EXHAUSTED",
        500 => "INTERNAL",
        503 => "UNAVAILABLE",
        _ => return None,
    })
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        204 => "No Content",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "Status",
    }
}

/// What the accept loop and the connection threads share with the `FakeGoogle` value.
pub(crate) struct Shared {
    pub(crate) state: Mutex<State>,
    stopping: AtomicBool,
    stop_lock: Mutex<bool>,
    stop_signal: Condvar,
    handlers: Mutex<Vec<JoinHandle<()>>>,
    connections: AtomicU64,
}

impl Shared {
    pub(crate) fn new(state: State) -> Shared {
        Shared {
            state: Mutex::new(state),
            stopping: AtomicBool::new(false),
            stop_lock: Mutex::new(false),
            stop_signal: Condvar::new(),
            handlers: Mutex::new(Vec::new()),
            connections: AtomicU64::new(0),
        }
    }

    /// The state, also after a handler panicked while holding it.
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn connections(&self) -> u64 {
        self.connections.load(Ordering::SeqCst)
    }

    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Sleeps `duration`, or less when the server stops. True when it slept the whole time.
    fn sleep(&self, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        let mut stopped = self.stop_lock.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if *stopped {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            stopped = match self.stop_signal.wait_timeout(stopped, deadline - now) {
                Ok((guard, _)) => guard,
                Err(p) => p.into_inner().0,
            };
        }
    }

    /// Stops the server: no new connection is served, every sleep ends, and every connection
    /// thread is joined.
    pub(crate) fn stop(&self, addr: SocketAddr, accept: Option<JoinHandle<()>>) {
        self.stopping.store(true, Ordering::SeqCst);
        {
            let mut stopped = self.stop_lock.lock().unwrap_or_else(|p| p.into_inner());
            *stopped = true;
        }
        self.stop_signal.notify_all();
        // Wake the accept loop with a connection of its own.
        let _ = TcpStream::connect_timeout(&addr, Duration::from_secs(1));
        if let Some(handle) = accept {
            let _ = handle.join();
        }
        let handlers: Vec<JoinHandle<()>> =
            std::mem::take(&mut *self.handlers.lock().unwrap_or_else(|p| p.into_inner()));
        for handle in handlers {
            let _ = handle.join();
        }
    }
}

/// Starts the accept loop on `listener`.
pub(crate) fn spawn(listener: TcpListener, shared: Arc<Shared>) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("fake-google-accept".into())
        .spawn(move || accept_loop(listener, shared))
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) if shared.stopping() => return,
            Err(_) => {
                // A failed accept (too many open files, an aborted connection): try again.
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
        };
        if shared.stopping() {
            return;
        }
        let connection = shared.connections.fetch_add(1, Ordering::SeqCst) + 1;
        let worker = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name(format!("fake-google-conn-{connection}"))
            .spawn(move || serve(&worker, stream, connection));
        if let Ok(handle) = spawned {
            let mut handlers = shared.handlers.lock().unwrap_or_else(|p| p.into_inner());
            handlers.retain(|h| !h.is_finished());
            handlers.push(handle);
        }
    }
}

enum ReadError {
    /// The connection closed, or stayed idle, before sending anything: nothing to log.
    Idle,
    /// The server is stopping.
    Stopped,
    /// A request that cannot be served: log it with what could be read and answer `status`.
    Bad {
        status: u16,
        problem: &'static str,
        head: Option<Box<Request>>,
    },
}

fn serve(shared: &Shared, mut stream: TcpStream, connection: u64) {
    let _ = stream.set_nodelay(true);
    match read_request(shared, &mut stream, connection) {
        Ok(request) => dispatch(shared, stream, request),
        Err(ReadError::Idle | ReadError::Stopped) => {}
        Err(ReadError::Bad {
            status,
            problem,
            head,
        }) => {
            let request = head.map(|h| *h).unwrap_or_else(|| Request {
                connection,
                method: String::new(),
                target: String::new(),
                raw_path: String::new(),
                path: String::new(),
                segments: Vec::new(),
                query: Vec::new(),
                headers: Vec::new(),
                body: Vec::new(),
            });
            {
                let mut state = shared.lock();
                let seq = log_request(&mut state, &request);
                state.log[seq].problem = Some(problem.to_string());
                state.log[seq].answered = Some(status);
            }
            let response = Response::html(status, problem);
            let _ = stream.write_all(&response.to_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(Shutdown::Write);
            drain(&mut stream);
        }
    }
}

/// Reads what the client still sends, briefly, so that closing does not reset the connection
/// before the client read the answer.
fn drain(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut buffer = [0u8; 4096];
    let until = Instant::now() + Duration::from_millis(200);
    while Instant::now() < until {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn bad(status: u16, problem: &'static str, head: Option<Request>) -> ReadError {
    ReadError::Bad {
        status,
        problem,
        head: head.map(Box::new),
    }
}

fn read_request(
    shared: &Shared,
    stream: &mut TcpStream,
    connection: u64,
) -> Result<Request, ReadError> {
    let _ = stream.set_read_timeout(Some(POLL));
    let mut received = Vec::new();
    let mut buffer = [0u8; 8192];
    let deadline = Instant::now() + READ_WINDOW;
    let head_end = loop {
        if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        if received.windows(2).any(|w| w == b"\n\n") {
            return Err(bad(400, "a bare line feed in the head", None));
        }
        if received.len() > MAX_HEAD {
            return Err(bad(431, "the head is longer than 16 KiB", None));
        }
        if shared.stopping() {
            return Err(ReadError::Stopped);
        }
        if Instant::now() >= deadline {
            return Err(if received.is_empty() {
                ReadError::Idle
            } else {
                bad(408, "the head did not arrive in time", None)
            });
        }
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Err(if received.is_empty() {
                    ReadError::Idle
                } else {
                    bad(400, "the connection closed inside the head", None)
                });
            }
            Ok(n) => received.extend_from_slice(&buffer[..n]),
            Err(e) if retry(&e) => {}
            Err(_) => return Err(ReadError::Idle),
        }
    };
    if head_end > MAX_HEAD {
        return Err(bad(431, "the head is longer than 16 KiB", None));
    }
    let mut request = parse_head(&received[..head_end], connection)?;
    let mut body = received[head_end + 4..].to_vec();

    if request.header("transfer-encoding").is_some() {
        return Err(bad(
            411,
            "a Transfer-Encoding body (only Content-Length is read)",
            Some(request),
        ));
    }
    let lengths: Vec<&str> = request
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.as_str())
        .collect();
    let length = match lengths.as_slice() {
        [] => 0,
        [one] => match one.parse::<usize>() {
            Ok(n) if one.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => {
                return Err(bad(
                    400,
                    "a Content-Length that is not a number",
                    Some(request),
                ));
            }
        },
        _ => {
            return Err(bad(
                400,
                "Content-Length appears more than once",
                Some(request),
            ));
        }
    };
    if length > MAX_BODY {
        return Err(bad(413, "the body is longer than 1 MiB", Some(request)));
    }
    if request
        .header("expect")
        .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"))
        && body.len() < length
    {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let deadline = Instant::now() + READ_WINDOW;
    while body.len() < length {
        if shared.stopping() {
            return Err(ReadError::Stopped);
        }
        if Instant::now() >= deadline {
            return Err(bad(408, "the body did not arrive in time", Some(request)));
        }
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Err(bad(
                    400,
                    "the connection closed inside the body",
                    Some(request),
                ));
            }
            Ok(n) => body.extend_from_slice(&buffer[..n]),
            Err(e) if retry(&e) => {}
            Err(_) => return Err(ReadError::Idle),
        }
    }
    body.truncate(length);
    request.body = body;
    Ok(request)
}

fn retry(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

fn parse_head(head: &[u8], connection: u64) -> Result<Request, ReadError> {
    let Ok(text) = std::str::from_utf8(head) else {
        return Err(bad(400, "the head is not UTF-8", None));
    };
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split(' ').collect();
    let [method, target, version] = parts.as_slice() else {
        return Err(bad(400, "a broken request line", None));
    };
    let mut request = Request {
        connection,
        method: method.to_string(),
        target: target.to_string(),
        raw_path: String::new(),
        path: String::new(),
        segments: Vec::new(),
        query: Vec::new(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    for line in lines {
        if line.starts_with([' ', '\t']) {
            return Err(bad(400, "a folded header line", Some(request)));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(bad(400, "a header line without a colon", Some(request)));
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic() && b != b':') {
            return Err(bad(400, "a broken header name", Some(request)));
        }
        request.headers.push((
            name.to_string(),
            value.trim_matches([' ', '\t']).to_string(),
        ));
    }
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(bad(400, "a broken method", Some(request)));
    }
    if *version != "HTTP/1.1" {
        return Err(bad(505, "not HTTP/1.1", Some(request)));
    }
    if !target.starts_with('/') {
        return Err(bad(
            400,
            "a request target that is not a path",
            Some(request),
        ));
    }
    let hosts = request
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("host"))
        .count();
    if hosts != 1 {
        return Err(bad(400, "not exactly one Host header", Some(request)));
    }
    let (raw_path, query) = target.split_once('?').unwrap_or((target, ""));
    request.raw_path = raw_path.to_string();
    let mut segments = Vec::new();
    for segment in raw_path[1..].split('/') {
        match percent_decode(segment, false) {
            Some(s) => segments.push(s),
            None => {
                return Err(bad(
                    400,
                    "a path segment that does not decode",
                    Some(request),
                ));
            }
        }
    }
    request.path = format!("/{}", segments.join("/"));
    request.segments = segments;
    match parse_pairs(query) {
        Some(pairs) => request.query = pairs,
        None => return Err(bad(400, "a query that does not decode", Some(request))),
    }
    Ok(request)
}

pub(crate) fn log_request(state: &mut State, request: &Request) -> usize {
    let seq = state.log.len();
    let bearer = request
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let form = request.form();
    let token_account = match (request.path.as_str(), bearer, &form) {
        (_, Some(token), _) => state.subject_of_token(token),
        ("/token" | "/revoke", None, Some(form)) => form
            .iter()
            .find(|(n, _)| matches!(n.as_str(), "code" | "refresh_token" | "token"))
            .and_then(|(_, v)| state.subject_of_token(v)),
        ("/revoke", None, None) => request
            .query
            .iter()
            .find(|(n, _)| n == "token")
            .and_then(|(_, v)| state.subject_of_token(v)),
        _ => None,
    };
    let entry = LoggedRequest {
        seq,
        connection: request.connection,
        method: request.method.clone(),
        target: request.target.clone(),
        raw_path: request.raw_path.clone(),
        path: request.path.clone(),
        segments: request.segments.clone(),
        query: request.query.clone(),
        headers: request.headers.clone(),
        authorization: request.header("authorization").is_some(),
        token_account,
        if_match: request.header("if-match").map(str::to_string),
        content_type: request.header("content-type").map(str::to_string),
        accept_encoding: request.header("accept-encoding").map(str::to_string),
        body: request.body.clone(),
        form,
        problem: None,
        faults: Vec::new(),
        answered: None,
    };
    state.log.push(entry);
    seq
}

/// What to do with the connection after handling.
enum Outcome {
    Answer(Response),
    Close,
    Raw(Vec<u8>),
}

fn dispatch(shared: &Shared, mut stream: TcpStream, request: Request) {
    let handled = catch_unwind(AssertUnwindSafe(|| {
        let mut state = shared.lock();
        run_due_late_changes(&mut state);
        let seq = log_request(&mut state, &request);
        let plan = collect_plan(&mut state, &request);
        state.log[seq].faults = plan.labels.clone();
        for hook in &plan.hooks {
            hook(&mut state);
        }
        let outcome = decide(&mut state, &request, &plan);
        count_late_reads(&mut state, &request);
        state.log[seq].answered = match &outcome {
            Outcome::Answer(r) => Some(r.status),
            Outcome::Close => None,
            Outcome::Raw(bytes) => crate::client::HttpResponse::parse(bytes).map(|r| r.status),
        };
        (outcome, plan.delay)
    }));
    let (outcome, delay) = match handled {
        Ok(result) => result,
        Err(_) => {
            let mut state = shared.lock();
            state.handler_panics += 1;
            if let Some(entry) = state
                .log
                .iter_mut()
                .rev()
                .find(|e| e.connection == request.connection)
            {
                entry.problem = Some("the fake's handler panicked".into());
            }
            return;
        }
    };
    if let Some(delay) = delay
        && !shared.sleep(delay)
    {
        return;
    }
    let bytes = match outcome {
        Outcome::Answer(response) => response.to_bytes(),
        Outcome::Raw(bytes) => bytes,
        Outcome::Close => {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    };
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
    drain(&mut stream);
}

fn collect_plan(state: &mut State, request: &Request) -> Plan {
    let form = request.form();
    let grant_type = form.as_ref().and_then(|f| {
        f.iter()
            .find(|(n, _)| n == "grant_type")
            .map(|(_, v)| v.clone())
    });
    let mut plan = Plan::default();
    for armed in &mut state.faults {
        let fired = armed.fired.load(Ordering::SeqCst);
        if fired >= armed.fault.times {
            continue;
        }
        if !armed
            .fault
            .matcher
            .fits(&request.method, &request.path, grant_type.as_deref())
        {
            continue;
        }
        armed.fired.fetch_add(1, Ordering::SeqCst);
        for action in &armed.fault.actions {
            plan.add(action);
        }
    }
    state
        .faults
        .retain(|f| f.fired.load(Ordering::SeqCst) < f.fault.times);
    plan
}

fn decide(state: &mut State, request: &Request, plan: &Plan) -> Outcome {
    if let Some(instead) = &plan.instead {
        return match instead {
            Instead::Close => Outcome::Close,
            Instead::Raw(bytes) => Outcome::Raw(bytes.clone()),
            Instead::Json(status, body) => Outcome::Answer(apply_body_changes(
                Response::json(*status, body),
                &plan.body,
            )),
            Instead::Redirect(to) => {
                Outcome::Answer(Response::html(302, "Moved").header("Location", to))
            }
            Instead::Error {
                status,
                reason,
                retry_after,
            } => {
                let mut response = Response::google_error(*status, reason, &message_for(reason));
                if let Some(after) = retry_after {
                    response = response.header("Retry-After", after);
                }
                Outcome::Answer(apply_body_changes(response, &plan.body))
            }
            Instead::OAuthError {
                status,
                error,
                subtype,
            } => {
                let mut body = json!({"error": error, "error_description": "Bad Request"});
                if let Some(s) = subtype {
                    body["error_subtype"] = json!(s);
                }
                let response = Response::json(*status, &body).header(
                    "Cache-Control",
                    "no-cache, no-store, max-age=0, must-revalidate",
                );
                Outcome::Answer(apply_body_changes(response, &plan.body))
            }
        };
    }
    if let Some(trigger) = plan.late {
        let target_path = late_target(request);
        state.late.push(LateChange {
            request: request.clone(),
            target_path,
            trigger,
            reads: 0,
        });
        return Outcome::Close;
    }
    let response = route(state, request, plan);
    match &plan.after_apply {
        Some(AfterApply::Close) => Outcome::Close,
        Some(AfterApply::Error { status, reason }) => Outcome::Answer(apply_body_changes(
            Response::google_error(*status, reason, &message_for(reason)),
            &plan.body,
        )),
        None => Outcome::Answer(apply_body_changes(response, &plan.body)),
    }
}

/// A short message for a reason, as Google writes them.
pub(crate) fn message_for(reason: &str) -> String {
    match reason {
        "rateLimitExceeded" => "Rate Limit Exceeded",
        "userRateLimitExceeded" => "User Rate Limit Exceeded",
        "quotaExceeded" => "Calendar usage limits exceeded.",
        "forbiddenForNonOrganizer" => {
            "Shared properties can only be changed by the organizer of the event."
        }
        "insufficientPermissions" => "Insufficient Permission",
        "backendError" => "Backend Error",
        "notFound" => "Not Found",
        "deleted" => "Resource has been deleted",
        "conditionNotMet" => "Precondition Failed",
        "duplicate" => "The requested identifier already exists.",
        "authError" => "Invalid Credentials",
        "forbidden" => "Forbidden",
        _ => "Error",
    }
    .to_string()
}

fn route(state: &mut State, request: &Request, plan: &Plan) -> Response {
    let method = request.method.as_str();
    match request.path.as_str() {
        "/o/oauth2/v2/auth" if method == "GET" => oauth::authorize(state, request),
        "/token" if method == "POST" => oauth::token(state, request, plan),
        "/revoke" if method == "POST" => oauth::revoke(state, request),
        "/oauth2/v3/certs" if method == "GET" => oauth::certs(state),
        "/o/oauth2/v2/auth" | "/token" | "/revoke" | "/oauth2/v3/certs" => {
            Response::html(405, "Method Not Allowed")
        }
        path if path.starts_with("/calendar/v3/") => calendar::handle(state, request, plan),
        _ => Response::html(404, "Not Found"),
    }
}

/// The path whose GETs count for a late change: the event's own path (for an insert, the
/// collection path and the body's `id`).
fn late_target(request: &Request) -> String {
    if request.method == "POST"
        && request.segments.len() == 5
        && request.segments[4] == "events"
        && let Some(id) = serde_json::from_slice::<Value>(&request.body)
            .ok()
            .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    {
        return format!("{}/{}", request.path, id);
    }
    request.path.clone()
}

fn run_due_late_changes(state: &mut State) {
    let mut index = 0;
    while index < state.late.len() {
        let due = match state.late[index].trigger {
            LateTrigger::AfterReads(n) => state.late[index].reads >= n,
            LateTrigger::Manual => false,
        };
        if due {
            let late = state.late.remove(index);
            route(state, &late.request, &Plan::default());
        } else {
            index += 1;
        }
    }
}

/// Applies every pending late change now, whatever its trigger.
pub(crate) fn apply_all_late_changes(state: &mut State) -> usize {
    let pending = std::mem::take(&mut state.late);
    let count = pending.len();
    for late in pending {
        route(state, &late.request, &Plan::default());
    }
    count
}

fn count_late_reads(state: &mut State, request: &Request) {
    if request.method != "GET" {
        return;
    }
    for late in &mut state.late {
        if late.target_path == request.path {
            late.reads += 1;
        }
    }
}

fn apply_body_changes(mut response: Response, changes: &[BodyChange]) -> Response {
    for change in changes {
        response = match change {
            BodyChange::NotJson => response
                .header("Content-Type", "application/json; charset=UTF-8")
                .body(b"<!DOCTYPE html><html><body>Not JSON</body></html>".to_vec()),
            BodyChange::TooLong(n) => {
                let mut body = response.body.clone();
                if body.len() < *n {
                    body.resize(*n, b' ');
                }
                response.body(body)
            }
            BodyChange::ContentType(ct) => response.header("Content-Type", ct),
            BodyChange::Gzip => {
                let body = crate::gzip::compress(&response.body);
                response.header("Content-Encoding", "gzip").body(body)
            }
        };
    }
    response
}
