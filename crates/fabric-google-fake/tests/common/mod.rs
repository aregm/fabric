//! Helpers of the fake's own tests (f01): a raw HTTP client written here on `std::net`, so that
//! the fake is tested without the adapter and without its own client, a stand-in loopback
//! listener, and a fake set up with one client and two accounts.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fabric_google_fake::{
    ASKED_SCOPES, Account, CalendarEntry, Client, FakeGoogle, SCOPE_CALENDAR_LIST, SCOPE_EVENTS,
    SCOPE_FREEBUSY,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const CLIENT_ID: &str = "fake-client.apps.googleusercontent.com";
pub const CLIENT_SECRET: &str = "fake-client-secret";
pub const ALICE: &str = "100000000000000000001";
pub const ALICE_EMAIL: &str = "alice@example.com";
pub const BOB: &str = "100000000000000000002";
pub const BOB_EMAIL: &str = "bob@example.com";
/// A calendar Bob shared with Alice for reading.
pub const SHARED: &str = "team#ops@group.calendar.google.com";
/// A verifier of 43 characters.
pub const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

/// The upper bound on any wait in these tests.
pub const BOUND: Duration = Duration::from_secs(10);

/// The calendar scopes, for an access token made without a sign-in.
pub const CALENDAR_SCOPES: [&str; 3] = [SCOPE_CALENDAR_LIST, SCOPE_FREEBUSY, SCOPE_EVENTS];

/// A fake with the client, Alice (primary calendar, owner) and Bob (primary calendar; the
/// shared calendar as owner), Alice reading the shared calendar.
pub fn fake() -> FakeGoogle {
    let fake = FakeGoogle::start();
    fake.add_client(Client::new(CLIENT_ID, CLIENT_SECRET));
    fake.add_account(Account::new(ALICE, ALICE_EMAIL));
    fake.add_account(Account::new(BOB, BOB_EMAIL));
    fake.add_calendar(
        ALICE,
        CalendarEntry::new(ALICE_EMAIL, "owner")
            .primary()
            .time_zone("Europe/Berlin"),
    );
    fake.add_calendar(ALICE, CalendarEntry::new(SHARED, "reader"));
    fake.add_calendar(BOB, CalendarEntry::new(BOB_EMAIL, "owner").primary());
    fake.add_calendar(BOB, CalendarEntry::new(SHARED, "owner"));
    fake
}

/// An access token of `subject` with the three calendar scopes, for an hour.
pub fn token(fake: &FakeGoogle, subject: &str) -> String {
    fake.issue_access_token(subject, CLIENT_ID, &CALENDAR_SCOPES, 3600)
}

/// The S256 challenge of a verifier.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Encodes a path segment or query value (everything but `A-Z a-z 0-9 - . _ ~`).
pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// The authorization address the adapter would build (3.2.1), with `redirect` as the
/// loopback redirect.
pub fn authorization_address(
    fake: &FakeGoogle,
    redirect: &str,
    state: &str,
    nonce: &str,
) -> String {
    let scope = ASKED_SCOPES.join(" ");
    format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&nonce={}&prompt=select_account",
        fake.authorization_url(),
        enc(CLIENT_ID),
        enc(redirect),
        enc(&scope),
        challenge(VERIFIER),
        enc(state),
        enc(nonce),
    )
}

/// An answer read by the raw client.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// False when the connection closed before a complete answer.
    pub complete: bool,
}

impl Answer {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    pub fn reason(&self) -> Option<String> {
        self.json()["error"]["errors"][0]["reason"]
            .as_str()
            .map(str::to_string)
    }
}

/// Sends raw bytes to `127.0.0.1:port` and reads until the connection closes.
pub fn send_bytes(port: u16, bytes: &[u8]) -> Answer {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, BOUND).expect("connect");
    stream.set_read_timeout(Some(BOUND)).unwrap();
    stream.write_all(bytes).unwrap();
    let mut received = Vec::new();
    let _ = stream.read_to_end(&mut received);
    parse_answer(&received)
}

fn parse_answer(received: &[u8]) -> Answer {
    let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Answer {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
            complete: false,
        };
    };
    let head = String::from_utf8_lossy(&received[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    let body = received[end + 4..].to_vec();
    let length = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok());
    let complete = length.is_some_and(|l| body.len() == l);
    Answer {
        status,
        headers,
        body,
        complete,
    }
}

/// Sends a request with a `Content-Length` body (when there is one) and `Connection: close`.
pub fn request(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Answer {
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if !body.is_empty() || method != "GET" && method != "DELETE" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("Accept-Encoding: identity\r\nConnection: close\r\n\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    send_bytes(port, &bytes)
}

/// A Calendar API request with a bearer token.
pub fn api(
    fake: &FakeGoogle,
    token: &str,
    method: &str,
    target: &str,
    extra: &[(&str, &str)],
    body: Option<&Value>,
) -> Answer {
    let bearer = format!("Bearer {token}");
    let mut headers = vec![("Authorization", bearer.as_str())];
    headers.extend_from_slice(extra);
    let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    if body.is_some() {
        headers.push(("Content-Type", "application/json"));
    }
    request(
        fake.port(),
        method,
        &format!("/calendar/v3{target}"),
        &headers,
        &bytes,
    )
}

/// A form POST.
pub fn form(fake: &FakeGoogle, path: &str, pairs: &[(&str, &str)]) -> Answer {
    let body = pairs
        .iter()
        .map(|(n, v)| format!("{}={}", enc(n), enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    request(
        fake.port(),
        "POST",
        path,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.as_bytes(),
    )
}

/// The events path of a calendar.
pub fn events_path(calendar: &str) -> String {
    format!("/calendars/{}/events", enc(calendar))
}

/// The path of one event.
pub fn event_path(calendar: &str, id: &str) -> String {
    format!("/calendars/{}/events/{}", enc(calendar), enc(id))
}

/// A stand-in for the adapter's loopback listener: answers every request 200 with a fixed
/// page and hands each request's head to the test.
pub struct Listener {
    pub port: u16,
    pub requests: Receiver<String>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start() -> Listener {
        let listener =
            TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(BOUND)).unwrap();
                        let mut head = Vec::new();
                        let mut buf = [0u8; 1024];
                        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                            match stream.read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => head.extend_from_slice(&buf[..n]),
                            }
                        }
                        let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
                        let page = "listener page";
                        let _ = stream.write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                                page.len()
                            )
                            .as_bytes(),
                        );
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Listener {
            port,
            requests: rx,
            stop,
            thread: Some(thread),
        }
    }

    pub fn redirect(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The head of the next request it received, within the bound.
    pub fn next(&self) -> String {
        self.requests.recv_timeout(BOUND).expect("a callback")
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Waits until `done` holds, at most [`BOUND`].
pub fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + BOUND;
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    done()
}
