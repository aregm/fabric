//! Test helpers shared by the integration tests: a scripted HTTP peer on `127.0.0.1` for the
//! transport rules of 5.2 (without the fake Google server), and a scripted token store (g17).
//!
//! Every helper is deterministic: a peer acts on what it reads, and waits only for the client
//! to close, never for a fixed time. Each read has a 10 s guard so a broken test fails instead
//! of hanging.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use fabric_desktop_contracts::scalar::Id;
use fabric_google_calendar::{Secret, TokenStore, TokenStoreError};

/// How long a peer waits on one read before it gives up (a guard, not a timing assumption).
const GUARD: Duration = Duration::from_secs(10);

/// What the peer does on one connection.
#[derive(Clone, Debug)]
pub enum Reply {
    /// Reads the request, writes these bytes, then keeps reading the connection until the
    /// client closes it, recording every further request (connection reuse).
    Answer(Vec<u8>),
    /// Reads the request, writes these bytes, and closes at once.
    AnswerAndClose(Vec<u8>),
    /// Closes at once without reading anything.
    AcceptAndClose,
    /// Reads the request and closes without answering.
    ReadAndClose,
    /// Reads the request, then waits until the client closes (a stall past any deadline).
    Stall,
}

/// One request as the peer read it.
#[derive(Clone, Debug, Default)]
pub struct Captured {
    /// The request line and headers, as text.
    pub head: String,
    /// The body (by `Content-Length`).
    pub body: Vec<u8>,
}

impl Captured {
    /// The request line.
    pub fn request_line(&self) -> &str {
        self.head.lines().next().unwrap_or("")
    }

    /// Header names in lower case, in order.
    pub fn header_names(&self) -> Vec<String> {
        self.head
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .map(|(n, _)| n.trim().to_ascii_lowercase())
            .collect()
    }

    /// The value of a header (first occurrence), compared without regard to case.
    pub fn header(&self, name: &str) -> Option<String> {
        self.head
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
    }
}

/// What happened on one connection.
#[derive(Clone, Debug, Default)]
pub struct Connection {
    /// Every request read on it, in order.
    pub requests: Vec<Captured>,
}

/// A peer that serves one scripted reply per accepted connection, in order, then stops.
pub struct ScriptedPeer {
    port: u16,
    handle: Option<JoinHandle<Vec<Connection>>>,
}

/// Reads one request (head and `Content-Length` body). `None` at a clean end of stream.
fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Captured> {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        head.push_str(&line);
        if head.len() > 64 * 1024 {
            return None;
        }
    }
    let mut captured = Captured {
        head,
        body: Vec::new(),
    };
    let length: usize = captured
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    captured.body = body;
    Some(captured)
}

impl ScriptedPeer {
    /// Binds `127.0.0.1` on a port the system picks and serves `replies`, one per connection.
    pub fn start(replies: Vec<Reply>) -> ScriptedPeer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("an address").port();
        let handle = std::thread::spawn(move || {
            let mut connections = Vec::new();
            for reply in replies {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                stream.set_read_timeout(Some(GUARD)).expect("a read guard");
                let mut writer = stream.try_clone().expect("a second handle");
                let mut reader = BufReader::new(stream);
                let mut connection = Connection::default();
                match reply {
                    Reply::AcceptAndClose => {}
                    Reply::ReadAndClose => {
                        connection.requests.extend(read_request(&mut reader));
                    }
                    Reply::Stall => {
                        connection.requests.extend(read_request(&mut reader));
                        let mut rest = Vec::new();
                        let _ = reader.read_to_end(&mut rest);
                    }
                    Reply::AnswerAndClose(bytes) => {
                        connection.requests.extend(read_request(&mut reader));
                        let _ = writer.write_all(&bytes);
                        let _ = writer.flush();
                    }
                    Reply::Answer(bytes) => {
                        if let Some(first) = read_request(&mut reader) {
                            connection.requests.push(first);
                            let _ = writer.write_all(&bytes);
                            let _ = writer.flush();
                            while let Some(next) = read_request(&mut reader) {
                                connection.requests.push(next);
                                let _ = writer.write_all(&bytes);
                            }
                        }
                    }
                }
                let _ = writer.shutdown(std::net::Shutdown::Both);
                connections.push(connection);
            }
            connections
        });
        ScriptedPeer {
            port,
            handle: Some(handle),
        }
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Waits until every scripted connection has been served and returns what it saw. Call it
    /// only when the test made one connection per reply.
    pub fn finish(mut self) -> Vec<Connection> {
        self.handle
            .take()
            .expect("not finished yet")
            .join()
            .expect("the peer thread ends")
    }
}

/// A listener that answers nothing and records whether anyone connected. Used as a redirect
/// target and as a proxy that must never be used.
pub struct Witness {
    listener: TcpListener,
}

impl Witness {
    /// Binds `127.0.0.1` on a port the system picks.
    pub fn start() -> Witness {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        listener.set_nonblocking(true).expect("non-blocking");
        Witness { listener }
    }

    /// The port.
    pub fn port(&self) -> u16 {
        self.listener.local_addr().expect("an address").port()
    }

    /// Whether a connection is waiting. A client connect completes before the client's call
    /// returns, so after the call this is exact. A connection that the client already reset
    /// may come back as an error other than `WouldBlock` (seen on some systems); that still
    /// means someone connected.
    pub fn was_contacted(&self) -> bool {
        match self.listener.accept() {
            Ok(_) => true,
            Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
        }
    }
}

/// A port on which nothing listens: bound, then released.
pub fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    listener.local_addr().expect("an address").port()
}

/// An HTTP/1.1 answer with `Content-Length`.
pub fn answer(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// An HTTP/1.1 answer whose body ends when the connection closes (no length).
pub fn answer_until_close(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// A token store whose calls fail with a chosen error, or pass to an inner memory map (g17).
/// Clones share the script and the map.
#[derive(Clone, Default)]
pub struct ScriptedTokenStore {
    failure: Arc<Mutex<Option<TokenStoreError>>>,
    tokens: Arc<Mutex<std::collections::HashMap<Id, String>>>,
    calls: Arc<Mutex<usize>>,
}

impl ScriptedTokenStore {
    /// A store that works until told to fail.
    pub fn new() -> ScriptedTokenStore {
        ScriptedTokenStore::default()
    }

    /// Every later call fails with `error`, or works again with `None`.
    pub fn fail_with(&self, error: Option<TokenStoreError>) {
        *self.failure.lock().expect("unpoisoned") = error;
    }

    /// How many calls were made.
    pub fn calls(&self) -> usize {
        *self.calls.lock().expect("unpoisoned")
    }

    fn check(&self) -> Result<(), TokenStoreError> {
        *self.calls.lock().expect("unpoisoned") += 1;
        match *self.failure.lock().expect("unpoisoned") {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl TokenStore for ScriptedTokenStore {
    fn read(&self, account: &Id) -> Result<Option<Secret>, TokenStoreError> {
        self.check()?;
        Ok(self
            .tokens
            .lock()
            .expect("unpoisoned")
            .get(account)
            .map(|t| Secret::from(t.as_str())))
    }

    fn write(&mut self, account: &Id, refresh_token: &Secret) -> Result<(), TokenStoreError> {
        self.check()?;
        self.tokens
            .lock()
            .expect("unpoisoned")
            .insert(account.clone(), refresh_token.expose().to_owned());
        Ok(())
    }

    fn delete(&mut self, account: &Id) -> Result<(), TokenStoreError> {
        self.check()?;
        self.tokens.lock().expect("unpoisoned").remove(account);
        Ok(())
    }
}
