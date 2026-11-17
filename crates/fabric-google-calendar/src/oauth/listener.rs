//! The loopback listener that hears Google's redirect (3.3, PD6).
//!
//! The rules of 3.3 and where each is kept:
//!
//! - It binds `127.0.0.1:0`, IPv4 only ([`Listener::bind`]).
//! - It lives at most [`LIFETIME_MS`] from the start of the flow, on the crate's [`Clock`] and
//!   on the monotonic system clock ([`Instant`]), whichever ends first, or until the flow is
//!   superseded; it reads all three every [`POLL`] (well under the 250 ms of 3.3). The crate's
//!   clock is the wall clock in production, which can be set back; the monotonic one cannot.
//!   Nothing else ends it without a result: wrong requests never do ([`Listener::run`]).
//! - It serves up to [`MAX_CONNECTIONS`] connections at once; a further one waits in the
//!   system's backlog until a slot is free. Each connection has [`HEAD_TIME`] to send a request
//!   head of at most [`MAX_HEAD`] bytes; no body is read. These 2 s are measured on the
//!   monotonic system clock, not the crate's [`Clock`]: they bound a socket, not the flow.
//! - Only `GET` of the path `/` with `Host: 127.0.0.1:<port>` is read. Another method or host is
//!   answered 400, another path (`/favicon.ico` among them) 404 ([`judge`]).
//! - `state` must appear exactly once and is compared in constant time ([`Secret::ct_eq`]).
//!   A wrong or missing state is answered 400 and does not end the flow.
//! - The first request with the right state ends the listener: `code` exactly once, or `error`
//!   exactly once; both or neither is a Google error. `iss`, when present, must be
//!   [`ISSUER`]. Every other parameter is ignored and dropped.
//! - Every answer is a fixed page with `Cache-Control: no-store`,
//!   `Content-Type: text/html; charset=utf-8` and `Connection: close` (and `Content-Length`,
//!   which only frames the page). Nothing of the request is echoed.
//! - [`Listener::run`] consumes the listener: the socket is closed, and every connection
//!   thread joined, before it returns, so before the token request is made (3.3: a code can be
//!   presented once).
//!
//! - After the answer the connection's write side is shut down, and what the browser still
//!   sends is read and dropped (at most [`DRAIN_MAX`] bytes, for at most [`DRAIN_TIME`])
//!   before the socket is closed: closing with unread bytes makes some systems reset the
//!   connection, and the browser may then lose the page.
//!
//! Threads: the accept loop runs on the thread that calls [`Listener::run`]. Each accepted
//! connection gets a thread of its own, which ends after at most [`HEAD_TIME`] for the head,
//! [`HEAD_TIME`] for writing the answer and [`DRAIN_TIME`] for the drain. `run` joins every one of them before it returns;
//! none is detached, so no thread outlives the flow.

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::clock::Clock;
use crate::secret::Secret;

/// The longest a listener lives, from the start of the flow, in ms (3.3; OQ2).
pub const LIFETIME_MS: u64 = 300_000;

/// How many connections are served at once (3.3).
pub const MAX_CONNECTIONS: usize = 8;

/// How long a connection has to send its request head (3.3), and to take the answer.
pub const HEAD_TIME: Duration = Duration::from_secs(2);

/// The largest request head read, in bytes (3.3).
pub const MAX_HEAD: usize = 8 * 1024;

/// How often the accept loop looks at the clocks and the supersede flag (3.3: at least every
/// 250 ms).
pub const POLL: Duration = Duration::from_millis(20);

/// The lifetime of [`LIFETIME_MS`] on the monotonic clock (3.3).
const LIFETIME: Duration = Duration::from_millis(LIFETIME_MS);

/// The most bytes read and dropped after an answer, before the connection is closed.
pub const DRAIN_MAX: usize = 4 * 1024;

/// The longest the drain after an answer waits for the browser to close its side.
pub const DRAIN_TIME: Duration = Duration::from_millis(200);

/// The issuer a callback's `iss` must name when present (3.3; G-DOC, RFC 9207).
pub const ISSUER: &str = "https://accounts.google.com";

/// The sentence of the page that ends a sign-in (3.3).
pub const PAGE_TEXT: &str = "You can close this window and return to Fabric Calendar.";

/// The sentence of every error page. Fixed; it echoes nothing.
pub const ERROR_TEXT: &str = "This address is not part of a Fabric Calendar sign-in.";

/// How a listener ended (3.3).
pub enum CallbackEnd {
    /// The right state and exactly one `code`.
    Code(Secret),
    /// The right state and `error=access_denied`.
    Denied,
    /// The right state with another `error`, both or neither of `code` and `error`, an empty
    /// or unreadable `code`, or an `iss` other than Google's.
    GoogleError,
    /// [`LIFETIME_MS`] passed on the crate's clock or on the monotonic clock.
    TimedOut,
    /// The supersede flag was set.
    Superseded,
}

/// Prints the kind, never the code.
impl fmt::Debug for CallbackEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CallbackEnd::Code(_) => "Code([redacted])",
            CallbackEnd::Denied => "Denied",
            CallbackEnd::GoogleError => "GoogleError",
            CallbackEnd::TimedOut => "TimedOut",
            CallbackEnd::Superseded => "Superseded",
        })
    }
}

/// The loopback socket could not be bound. A fixed label; the I/O error is dropped (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindFailed;

impl fmt::Display for BindFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the loopback listener could not be bound")
    }
}

impl std::error::Error for BindFailed {}

/// A bound loopback listener, not yet serving.
pub struct Listener {
    socket: TcpListener,
    port: u16,
}

impl Listener {
    /// Binds `127.0.0.1` on a port the system picks (3.3).
    pub fn bind() -> Result<Listener, BindFailed> {
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
        let socket = TcpListener::bind(address).map_err(|_| BindFailed)?;
        let port = socket.local_addr().map_err(|_| BindFailed)?.port();
        socket.set_nonblocking(true).map_err(|_| BindFailed)?;
        Ok(Listener { socket, port })
    }

    /// The port; the redirect is `http://127.0.0.1:<port>`.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Serves until the first request with the right `state`, until [`LIFETIME_MS`] after
    /// `started_ms` on `clock` or after `started_at` on the monotonic clock, or until
    /// `superseded` is set, whichever comes first. A time-out or a supersede seen by the loop
    /// wins over a callback that arrives while the loop shuts down. Closes the socket and
    /// joins every connection thread before it returns.
    pub fn run(
        self,
        state: &Secret,
        clock: &dyn Clock,
        started_ms: u64,
        started_at: Instant,
        superseded: &AtomicBool,
    ) -> CallbackEnd {
        let state = Arc::new(Secret::from(state.expose()));
        let result: Arc<Mutex<Option<CallbackEnd>>> = Arc::new(Mutex::new(None));
        let mut workers: Vec<JoinHandle<()>> = Vec::new();
        let ended_by_loop = loop {
            if lock(&result).is_some() {
                break None;
            }
            if superseded.load(Ordering::SeqCst) {
                break Some(CallbackEnd::Superseded);
            }
            if clock.now_ms().saturating_sub(started_ms) >= LIFETIME_MS
                || started_at.elapsed() >= LIFETIME
            {
                break Some(CallbackEnd::TimedOut);
            }
            workers.retain(|worker| !worker.is_finished());
            if workers.len() < MAX_CONNECTIONS {
                // No connection waiting, or one that failed before it was accepted: the
                // listener goes on either way (3.3: nothing ends it but time, a supersede or
                // the right state).
                if let Ok((stream, _)) = self.socket.accept() {
                    let (state, result) = (Arc::clone(&state), Arc::clone(&result));
                    let port = self.port;
                    // A thread that cannot be started drops its stream, which closes it.
                    if let Ok(worker) = std::thread::Builder::new()
                        .name("fabric-oauth-callback".to_owned())
                        .spawn(move || serve(stream, port, &state, &result))
                    {
                        workers.push(worker);
                    }
                    continue;
                }
            }
            std::thread::sleep(POLL);
        };
        drop(self.socket);
        for worker in workers {
            // A worker that panicked has answered nothing; its result is simply absent.
            let _ = worker.join();
        }
        let callback = lock(&result).take();
        match ended_by_loop {
            Some(end) => end,
            None => callback.unwrap_or(CallbackEnd::GoogleError),
        }
    }
}

/// Prints the port only.
impl fmt::Debug for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Listener(port {})", self.port)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What one request gets.
#[derive(Debug)]
pub enum Verdict {
    /// 400: another method, another host, a malformed head, a wrong or missing state.
    BadRequest,
    /// 404: another path.
    NotFound,
    /// The right state: the listener ends with this.
    Ends(EndKind),
}

/// How the request with the right state ends the listener.
pub enum EndKind {
    /// `code` exactly once, readable and not empty: the code.
    Code(Secret),
    /// `error=access_denied`.
    Denied,
    /// Any other end with the right state.
    GoogleError,
}

/// Prints the kind, never the code.
impl fmt::Debug for EndKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EndKind::Code(_) => "Code([redacted])",
            EndKind::Denied => "Denied",
            EndKind::GoogleError => "GoogleError",
        })
    }
}

impl From<EndKind> for CallbackEnd {
    fn from(end: EndKind) -> CallbackEnd {
        match end {
            EndKind::Code(code) => CallbackEnd::Code(code),
            EndKind::Denied => CallbackEnd::Denied,
            EndKind::GoogleError => CallbackEnd::GoogleError,
        }
    }
}

/// Judges a request head (3.3).
#[must_use]
pub fn judge(head: &[u8], port: u16, state: &Secret) -> Verdict {
    let bad = Verdict::BadRequest;
    let Ok(text) = std::str::from_utf8(head) else {
        return bad;
    };
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return bad;
    };
    if !version.starts_with("HTTP/1.") {
        return bad;
    }
    let mut hosts = 0usize;
    let mut host_ok = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return bad;
        };
        if name.eq_ignore_ascii_case("host") {
            hosts += 1;
            host_ok = value.trim() == format!("127.0.0.1:{port}");
        }
    }
    if method != "GET" || hosts != 1 || !host_ok {
        return bad;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return Verdict::NotFound;
    }
    let mut params = Params::read(query);
    let right_state = params.state.len() == 1
        && params.state[0]
            .as_ref()
            .is_some_and(|value| state.ct_eq(value.expose()));
    if !right_state {
        return bad;
    }
    let iss_ok = match params.iss.as_slice() {
        [] => true,
        [one] => *one,
        _ => false,
    };
    if !iss_ok {
        return Verdict::Ends(EndKind::GoogleError);
    }
    Verdict::Ends(
        match (params.code.as_mut_slice(), params.error.as_slice()) {
            ([code], []) => match code.take() {
                Some(code) if !code.is_empty() => EndKind::Code(code),
                _ => EndKind::GoogleError,
            },
            ([], [denied]) if *denied => EndKind::Denied,
            _ => EndKind::GoogleError,
        },
    )
}

/// The parameters the listener reads; every other one is dropped while reading (3.3).
struct Params {
    /// Each `state`, `None` when its value did not decode.
    state: Vec<Option<Secret>>,
    /// Each `code`, `None` when its value did not decode.
    code: Vec<Option<Secret>>,
    /// Each `error`: whether it is `access_denied`.
    error: Vec<bool>,
    /// Each `iss`: whether it is Google's issuer.
    iss: Vec<bool>,
}

impl Params {
    fn read(query: &str) -> Params {
        let mut params = Params {
            state: Vec::new(),
            code: Vec::new(),
            error: Vec::new(),
            iss: Vec::new(),
        };
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            // A name that does not decode cannot be one of the four.
            let Some(name) = decode(name) else {
                continue;
            };
            match name.as_str() {
                "state" => params.state.push(decode(value).map(Secret::new)),
                "code" => params.code.push(decode(value).map(Secret::new)),
                "error" => params
                    .error
                    .push(decode(value).is_some_and(|v| v == "access_denied")),
                "iss" => params.iss.push(decode(value).is_some_and(|v| v == ISSUER)),
                _ => {}
            }
        }
        params
    }
}

/// Percent-decodes a query name or value, `+` as a space. `None` for a broken escape or text
/// that is not UTF-8.
fn decode(text: &str) -> Option<String> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len()));
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let high = hex(*bytes.get(i + 1)?)?;
                let low = hex(*bytes.get(i + 2)?)?;
                out.push((high << 4) | low);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out.to_vec()).ok()
}

/// Serves one connection: reads the head within [`HEAD_TIME`] and [`MAX_HEAD`], judges it,
/// records the end when it is the first with the right state, and answers a fixed page.
fn serve(mut stream: TcpStream, port: u16, state: &Secret, result: &Mutex<Option<CallbackEnd>>) {
    // An accepted socket may inherit the listener's non-blocking mode on some systems.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let head = match read_head(&mut stream) {
        HeadRead::Head(head) => head,
        HeadRead::TooLarge => {
            answer(&mut stream, 400);
            return;
        }
        HeadRead::Nothing => {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    };
    let status = match judge(&head, port, state) {
        Verdict::BadRequest => 400,
        Verdict::NotFound => 404,
        Verdict::Ends(end) => {
            let mut slot = lock(result);
            if slot.is_some() {
                // Another request with the right state came first; this one changes nothing.
                400
            } else {
                *slot = Some(end.into());
                200
            }
        }
    };
    answer(&mut stream, status);
}

enum HeadRead {
    Head(Zeroizing<Vec<u8>>),
    TooLarge,
    Nothing,
}

/// Reads up to the blank line that ends the head, within [`HEAD_TIME`] and [`MAX_HEAD`].
fn read_head(stream: &mut TcpStream) -> HeadRead {
    let deadline = Instant::now() + HEAD_TIME;
    let mut head = Zeroizing::new(Vec::with_capacity(1024));
    let mut chunk = Zeroizing::new([0u8; 1024]);
    loop {
        if let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            if end + 4 > MAX_HEAD {
                return HeadRead::TooLarge;
            }
            head.truncate(end);
            return HeadRead::Head(head);
        }
        if head.len() > MAX_HEAD {
            return HeadRead::TooLarge;
        }
        let now = Instant::now();
        if now >= deadline {
            return HeadRead::Nothing;
        }
        if stream.set_read_timeout(Some(deadline - now)).is_err() {
            return HeadRead::Nothing;
        }
        match stream.read(chunk.as_mut()) {
            Ok(0) => return HeadRead::Nothing,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::Interrupted) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return HeadRead::Nothing,
        }
    }
}

/// The bytes of the fixed answer for `status` (200, 400 or 404).
#[must_use]
pub fn answer_bytes(status: u16) -> Vec<u8> {
    let (reason, text) = match status {
        200 => ("OK", PAGE_TEXT),
        404 => ("Not Found", ERROR_TEXT),
        _ => ("Bad Request", ERROR_TEXT),
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Fabric Calendar</title></head>\
         <body><p>{text}</p></body></html>"
    );
    let status = if matches!(status, 200 | 404) {
        status
    } else {
        400
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nCache-Control: no-store\r\n\
         Content-Type: text/html; charset=utf-8\r\nConnection: close\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Writes the fixed answer within [`HEAD_TIME`], shuts down the write side, reads and drops
/// what the browser still sends (at most [`DRAIN_MAX`] bytes within [`DRAIN_TIME`]), and
/// closes the connection when the stream is dropped. Closing at once with unread bytes in the
/// receive buffer makes some systems reset the connection, and the browser may lose the page.
fn answer(stream: &mut TcpStream, status: u16) {
    if stream.set_write_timeout(Some(HEAD_TIME)).is_ok() {
        let _ = stream.write_all(&answer_bytes(status));
        let _ = stream.flush();
    }
    if stream.shutdown(Shutdown::Write).is_err() {
        return;
    }
    drain(stream);
}

/// Reads and drops at most [`DRAIN_MAX`] bytes within [`DRAIN_TIME`], until the peer closes.
fn drain(stream: &mut TcpStream) {
    let deadline = Instant::now() + DRAIN_TIME;
    let mut chunk = Zeroizing::new([0u8; 1024]);
    let mut read = 0usize;
    while read < DRAIN_MAX {
        let now = Instant::now();
        if now >= deadline || stream.set_read_timeout(Some(deadline - now)).is_err() {
            return;
        }
        match stream.read(chunk.as_mut()) {
            Ok(0) => return,
            Ok(n) => read += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 51_234;

    fn state() -> Secret {
        Secret::from("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ")
    }

    fn head(target: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nAccept: text/html").into_bytes()
    }

    /// A verdict without the code, to compare.
    #[derive(Debug, PartialEq, Eq)]
    enum V {
        BadRequest,
        NotFound,
        Code,
        Denied,
        GoogleError,
    }

    fn kind(verdict: Verdict) -> V {
        match verdict {
            Verdict::BadRequest => V::BadRequest,
            Verdict::NotFound => V::NotFound,
            Verdict::Ends(EndKind::Code(_)) => V::Code,
            Verdict::Ends(EndKind::Denied) => V::Denied,
            Verdict::Ends(EndKind::GoogleError) => V::GoogleError,
        }
    }

    fn verdict(head: &[u8]) -> V {
        kind(judge(head, PORT, &state()))
    }

    const S: &str = "state=abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
    const ISS: &str = "iss=https%3A%2F%2Faccounts.google.com";

    #[test]
    fn the_right_state_and_one_code_ends_with_the_code() {
        let v = judge(
            &head(&format!(
                "/?{S}&{ISS}&code=4%2F0abc&scope=x&authuser=0&prompt=consent"
            )),
            PORT,
            &state(),
        );
        let Verdict::Ends(EndKind::Code(code)) = v else {
            panic!("the verdict ends with a code");
        };
        assert_eq!(code.expose(), "4/0abc");
        assert_eq!(
            format!("{:?}", Verdict::Ends(EndKind::Code(code))),
            "Ends(Code([redacted]))"
        );
        // Without `iss` too.
        assert_eq!(verdict(&head(&format!("/?code=c&{S}"))), V::Code);
    }

    #[test]
    fn a_wrong_missing_doubled_or_broken_state_is_400() {
        for target in [
            "/?code=c".to_owned(),
            "/".to_owned(),
            "/?state=abc&code=c".to_owned(),
            format!("/?{S}&{S}&code=c"),
            format!("/?{S}x&code=c"),
            "/?state=%zz&code=c".to_owned(),
            "/?STATE=abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ&code=c".to_owned(),
        ] {
            assert_eq!(verdict(&head(&target)), V::BadRequest, "{target}");
        }
        // An encoded name still is the name.
        assert_eq!(
            verdict(&head(
                "/?st%61te=abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ&code=c"
            )),
            V::Code
        );
    }

    #[test]
    fn code_and_error_must_be_exactly_one_of_them() {
        assert_eq!(
            verdict(&head(&format!("/?{S}&error=access_denied&{ISS}"))),
            V::Denied
        );
        for target in [
            format!("/?{S}&error=server_error"),
            format!("/?{S}&error=access_denied&code=c"),
            format!("/?{S}"),
            format!("/?{S}&code=a&code=b"),
            format!("/?{S}&error=access_denied&error=access_denied"),
            format!("/?{S}&code="),
            format!("/?{S}&code=%zz"),
        ] {
            assert_eq!(verdict(&head(&target)), V::GoogleError, "{target}");
        }
    }

    #[test]
    fn an_iss_other_than_google_is_a_google_error() {
        for iss in [
            "iss=https%3A%2F%2Fevil.example",
            "iss=accounts.google.com",
            "iss=",
        ] {
            assert_eq!(
                verdict(&head(&format!("/?{S}&{iss}&code=c"))),
                V::GoogleError,
                "{iss}"
            );
        }
        assert_eq!(
            verdict(&head(&format!("/?{S}&{ISS}&{ISS}&code=c"))),
            V::GoogleError
        );
    }

    #[test]
    fn other_methods_hosts_and_paths_are_refused() {
        let with = |method: &str, target: &str, host: &str| {
            format!("{method} {target} HTTP/1.1\r\nHost: {host}").into_bytes()
        };
        let good = format!("/?{S}&code=c");
        let host = format!("127.0.0.1:{PORT}");
        assert_eq!(verdict(&with("POST", &good, &host)), V::BadRequest);
        assert_eq!(verdict(&with("get", &good, &host)), V::BadRequest);
        assert_eq!(
            verdict(&with("GET", &good, "localhost:51234")),
            V::BadRequest
        );
        assert_eq!(verdict(&with("GET", &good, "127.0.0.1:1")), V::BadRequest);
        assert_eq!(verdict(&with("GET", &good, "127.0.0.1")), V::BadRequest);
        assert_eq!(
            verdict(&format!("GET {good} HTTP/1.1\r\nHost: {host}\r\nHost: {host}").into_bytes()),
            V::BadRequest
        );
        assert_eq!(
            verdict(&format!("GET {good} HTTP/1.1").into_bytes()),
            V::BadRequest
        );
        assert_eq!(verdict(&with("GET", "/favicon.ico", &host)), V::NotFound);
        assert_eq!(
            verdict(&with("GET", &format!("/callback?{S}&code=c"), &host)),
            V::NotFound
        );
        assert_eq!(
            verdict(&with(
                "GET",
                &format!("http://127.0.0.1:{PORT}/?{S}&code=c"),
                &host
            )),
            V::NotFound
        );
        assert_eq!(
            verdict(&format!("GET {good} SPDY/3\r\nHost: {host}").into_bytes()),
            V::BadRequest
        );
        assert_eq!(verdict(b"\xff\xfe"), V::BadRequest);
    }

    #[test]
    fn the_answers_are_fixed_pages_with_the_three_headers() {
        for status in [200, 400, 404] {
            let text = String::from_utf8(answer_bytes(status)).expect("text");
            assert!(text.starts_with(&format!("HTTP/1.1 {status} ")));
            assert!(text.contains("\r\nCache-Control: no-store\r\n"));
            assert!(text.contains("\r\nContent-Type: text/html; charset=utf-8\r\n"));
            assert!(text.contains("\r\nConnection: close\r\n"));
        }
        let ok = String::from_utf8(answer_bytes(200)).expect("text");
        assert!(ok.contains(PAGE_TEXT));
        assert!(!ok.contains(ERROR_TEXT));
    }

    #[test]
    fn the_listener_ends_after_300_s_on_the_monotonic_clock_when_the_crate_clock_stands_still() {
        /// A wall clock that was set back: it shows no time passing.
        struct Still;
        impl Clock for Still {
            fn now_ms(&self) -> u64 {
                5_000
            }
        }
        // A system that has run for less than 300 s cannot give such an instant.
        let Some(long_ago) = Instant::now().checked_sub(LIFETIME) else {
            return;
        };
        let listener = Listener::bind().expect("a listener");
        let end = listener.run(&state(), &Still, 5_000, long_ago, &AtomicBool::new(false));
        assert!(matches!(end, CallbackEnd::TimedOut), "{end:?}");
    }

    #[test]
    fn the_page_arrives_whole_when_the_browser_sent_more_than_the_head() {
        let server = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = server.local_addr().expect("an address").port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connected");
        let (stream, _) = server.accept().expect("accepted");
        let serving = std::thread::spawn(move || {
            let result = Mutex::new(None);
            serve(stream, port, &state(), &result);
            lock(&result).is_none()
        });
        // A head with a wrong state, and bytes after it that the listener never reads.
        let mut request =
            format!("GET /?state=wrong HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").into_bytes();
        request.extend(std::iter::repeat_n(b'x', 3_000));
        client.write_all(&request).expect("sent");
        // A guard against a hang, not a timing assumption.
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a guard");
        let mut answer = Vec::new();
        client
            .read_to_end(&mut answer)
            .expect("the whole answer and a clean end, no reset");
        assert_eq!(answer, answer_bytes(400));
        drop(client);
        assert!(
            serving.join().expect("served"),
            "a wrong state ends nothing"
        );
    }

    #[test]
    fn decoding_reads_percent_and_plus_and_refuses_broken_escapes() {
        assert_eq!(decode("a%2Fb+c").as_deref(), Some("a/b c"));
        assert_eq!(decode("%").as_deref(), None);
        assert_eq!(decode("%4").as_deref(), None);
        assert_eq!(decode("%ff").as_deref(), None);
        assert_eq!(decode("%C3%A9").as_deref(), Some("\u{e9}"));
    }
}
