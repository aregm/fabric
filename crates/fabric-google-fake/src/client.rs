//! A small HTTP/1.1 client for `127.0.0.1` only: what the browser of [`crate::browser`] uses,
//! and what a test uses to send any request it likes to the adapter's loopback listener (g07,
//! g09). One request per connection; the answer is read until its `Content-Length` or the end of
//! the connection.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::url::HttpAddress;

/// How long a request of this client may take by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The most this client reads of an answer.
const MAX_ANSWER: usize = 8 * 1024 * 1024;

/// An answer as received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The headers, in order, names as sent.
    pub headers: Vec<(String, String)>,
    /// The body, as received (not decompressed).
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The first header of this name, without regard to case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body as text (invalid UTF-8 replaced).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON, when it is JSON.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// Reads a complete answer: status line, headers, and the body up to `Content-Length` (or
    /// all that follows when there is none). `None` when it is not an HTTP/1.x answer.
    pub fn parse(bytes: &[u8]) -> Option<HttpResponse> {
        let end = find_head_end(bytes)?;
        let head = std::str::from_utf8(&bytes[..end]).ok()?;
        let mut lines = head.split("\r\n");
        let status_line = lines.next()?;
        let mut parts = status_line.splitn(3, ' ');
        let version = parts.next()?;
        if !version.starts_with("HTTP/1.") {
            return None;
        }
        let status: u16 = parts.next()?.parse().ok()?;
        let mut headers = Vec::new();
        for line in lines {
            let (name, value) = line.split_once(':')?;
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
        let mut body = bytes[end + 4..].to_vec();
        let response = HttpResponse {
            status,
            headers,
            body: Vec::new(),
        };
        if let Some(length) = response
            .header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
        {
            body.truncate(length);
        }
        Some(HttpResponse { body, ..response })
    }
}

fn find_head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Why a request of this client failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientError {
    /// The address is not `http://127.0.0.1:<port>` (or `localhost`); nothing was sent.
    NotLoopback,
    /// Nothing listens there (connection refused); nothing was sent.
    Refused,
    /// The connection closed or timed out before a complete answer.
    NoAnswer,
    /// The bytes that came back are not an HTTP answer.
    Malformed,
    /// Another input or output error.
    Io(io::ErrorKind),
}

/// The loopback socket address for `port`.
pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
}

/// Opens a connection to `127.0.0.1:port` and sends nothing, as a browser's spare connection
/// does (g09). The connection stays open until the value is dropped.
pub fn connect_idle(port: u16) -> io::Result<TcpStream> {
    TcpStream::connect_timeout(&loopback(port), DEFAULT_TIMEOUT)
}

/// Sends `bytes` as they are to `127.0.0.1:port` and reads the answer until the end of the
/// connection, its `Content-Length`, or `timeout`.
pub fn send_raw(port: u16, bytes: &[u8], timeout: Duration) -> Result<HttpResponse, ClientError> {
    let deadline = Instant::now() + timeout;
    let mut stream = match TcpStream::connect_timeout(&loopback(port), timeout) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => return Err(ClientError::Refused),
        Err(e) => return Err(ClientError::Io(e.kind())),
    };
    stream
        .write_all(bytes)
        .and_then(|()| stream.flush())
        .map_err(|e| ClientError::Io(e.kind()))?;
    let mut received = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let _ = stream.set_read_timeout(Some((deadline - now).min(Duration::from_millis(200))));
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                received.extend_from_slice(&buffer[..n]);
                if received.len() > MAX_ANSWER {
                    break;
                }
                if answer_complete(&received) {
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    if received.is_empty() {
        return Err(ClientError::NoAnswer);
    }
    let Some(end) = find_head_end(&received) else {
        return Err(ClientError::NoAnswer);
    };
    let response = HttpResponse::parse(&received).ok_or(ClientError::Malformed)?;
    if let Some(length) = response
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        && received.len() < end + 4 + length
    {
        return Err(ClientError::NoAnswer);
    }
    Ok(response)
}

fn answer_complete(received: &[u8]) -> bool {
    let Some(end) = find_head_end(received) else {
        return false;
    };
    let Some(head) = HttpResponse::parse(&received[..end + 4]) else {
        return false;
    };
    match head
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        Some(length) => received.len() >= end + 4 + length,
        None => false,
    }
}

/// A request to send to `127.0.0.1`, built field by field so that a test can send what a
/// browser would not (another method, another `Host`, a doubled parameter).
#[derive(Clone, Debug)]
pub struct RawRequest {
    method: String,
    port: u16,
    target: String,
    host: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RawRequest {
    /// A request for an `http://127.0.0.1:<port>/...` (or `localhost`) address, with the
    /// `Host` a browser would send. `Err(NotLoopback)` for any other address.
    pub fn new(method: &str, address: &str) -> Result<RawRequest, ClientError> {
        let parsed = HttpAddress::parse(address).ok_or(ClientError::NotLoopback)?;
        if parsed.host != "127.0.0.1" && parsed.host != "localhost" {
            return Err(ClientError::NotLoopback);
        }
        Ok(RawRequest {
            method: method.to_string(),
            port: parsed.port,
            target: parsed.target(),
            host: format!("{}:{}", parsed.host, parsed.port),
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    /// A GET request for `address`.
    pub fn get(address: &str) -> Result<RawRequest, ClientError> {
        RawRequest::new("GET", address)
    }

    /// Replaces the method.
    pub fn method(mut self, method: &str) -> RawRequest {
        self.method = method.to_string();
        self
    }

    /// Replaces the request target (path and query) as sent on the request line.
    pub fn target(mut self, target: &str) -> RawRequest {
        self.target = target.to_string();
        self
    }

    /// Replaces the `Host` header (the connection still goes to `127.0.0.1`).
    pub fn host(mut self, host: &str) -> RawRequest {
        self.host = host.to_string();
        self
    }

    /// Adds a header.
    pub fn header(mut self, name: &str, value: &str) -> RawRequest {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Sets a body; `Content-Length` is written for it.
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> RawRequest {
        self.body = body.into();
        self
    }

    /// The port the request goes to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The request target as it will be sent.
    pub fn request_target(&self) -> &str {
        &self.target
    }

    /// The bytes this request sends.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\n",
            self.method, self.target, self.host
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if !self.body.is_empty() || matches!(self.method.as_str(), "POST" | "PUT" | "PATCH") {
            head.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        head.push_str("Connection: close\r\n\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }

    /// Sends the request on a connection of its own and reads the answer.
    pub fn send(&self) -> Result<HttpResponse, ClientError> {
        send_raw(self.port, &self.to_bytes(), DEFAULT_TIMEOUT)
    }
}
