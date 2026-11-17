//! The one place that talks HTTP (5.2, PD3).
//!
//! A [`Transport`] is made once for an [`Endpoints`] set. A [`Request`] names what it asks for
//! with a [`Target`] (never an address), carries query pairs, the headers the crate chooses
//! ([`Request::bearer`], [`Request::if_match`]), an optional JSON or form body, and the bound
//! for its answer ([`BodyBound`]). [`Transport::send`] takes the request and a deadline and gives
//! one of the three results of 5.2 as an [`Exchange`]:
//!
//! - [`Exchange::NothingSent`]: the deadline had passed before the request was handed to the
//!   client, or the connect phase failed (name lookup, TCP connect, TLS handshake).
//! - [`Exchange::Answer`]: a complete answer arrived: status, the few headers the crate reads,
//!   and the body within its bound.
//! - [`Exchange::SentUnanswered`]: every other failure, a time-out after connecting among them,
//!   and an answer that is no usable answer (a body over its bound, a compressed body).
//!
//! The rules of 5.2, and where each is enforced:
//!
//! - HTTPS only for [`Endpoints::google`] (the client is built with `https_only`), plain HTTP
//!   only for the loopback set, which a release build does not have. Addresses come only from
//!   the endpoint set and the typed [`Target`]; no caller passes one (2.2, 3.10).
//! - No redirect is followed (`redirect::Policy::none`): a 3xx comes back as an answer.
//! - No proxy (`no_proxy`, which also turns off the proxy variables of the environment), no
//!   cookies, no decompression (the manifest enables none of reqwest's compression features;
//!   `Accept-Encoding: identity` is sent and an answer with any other `Content-Encoding` is no
//!   usable answer).
//! - HTTP/1.1 only (`http1_only`). No connection is reused: the pool keeps no idle connection
//!   (`pool_max_idle_per_host(0)`, so hyper-util opens a new connection for every request) and
//!   every request says `Connection: close`. The client repeats nothing: reqwest's retry policy
//!   is `retry::never()`, and hyper-util repeats a cancelled request only on a reused
//!   connection, which never exists here.
//! - The deadline is the earlier of the caller's and now + 30 s ([`CALL_LIMIT_MS`]), and is
//!   reqwest's total time-out of the request, which covers connecting, sending and reading the
//!   whole body. Connecting has at most 10 s of it ([`CONNECT_LIMIT_MS`]).
//! - Body bounds by [`BodyBound`]; an answer that is not 2xx is an error answer and is held to
//!   64 KiB whatever the bound of the request.
//! - `User-Agent: FabricCalendar/<crate version>` ([`USER_AGENT`]) and no other identifying
//!   header.
//! - No error value of reqwest, serde_json or the url crate, and no address, is formatted,
//!   stored, logged or passed to a panic (PD27): each is mapped to a fixed label in this
//!   module. [`Request`], [`Answer`] and [`Transport`] have hand-written `Debug` that prints no
//!   address, no header value and no body.
//!
//! # How "nothing was sent" is told apart from "sent and unanswered"
//!
//! Read in reqwest 0.13.5 and hyper-util 0.1.21. reqwest's `Error::is_connect` is true exactly
//! when the error chain holds a hyper-util client error of kind `Connect`. hyper-util gives that
//! kind only to failures of the connector: the name lookup, the TCP connect, the connect
//! time-out that reqwest wraps around the connector, and the TLS handshake, which runs inside
//! the connector (hyper-tls / native-tls). Nothing of the request is written before the
//! connector has returned a connection, so such a failure is "nothing was sent". Every other
//! error, including the HTTP/1 handshake on an open connection, a failure to write the request,
//! a closed connection, a malformed answer and every time-out that is not the connect
//! time-out, is "sent and unanswered".
//!
//! What cannot be told apart, and is therefore reported on the safe side (sent and
//! unanswered, which a change sends to reconciliation, PD17):
//!
//! - The caller's deadline ending during the connect phase. reqwest's total time-out then fires
//!   first, and its error is not a connect error, although nothing was written. This happens
//!   only when less than 10 s of the deadline remain when the request is handed over.
//! - hyper's "cancelled before it was written" (`Canceled`): the connection closed after it was
//!   opened but before the request was written. hyper-util repeats such a request only on a
//!   reused connection, so here it is an error, and it is not a connect error.
//! - A request rejected by reqwest itself before sending (a bad scheme, an invalid URI). The
//!   request is formed so that this cannot happen (fixed endpoints, encoded segments), but it
//!   would not be a connect error either.
//!
//! The crate cannot tell whether the server read the request, only whether the connection was
//! open; a TCP connection that the peer accepted and closed is "sent and unanswered" even when
//! the peer read nothing.

pub mod ids;

use std::fmt;
use std::io::Read;
use std::time::Duration;

use reqwest::header::{self, HeaderMap, HeaderValue};
use zeroize::Zeroizing;

use crate::clock::SharedClock;
use crate::endpoints::{Endpoint, Endpoints, Scheme, Service};
use crate::secret::Secret;
use ids::{CalendarId, EventId, Revision};

/// `FabricCalendar/<crate version>`, the only identifying header (5.2).
pub const USER_AGENT: &str = concat!("FabricCalendar/", env!("CARGO_PKG_VERSION"));

/// The longest a call may take, whatever the caller's deadline (5.2; PLAN 5).
pub const CALL_LIMIT_MS: u64 = 30_000;

/// The longest the connect phase may take, inside the call's deadline (5.2).
pub const CONNECT_LIMIT_MS: u64 = 10_000;

/// The bound of an error answer (any status other than 2xx), whatever the request's bound.
pub const ERROR_ANSWER_LIMIT: usize = 64 * 1024;

/// The HTTP methods the adapter uses (section 6, 3.4, 3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Reads.
    Get,
    /// The token and revocation requests, free/busy, `events.insert`.
    Post,
    /// Changes of an existing event (6.7).
    Patch,
    /// Deleting an event (6.8).
    Delete,
}

impl Method {
    fn to_reqwest(self) -> reqwest::Method {
        match self {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
        }
    }

    /// Whether a request with this method can change Google (PD13).
    #[must_use]
    pub const fn is_changing(self) -> bool {
        !matches!(self, Method::Get)
    }
}

/// How much of an answer's body is read (5.2). More than the bound is no usable answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyBound {
    /// 64 KiB: the token, revocation and error answers.
    Small,
    /// 256 KiB: the key set and one event.
    Medium,
    /// 4 MiB: a list page.
    Large,
}

impl BodyBound {
    /// The bound in bytes, counted as received (no decompression).
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            BodyBound::Small => 64 * 1024,
            BodyBound::Medium => 256 * 1024,
            BodyBound::Large => 4 * 1024 * 1024,
        }
    }
}

/// What a request asks for. The address is the endpoint's, and the path is built from fixed
/// strings and checked ids only, each id one percent-encoded segment (5.3). There is no
/// variant that takes a string, so no unchecked text can reach a path.
#[derive(Clone, PartialEq, Eq)]
pub enum Target {
    /// The token endpoint (3.4, 3.8).
    Token,
    /// The revocation endpoint (3.9).
    Revocation,
    /// The key set (3.5 item 3).
    Keys,
    /// `<calendar>/users/me/calendarList` (6.2).
    CalendarList,
    /// `<calendar>/freeBusy` (6.3).
    FreeBusy,
    /// `<calendar>/calendars/<calendar id>/events` (6.4, 6.6).
    Events(CalendarId),
    /// `<calendar>/calendars/<calendar id>/events/<event id>` (6.5, 6.7, 6.8, section 7).
    Event(CalendarId, EventId),
}

impl Target {
    /// The endpoint this target is under.
    #[must_use]
    pub const fn service(&self) -> Service {
        match self {
            Target::Token => Service::Token,
            Target::Revocation => Service::Revocation,
            Target::Keys => Service::Keys,
            Target::CalendarList | Target::FreeBusy | Target::Events(_) | Target::Event(..) => {
                Service::Calendar
            }
        }
    }

    /// A fixed name for `Debug` and tests.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Target::Token => "token",
            Target::Revocation => "revocation",
            Target::Keys => "keys",
            Target::CalendarList => "calendar_list",
            Target::FreeBusy => "free_busy",
            Target::Events(_) => "events",
            Target::Event(..) => "event",
        }
    }

    /// The path: the endpoint's base path, then fixed segments and encoded ids.
    fn path(&self, endpoint: &Endpoint) -> String {
        let mut path = endpoint.path().to_owned();
        let mut push = |segment: &str| {
            path.push('/');
            encode_into(&mut path, segment);
        };
        match self {
            Target::Token | Target::Revocation | Target::Keys => {}
            Target::CalendarList => {
                push("users");
                push("me");
                push("calendarList");
            }
            Target::FreeBusy => push("freeBusy"),
            Target::Events(calendar) => {
                push("calendars");
                push(calendar.as_str());
                push("events");
            }
            Target::Event(calendar, event) => {
                push("calendars");
                push(calendar.as_str());
                push("events");
                push(event.as_str());
            }
        }
        path
    }
}

/// Prints the kind of target, never an id (g16).
impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Percent-encodes everything except `A-Z a-z 0-9 - . _ ~` (5.3), for path segments, query
/// names and values, and form bodies.
fn encode_into(out: &mut String, text: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
    }
}

/// Encodes name-value pairs as `name=value&...` with [`encode_into`].
fn encode_pairs<'a>(pairs: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let mut out = String::new();
    for (i, (name, value)) in pairs.enumerate() {
        if i > 0 {
            out.push('&');
        }
        encode_into(&mut out, name);
        out.push('=');
        encode_into(&mut out, value);
    }
    out
}

/// The address the browser is given (3.2.1): the endpoint's address and the pairs, each name
/// and value percent-encoded as in 5.3. It holds the state, the nonce and the challenge, so it
/// is a [`Secret`]. Only `oauth` uses it, with [`Endpoints::authorization`].
#[must_use]
pub fn browser_address(endpoint: &Endpoint, pairs: &[(&'static str, &str)]) -> Secret {
    let mut address = endpoint.address();
    if !pairs.is_empty() {
        address.push('?');
        address.push_str(&encode_pairs(pairs.iter().copied()));
    }
    Secret::new(address)
}

/// Why a request could not be formed. Nothing exists to send, so nothing was sent. A fixed
/// label; the offending value is not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// An access token holds a character that cannot be sent in a header (only `!` to `~`).
    TokenNotUsable,
    /// A header value could not be formed.
    HeaderNotFormed,
    /// The JSON body could not be serialized.
    BodyNotEncoded,
}

impl RequestError {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            RequestError::TokenNotUsable => "the access token cannot be sent in a header",
            RequestError::HeaderNotFormed => "a header value could not be formed",
            RequestError::BodyNotEncoded => "the request body could not be encoded",
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for RequestError {}

enum Body {
    None,
    Json(Vec<u8>),
    /// Form bodies carry codes, verifiers, refresh tokens and the client secret (3.4, 3.8,
    /// 3.9); the crate's own copy is wiped. reqwest's copy is not (RK14).
    Form(Zeroizing<Vec<u8>>),
}

/// One request, before it has an address. Built by the modules of the crate from a
/// [`Target`]; sent with [`Transport::send`].
pub struct Request {
    method: Method,
    target: Target,
    query: Vec<(&'static str, String)>,
    authorization: Option<HeaderValue>,
    if_match: Option<HeaderValue>,
    body: Body,
    bound: BodyBound,
}

impl Request {
    /// A request with no query, no body and the bound `bound`.
    #[must_use]
    pub fn new(method: Method, target: Target, bound: BodyBound) -> Request {
        Request {
            method,
            target,
            query: Vec::new(),
            authorization: None,
            if_match: None,
            body: Body::None,
            bound,
        }
    }

    /// Adds a query pair. The name is the crate's own constant; the value is percent-encoded
    /// (5.3), so it cannot change the path or add another pair.
    #[must_use]
    pub fn query(mut self, name: &'static str, value: impl Into<String>) -> Request {
        self.query.push((name, value.into()));
        self
    }

    /// Adds `Authorization: Bearer <token>`. The token must be `!` to `~` only (the
    /// characters an OAuth bearer token uses); the header value is marked sensitive.
    pub fn bearer(mut self, token: &Secret) -> Result<Request, RequestError> {
        let t = token.expose();
        if t.is_empty() || !t.bytes().all(|b| (b'!'..=b'~').contains(&b)) {
            return Err(RequestError::TokenNotUsable);
        }
        let mut value = HeaderValue::from_str(&format!("Bearer {t}"))
            .map_err(|_| RequestError::TokenNotUsable)?;
        value.set_sensitive(true);
        self.authorization = Some(value);
        Ok(self)
    }

    /// Adds `If-Match: <revision>`: one strong entity tag, never a wildcard (5.3, PD14).
    pub fn if_match(mut self, revision: &Revision) -> Result<Request, RequestError> {
        let value =
            HeaderValue::from_str(revision.as_str()).map_err(|_| RequestError::HeaderNotFormed)?;
        self.if_match = Some(value);
        Ok(self)
    }

    /// Sets a JSON body, sent as `application/json`.
    pub fn json(mut self, body: &serde_json::Value) -> Result<Request, RequestError> {
        let bytes = serde_json::to_vec(body).map_err(|_| RequestError::BodyNotEncoded)?;
        self.body = Body::Json(bytes);
        Ok(self)
    }

    /// Sets a form body, sent as `application/x-www-form-urlencoded`, every name and value
    /// percent-encoded as in 5.3 (a space is `%20`).
    #[must_use]
    pub fn form(mut self, pairs: &[(&'static str, &str)]) -> Request {
        let encoded = encode_pairs(pairs.iter().copied());
        self.body = Body::Form(Zeroizing::new(encoded.into_bytes()));
        self
    }

    /// The method.
    #[must_use]
    pub fn method(&self) -> Method {
        self.method
    }

    /// The target.
    #[must_use]
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The bound of the answer's body.
    #[must_use]
    pub fn bound(&self) -> BodyBound {
        self.bound
    }
}

/// Prints the method, the kind of target, how many query pairs, which headers and which kind of
/// body; never an id, a value or a body (5.2).
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let body = match self.body {
            Body::None => "none",
            Body::Json(_) => "json",
            Body::Form(_) => "form",
        };
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("target", &self.target)
            .field("query_pairs", &self.query.len())
            .field("bearer", &self.authorization.is_some())
            .field("if_match", &self.if_match.is_some())
            .field("body", &body)
            .field("bound", &self.bound)
            .finish()
    }
}

/// Why nothing was sent. Fixed labels (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSentReason {
    /// The deadline had passed before the request was handed to the client.
    DeadlinePassed,
    /// The name lookup failed.
    NameLookup,
    /// Connecting took longer than its 10 s.
    ConnectTimedOut,
    /// The TCP connect or the TLS handshake failed.
    ConnectFailed,
}

impl NotSentReason {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            NotSentReason::DeadlinePassed => "the deadline had passed before sending",
            NotSentReason::NameLookup => "the name lookup failed",
            NotSentReason::ConnectTimedOut => "connecting timed out",
            NotSentReason::ConnectFailed => "connecting failed",
        }
    }
}

impl fmt::Display for NotSentReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why a request that may have been sent has no usable answer. Fixed labels (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnansweredReason {
    /// The deadline ended after connecting began (or during it; see the module text).
    TimedOut,
    /// The connection failed while sending or before a complete answer head arrived, or the
    /// answer head was malformed.
    Transport,
    /// The answer head arrived but the body ended early or broke off.
    BodyIncomplete,
    /// The body was longer than its bound (5.2).
    BodyTooLarge,
    /// The answer carried a `Content-Encoding` other than `identity` (5.2: no compression).
    Compressed,
}

impl UnansweredReason {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            UnansweredReason::TimedOut => "no answer before the deadline",
            UnansweredReason::Transport => "the connection failed without a complete answer",
            UnansweredReason::BodyIncomplete => "the answer's body was incomplete",
            UnansweredReason::BodyTooLarge => "the answer's body was over its bound",
            UnansweredReason::Compressed => "the answer was compressed",
        }
    }
}

impl fmt::Display for UnansweredReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The result of [`Transport::send`]: exactly one of the three cases of 5.2.
#[derive(Debug)]
pub enum Exchange {
    /// Nothing was sent: the deadline had passed, or the connect phase failed.
    NothingSent(NotSentReason),
    /// A complete answer arrived, with its body inside the bound.
    Answer(Answer),
    /// The request may have been sent, and no usable answer arrived.
    SentUnanswered(UnansweredReason),
}

/// Why the body of an [`Answer`] is not the JSON the caller wanted. Fixed labels (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyError {
    /// The answer did not say `Content-Type: application/json` (5.2).
    NotJson,
    /// The body is not well-formed JSON.
    Unreadable,
}

impl BodyError {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            BodyError::NotJson => "the answer is not labelled as JSON",
            BodyError::Unreadable => "the answer is not well-formed JSON",
        }
    }
}

impl fmt::Display for BodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for BodyError {}

/// A complete answer: the status, the headers the crate reads, and the body (wiped on drop,
/// since a token answer holds tokens).
pub struct Answer {
    status: u16,
    content_type: Option<String>,
    retry_after: Option<String>,
    cache_control: Option<String>,
    body: Zeroizing<Vec<u8>>,
}

impl Answer {
    /// The status code. A 3xx is an answer like any other (5.2).
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Whether the status is 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the answer said `Content-Type: application/json` (parameters such as a charset
    /// allowed, case ignored), in exactly one header.
    #[must_use]
    pub fn is_json(&self) -> bool {
        self.content_type.as_deref().is_some_and(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
    }

    /// The body read as JSON (5.2): the answer must say `application/json` and be well-formed.
    /// The parser's error is dropped (PD27). Lenient reading into structs is the caller's.
    pub fn json(&self) -> Result<serde_json::Value, BodyError> {
        if !self.is_json() {
            return Err(BodyError::NotJson);
        }
        serde_json::from_slice(&self.body).map_err(|_| BodyError::Unreadable)
    }

    /// The raw body, at most its bound. Never format it (5.2).
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// `Retry-After` in whole seconds, when it is given as a number of seconds (6.10). An
    /// HTTP date or anything else is `None`. The cap of 120 s is the caller's (6.10).
    #[must_use]
    pub fn retry_after_seconds(&self) -> Option<u64> {
        let value = self.retry_after.as_deref()?.trim();
        if value.is_empty() || value.len() > 19 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        value.parse().ok()
    }

    /// The `max-age` of `Cache-Control` in seconds, when present (3.5 item 3). The bounds of
    /// 5 minutes and 24 hours are the caller's.
    #[must_use]
    pub fn max_age_seconds(&self) -> Option<u64> {
        let value = self.cache_control.as_deref()?;
        value.split(',').find_map(|directive| {
            let (name, number) = directive.trim().split_once('=')?;
            if !name.trim().eq_ignore_ascii_case("max-age") {
                return None;
            }
            let number = number.trim();
            if number.is_empty() || number.len() > 19 || !number.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            number.parse().ok()
        })
    }
}

/// Prints the status, the body's length and which headers were present; never a header value
/// or the body (5.2).
impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer")
            .field("status", &self.status)
            .field("json", &self.is_json())
            .field("retry_after", &self.retry_after.is_some())
            .field("cache_control", &self.cache_control.is_some())
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Why a [`Transport`] could not be made. Fixed labels (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// This build has no TLS, so it cannot reach Google (2.1: TLS is linked on macOS and
    /// Windows only; elsewhere only the loopback set can be reached).
    TlsUnavailable,
    /// The HTTP client could not be built (TLS backend or its thread).
    ClientNotBuilt,
    /// An endpoint address did not parse (cannot happen for the fixed sets).
    EndpointNotParsed,
}

impl SetupError {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            SetupError::TlsUnavailable => "this build cannot make TLS connections",
            SetupError::ClientNotBuilt => "the HTTP client could not be built",
            SetupError::EndpointNotParsed => "an endpoint address did not parse",
        }
    }
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for SetupError {}

/// The HTTP client of one endpoint set (5.2). Cheap to share by reference; one per adapter.
pub struct Transport {
    client: reqwest::blocking::Client,
    endpoints: Endpoints,
    /// The parsed base of each endpoint (scheme, host, port), indexed by [`Service`].
    bases: [reqwest::Url; 5],
    clock: SharedClock,
}

const fn service_index(service: Service) -> usize {
    match service {
        Service::Authorization => 0,
        Service::Token => 1,
        Service::Revocation => 2,
        Service::Keys => 3,
        Service::Calendar => 4,
    }
}

impl Transport {
    /// Builds the client for `endpoints` with every rule of 5.2 and parses the five base
    /// addresses once. `clock` gives the time for deadlines.
    pub fn new(endpoints: Endpoints, clock: SharedClock) -> Result<Transport, SetupError> {
        let https = matches!(endpoints.scheme(), Scheme::Https);
        if https && !cfg!(any(windows, target_os = "macos")) {
            return Err(SetupError::TlsUnavailable);
        }
        let builder = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .referer(false)
            .http1_only()
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_millis(CONNECT_LIMIT_MS))
            .timeout(Duration::from_millis(CALL_LIMIT_MS))
            .https_only(https);
        #[cfg(any(windows, target_os = "macos"))]
        let builder = builder.tls_version_min(reqwest::tls::Version::TLS_1_2);
        let client = builder.build().map_err(|_| SetupError::ClientNotBuilt)?;
        let parse = |service: Service| {
            let endpoint = endpoints.get(service);
            let origin = match endpoint.port() {
                Some(port) => format!(
                    "{}://{}:{}/",
                    endpoint.scheme().as_str(),
                    endpoint.host(),
                    port
                ),
                None => format!("{}://{}/", endpoint.scheme().as_str(), endpoint.host()),
            };
            reqwest::Url::parse(&origin).map_err(|_| SetupError::EndpointNotParsed)
        };
        let bases = [
            parse(Service::Authorization)?,
            parse(Service::Token)?,
            parse(Service::Revocation)?,
            parse(Service::Keys)?,
            parse(Service::Calendar)?,
        ];
        Ok(Transport {
            client,
            endpoints,
            bases,
            clock,
        })
    }

    /// The endpoint set this transport serves.
    #[must_use]
    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// The address of `request`, built from the endpoint, the target's segments and the
    /// encoded query. Infallible: setting the path and the query of a parsed URL cannot fail.
    fn url(&self, request: &Request) -> reqwest::Url {
        let service = request.target.service();
        let endpoint = self.endpoints.get(service);
        let mut url = self.bases[service_index(service)].clone();
        url.set_path(&request.target.path(&endpoint));
        if request.query.is_empty() {
            url.set_query(None);
        } else {
            let query = encode_pairs(request.query.iter().map(|(n, v)| (*n, v.as_str())));
            url.set_query(Some(&query));
        }
        url
    }

    /// Sends `request` once, with the deadline `deadline_ms` (Unix ms on the transport's clock)
    /// cut to now + 30 s, and returns one of the three results of 5.2. Never repeats.
    pub fn send(&self, request: Request, deadline_ms: u64) -> Exchange {
        let Some(remaining) = call_budget_ms(self.clock.now_ms(), deadline_ms) else {
            return Exchange::NothingSent(NotSentReason::DeadlinePassed);
        };

        let url = self.url(&request);
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
        if let Some(value) = request.authorization {
            headers.insert(header::AUTHORIZATION, value);
        }
        if let Some(value) = request.if_match {
            headers.insert(header::IF_MATCH, value);
        }
        let mut builder = self
            .client
            .request(request.method.to_reqwest(), url)
            .timeout(Duration::from_millis(remaining));
        match request.body {
            Body::None => {}
            Body::Json(bytes) => {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                builder = builder.body(bytes);
            }
            Body::Form(bytes) => {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
                builder = builder.body(bytes.to_vec());
            }
        }
        let builder = builder.headers(headers);

        let response = match builder.send() {
            Ok(response) => response,
            Err(error) => return classify_send_error(&error),
        };
        read_answer(response, request.bound)
    }
}

/// Prints which endpoint set, never an address.
impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport")
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

/// The time a call may take: the earlier of the deadline and now + 30 s, as a duration from
/// now (5.2). `None` when the deadline has passed.
fn call_budget_ms(now_ms: u64, deadline_ms: u64) -> Option<u64> {
    (deadline_ms > now_ms).then(|| (deadline_ms - now_ms).min(CALL_LIMIT_MS))
}

/// The split of the module text: a connect-phase error is nothing sent, every other error is
/// sent and unanswered. The error itself is dropped here (PD27).
fn classify_send_error(error: &reqwest::Error) -> Exchange {
    if error.is_connect() {
        let reason = if error.is_dns() {
            NotSentReason::NameLookup
        } else if error.is_timeout() {
            NotSentReason::ConnectTimedOut
        } else {
            NotSentReason::ConnectFailed
        };
        Exchange::NothingSent(reason)
    } else if error.is_timeout() {
        Exchange::SentUnanswered(UnansweredReason::TimedOut)
    } else {
        Exchange::SentUnanswered(UnansweredReason::Transport)
    }
}

/// The value of a header that appears exactly once and is visible ASCII; otherwise `None`.
fn single(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    first.to_str().ok().map(str::to_owned)
}

/// Reads the answer's head and body within the bound (5.2).
fn read_answer(response: reqwest::blocking::Response, bound: BodyBound) -> Exchange {
    let status = response.status().as_u16();
    let headers = response.headers();
    let compressed = headers.get_all(header::CONTENT_ENCODING).iter().any(|v| {
        v.to_str().map_or(true, |s| {
            s.split(',')
                .any(|coding| !coding.trim().eq_ignore_ascii_case("identity"))
        })
    });
    if compressed {
        return Exchange::SentUnanswered(UnansweredReason::Compressed);
    }
    let limit = if (200..300).contains(&status) {
        bound.bytes()
    } else {
        bound.bytes().min(ERROR_ANSWER_LIMIT)
    };
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Exchange::SentUnanswered(UnansweredReason::BodyTooLarge);
    }
    let content_type = single(headers, header::CONTENT_TYPE);
    let retry_after = single(headers, header::RETRY_AFTER);
    let cache_control = single(headers, header::CACHE_CONTROL);

    let mut body = Zeroizing::new(Vec::new());
    let mut reader = response.take(limit as u64 + 1);
    if let Err(error) = reader.read_to_end(&mut body) {
        let timed_out = error.kind() == std::io::ErrorKind::TimedOut
            || error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
                .is_some_and(reqwest::Error::is_timeout);
        return Exchange::SentUnanswered(if timed_out {
            UnansweredReason::TimedOut
        } else {
            UnansweredReason::BodyIncomplete
        });
    }
    if body.len() > limit {
        return Exchange::SentUnanswered(UnansweredReason::BodyTooLarge);
    }
    Exchange::Answer(Answer {
        status,
        content_type,
        retry_after,
        cache_control,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::clock::ManualClock;

    fn transport() -> Transport {
        Transport::new(
            Endpoints::loopback(40_123),
            Arc::new(ManualClock::new(1_000)),
        )
        .expect("a transport")
    }

    fn calendar(id: &str) -> CalendarId {
        CalendarId::new(id).expect("valid")
    }

    #[test]
    fn encoding_keeps_only_unreserved_characters() {
        let mut out = String::new();
        encode_into(&mut out, "aZ09-._~ /?#%@:+*&=\"'");
        assert_eq!(out, "aZ09-._~%20%2F%3F%23%25%40%3A%2B%2A%26%3D%22%27");
        let mut out = String::new();
        encode_into(&mut out, "\u{e9}");
        assert_eq!(out, "%C3%A9");
    }

    #[test]
    fn each_id_is_one_path_segment_after_the_url_library_has_parsed_it() {
        let t = transport();
        for id in [
            "a#b@group.calendar.google.com",
            "%2e%2e",
            "...",
            "a?b",
            "a b",
            ".%2e",
        ] {
            let request = Request::new(
                Method::Get,
                Target::Event(calendar(id), EventId::new("ev_1").expect("valid")),
                BodyBound::Medium,
            );
            let url = t.url(&request);
            let segments: Vec<&str> = url.path_segments().expect("segments").collect();
            assert_eq!(segments.len(), 6, "{id:?}");
            assert_eq!(&segments[..3], &["calendar", "v3", "calendars"]);
            assert_eq!(segments[4], "events");
            assert_eq!(segments[5], "ev_1");
            assert!(url.query().is_none());
            assert!(url.fragment().is_none());
        }
    }

    #[test]
    fn the_paths_of_every_target_are_fixed_strings_and_encoded_ids() {
        let t = transport();
        let cal = calendar("me@example.com");
        let ev = EventId::new("abc_123").expect("valid");
        let cases = [
            (Target::Token, "/token"),
            (Target::Revocation, "/revoke"),
            (Target::Keys, "/oauth2/v3/certs"),
            (Target::CalendarList, "/calendar/v3/users/me/calendarList"),
            (Target::FreeBusy, "/calendar/v3/freeBusy"),
            (
                Target::Events(cal.clone()),
                "/calendar/v3/calendars/me%40example.com/events",
            ),
            (
                Target::Event(cal, ev),
                "/calendar/v3/calendars/me%40example.com/events/abc_123",
            ),
        ];
        for (target, path) in cases {
            let url = t.url(&Request::new(Method::Get, target, BodyBound::Small));
            assert_eq!(url.path(), path);
            assert_eq!(url.host_str(), Some("127.0.0.1"));
            assert_eq!(url.port(), Some(40_123));
            assert_eq!(url.scheme(), "http");
        }
    }

    #[test]
    fn query_values_are_encoded_and_cannot_add_pairs() {
        let t = transport();
        let request = Request::new(Method::Get, Target::CalendarList, BodyBound::Large)
            .query("maxResults", "250")
            .query("pageToken", "a&b=c#d e+f");
        let url = t.url(&request);
        assert_eq!(
            url.query(),
            Some("maxResults=250&pageToken=a%26b%3Dc%23d%20e%2Bf")
        );
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[1].1, "a&b=c#d e+f");
        assert!(url.fragment().is_none());
    }

    #[test]
    fn a_token_with_a_control_or_space_character_is_refused() {
        for bad in ["", "a b", "a\nb", "a\u{e9}", "a\0"] {
            let r = Request::new(Method::Get, Target::Keys, BodyBound::Small)
                .bearer(&Secret::from(bad));
            assert_eq!(r.err(), Some(RequestError::TokenNotUsable), "{bad:?}");
        }
        assert!(
            Request::new(Method::Get, Target::Keys, BodyBound::Small)
                .bearer(&Secret::from("ya29.a0-_~+/="))
                .is_ok()
        );
    }

    #[test]
    fn the_browser_address_is_the_authorization_endpoint_with_encoded_pairs() {
        let address = browser_address(
            &Endpoints::google().authorization(),
            &[
                ("redirect_uri", "http://127.0.0.1:5000"),
                ("scope", "openid email"),
                ("state", "a-b_c~d"),
            ],
        );
        assert_eq!(
            address.expose(),
            "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A5000\
             &scope=openid%20email&state=a-b_c~d"
        );
        assert_eq!(format!("{address:?}"), "[redacted]");
        let bare = browser_address(&Endpoints::google().authorization(), &[]);
        assert_eq!(
            bare.expose(),
            "https://accounts.google.com/o/oauth2/v2/auth"
        );
    }

    #[test]
    fn the_call_budget_is_the_earlier_of_the_deadline_and_30_seconds() {
        assert_eq!(call_budget_ms(1_000, 1_000), None);
        assert_eq!(call_budget_ms(1_000, 999), None);
        assert_eq!(call_budget_ms(1_000, 1_001), Some(1));
        assert_eq!(call_budget_ms(1_000, 31_000), Some(30_000));
        assert_eq!(call_budget_ms(1_000, u64::MAX), Some(30_000));
        assert_eq!(CONNECT_LIMIT_MS, 10_000);
    }

    #[test]
    fn the_bounds_are_64_kib_256_kib_and_4_mib() {
        assert_eq!(BodyBound::Small.bytes(), 65_536);
        assert_eq!(BodyBound::Medium.bytes(), 262_144);
        assert_eq!(BodyBound::Large.bytes(), 4_194_304);
        assert_eq!(ERROR_ANSWER_LIMIT, 65_536);
    }

    #[test]
    fn the_user_agent_names_the_crate_version() {
        assert_eq!(
            USER_AGENT,
            format!("FabricCalendar/{}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn retry_after_and_max_age_parse_only_plain_numbers() {
        let answer = |retry: Option<&str>, cache: Option<&str>| Answer {
            status: 429,
            content_type: None,
            retry_after: retry.map(str::to_owned),
            cache_control: cache.map(str::to_owned),
            body: Zeroizing::new(Vec::new()),
        };
        assert_eq!(answer(Some("30"), None).retry_after_seconds(), Some(30));
        assert_eq!(answer(Some(" 7 "), None).retry_after_seconds(), Some(7));
        for bad in [
            "",
            "-1",
            "1.5",
            "Wed, 21 Oct 2015 07:28:00 GMT",
            "99999999999999999999",
        ] {
            assert_eq!(answer(Some(bad), None).retry_after_seconds(), None, "{bad}");
        }
        assert_eq!(answer(None, None).retry_after_seconds(), None);
        assert_eq!(
            answer(None, Some("public, max-age=19845, must-revalidate")).max_age_seconds(),
            Some(19_845)
        );
        assert_eq!(answer(None, Some("MAX-AGE=5")).max_age_seconds(), Some(5));
        assert_eq!(answer(None, Some("no-store")).max_age_seconds(), None);
        assert_eq!(answer(None, Some("max-age=")).max_age_seconds(), None);
        assert_eq!(answer(None, Some("max-age=-3")).max_age_seconds(), None);
    }

    #[test]
    fn json_needs_the_json_content_type() {
        let answer = |content_type: &str, body: &str| Answer {
            status: 200,
            content_type: Some(content_type.to_owned()),
            retry_after: None,
            cache_control: None,
            body: Zeroizing::new(body.as_bytes().to_vec()),
        };
        assert!(
            answer("application/json; charset=UTF-8", "{}")
                .json()
                .is_ok()
        );
        assert!(answer("Application/JSON", "{}").json().is_ok());
        assert_eq!(
            answer("text/html", "{}").json().err(),
            Some(BodyError::NotJson)
        );
        assert_eq!(
            answer("application/jsonx", "{}").json().err(),
            Some(BodyError::NotJson)
        );
        assert_eq!(
            answer("application/json", "{\"a\":").json().err(),
            Some(BodyError::Unreadable)
        );
    }
}
