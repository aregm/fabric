//! The MCP test client: a second reading of the MCP specification, written from its text and
//! not from the bridge's code (design 10.2, R10).
//!
//! Sources: MCP 2026-07-28 "Overview" (messages, `_meta`), "Versioning", "stdio",
//! "Discovery", "Tools", "Cancellation", "Subscriptions"; MCP 2025-11-25 "Lifecycle" and
//! "Tools".
//!
//! The client starts the bridge as a child with three pipes, reads each pipe on its own
//! thread, waits on every read with a time limit, and kills and reaps the child on drop. Every
//! stdout line must be exactly one JSON-RPC 2.0 message that a server may send: a response or
//! a notification, never a request ("The server MUST NOT write JSON-RPC requests to stdout";
//! "The server MUST NOT write anything to its stdout that is not a valid MCP message").

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::scratch::Scratch;

/// The modern revision (MCP "Versioning": "The current protocol version is 2026-07-28").
pub const MODERN: &str = "2026-07-28";
/// The last legacy revision.
pub const LEGACY: &str = "2025-11-25";
/// The longest wait for anything that is due promptly.
pub const PROMPT: Duration = Duration::from_secs(10);

/// One stdout event from the reader thread.
enum Out {
    Line(Vec<u8>),
    /// Bytes at the end of the stream with no line feed after them.
    Unterminated(Vec<u8>),
    End,
}

/// A running bridge and the client's view of it.
pub struct Bridge {
    child: Child,
    stdin: Option<ChildStdin>,
    out: Receiver<Out>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Messages read but not yet taken by a wait.
    backlog: VecDeque<Value>,
    /// Every message read from stdout, in order.
    seen: Vec<Value>,
    ended: bool,
}

pub fn bridge_exe() -> &'static str {
    env!("CARGO_BIN_EXE_fabric-mcp-bridge")
}

/// The `_meta` every modern request carries (MCP "Overview", per-request protocol fields:
/// `protocolVersion` and `clientCapabilities` are required, `clientInfo` should be sent).
pub fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "fabric-mcp-bridge-tests", "version": "0"},
    })
}

/// Checks one stdout line against JSON-RPC 2.0 and MCP's rules for what a server writes, and
/// returns the message.
fn check_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes)
        .unwrap_or_else(|e| panic!("a stdout line is not UTF-8: {e}: {bytes:?}"));
    assert!(
        !text.contains('\n'),
        "a stdout line holds a line feed: {text:?}"
    );
    let value: Value = serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("a stdout line is not one JSON value ({e}): {text:?}"));
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("a stdout line is not a JSON object: {text:?}"));
    assert_eq!(
        object.get("jsonrpc"),
        Some(&json!("2.0")),
        "a stdout line is not JSON-RPC 2.0: {text:?}"
    );
    let has_method = object.contains_key("method");
    let has_id = object.contains_key("id");
    if has_method {
        assert!(
            !has_id,
            "the server wrote a request to stdout, which MCP forbids: {text:?}"
        );
        assert!(
            object["method"].is_string(),
            "a notification's method is not a string: {text:?}"
        );
        for key in object.keys() {
            assert!(
                ["jsonrpc", "method", "params"].contains(&key.as_str()),
                "a notification has the member {key:?}: {text:?}"
            );
        }
        if let Some(params) = object.get("params") {
            assert!(params.is_object(), "params is not an object: {text:?}");
        }
    } else {
        let has_result = object.contains_key("result");
        let has_error = object.contains_key("error");
        assert!(
            has_result != has_error,
            "a response must carry exactly one of result and error: {text:?}"
        );
        for key in object.keys() {
            assert!(
                ["jsonrpc", "id", "result", "error"].contains(&key.as_str()),
                "a response has the member {key:?}: {text:?}"
            );
        }
        if let Some(id) = object.get("id") {
            // MCP: an id is a string or an integer, never null.
            assert!(
                id.is_string() || id.is_i64() || id.is_u64(),
                "a response id is not a string or an integer: {text:?}"
            );
        } else {
            assert!(has_error, "a result response has no id: {text:?}");
        }
        if has_result {
            assert!(
                object["result"].is_object(),
                "result is not an object: {text:?}"
            );
        } else {
            let error = object["error"]
                .as_object()
                .unwrap_or_else(|| panic!("error is not an object: {text:?}"));
            assert!(
                error.get("code").is_some_and(|c| c.is_i64()),
                "an error code is not an integer: {text:?}"
            );
            assert!(
                error.get("message").is_some_and(Value::is_string),
                "an error has no message: {text:?}"
            );
        }
    }
    value
}

impl Bridge {
    /// Starts the bridge as `--enrollment <id> --socket <s> --key-dir <k>`.
    pub fn start(scratch: &Scratch, enrollment: &str) -> Bridge {
        Bridge::start_program(Path::new(bridge_exe()), scratch, enrollment)
    }

    pub fn start_program(program: &Path, scratch: &Scratch, enrollment: &str) -> Bridge {
        let args: Vec<OsString> = vec![
            "--enrollment".into(),
            enrollment.into(),
            "--socket".into(),
            scratch.socket.clone().into_os_string(),
            "--key-dir".into(),
            scratch.key_dir.clone().into_os_string(),
        ];
        Bridge::spawn(program, &args)
    }

    pub fn spawn(program: &Path, args: &[OsString]) -> Bridge {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("start {}: {e}", program.display()));
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (tx, out) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(Out::End);
                        return;
                    }
                    Ok(_) => {
                        if line.last() == Some(&b'\n') {
                            line.pop();
                            if tx.send(Out::Line(line)).is_err() {
                                return;
                            }
                        } else {
                            let _ = tx.send(Out::Unterminated(line));
                            let _ = tx.send(Out::End);
                            return;
                        }
                    }
                }
            }
        });
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                let mut chunk = Vec::new();
                match reader.read_until(b'\n', &mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => sink
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(&chunk),
                }
            }
        });
        Bridge {
            child,
            stdin,
            out,
            stderr: collected,
            backlog: VecDeque::new(),
            seen: Vec::new(),
            ended: false,
        }
    }

    // ---- writing --------------------------------------------------------------------------

    /// Writes one line and a line feed to the bridge's stdin.
    pub fn send_raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .expect("write to the bridge's stdin");
    }

    pub fn send(&mut self, message: &Value) {
        let line = serde_json::to_string(message).expect("serialize");
        self.send_raw(&line);
    }

    /// A modern request: `params._meta` carries the per-request protocol fields.
    pub fn send_modern(&mut self, id: &Value, method: &str, params: Value) {
        let mut params = match params {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            other => panic!("params must be an object, not {other}"),
        };
        params.insert("_meta".into(), modern_meta());
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    /// A legacy request: no `_meta`.
    pub fn send_legacy(&mut self, id: &Value, method: &str, params: Value) {
        let mut message = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(&message);
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        let mut message = json!({"jsonrpc": "2.0", "method": method});
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(&message);
    }

    /// Closes stdin: MCP stdio "Shutdown".
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    // ---- reading --------------------------------------------------------------------------

    /// Reads one stdout message, waiting at most until `deadline`.
    fn read_one(&mut self, deadline: Instant) -> Option<Value> {
        if self.ended {
            return None;
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.out.recv_timeout(wait) {
            Ok(Out::Line(bytes)) => {
                let message = check_line(&bytes);
                self.seen.push(message.clone());
                Some(message)
            }
            Ok(Out::Unterminated(bytes)) => panic!(
                "stdout ended inside a line, without a line feed: {:?}",
                String::from_utf8_lossy(&bytes)
            ),
            Ok(Out::End) | Err(RecvTimeoutError::Disconnected) => {
                self.ended = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => None,
        }
    }

    /// Waits for the message that `matches` accepts, keeping others in the backlog.
    pub fn wait_for(
        &mut self,
        what: &str,
        within: Duration,
        matches: impl Fn(&Value) -> bool,
    ) -> Value {
        if let Some(i) = self.backlog.iter().position(&matches) {
            return self.backlog.remove(i).expect("index in range");
        }
        let deadline = Instant::now() + within;
        loop {
            match self.read_one(deadline) {
                Some(m) if matches(&m) => return m,
                Some(m) => self.backlog.push_back(m),
                None => {
                    if self.ended || Instant::now() >= deadline {
                        panic!(
                            "no {what} within {within:?} (stdout ended: {}); seen: {:?}; stderr: {}",
                            self.ended,
                            self.seen,
                            self.stderr_text()
                        );
                    }
                }
            }
        }
    }

    /// Waits for the response with `id`.
    pub fn wait_response(&mut self, id: &Value) -> Value {
        let id = id.clone();
        self.wait_for(&format!("response to {id}"), PROMPT, move |m| {
            m.get("method").is_none() && m.get("id") == Some(&id)
        })
    }

    /// Reads for `period` and returns what arrived (also kept in the backlog).
    pub fn drain_for(&mut self, period: Duration) -> Vec<Value> {
        let deadline = Instant::now() + period;
        let mut got = Vec::new();
        while Instant::now() < deadline {
            match self.read_one(deadline) {
                Some(m) => {
                    got.push(m.clone());
                    self.backlog.push_back(m);
                }
                None if self.ended => break,
                None => {}
            }
        }
        got
    }

    /// Every message read so far.
    pub fn seen(&self) -> &[Value] {
        &self.seen
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned()
    }

    /// Waits for the process to exit.
    pub fn wait_exit(&mut self, within: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    // ---- the methods ----------------------------------------------------------------------

    pub fn modern(&mut self, id: Value, method: &str, params: Value) -> Value {
        self.send_modern(&id, method, params);
        self.wait_response(&id)
    }

    pub fn legacy(&mut self, id: Value, method: &str, params: Value) -> Value {
        self.send_legacy(&id, method, params);
        self.wait_response(&id)
    }

    /// `server/discover` (MCP 2026-07-28 "Discovery").
    pub fn discover(&mut self, id: Value) -> Value {
        self.modern(id, "server/discover", json!({}))
    }

    /// `initialize` and then `notifications/initialized` (MCP 2025-11-25 "Lifecycle").
    pub fn initialize(&mut self, id: Value) -> Value {
        let response = self.legacy(
            id,
            "initialize",
            json!({
                "protocolVersion": LEGACY,
                "capabilities": {},
                "clientInfo": {"name": "fabric-mcp-bridge-tests", "version": "0"},
            }),
        );
        self.notify("notifications/initialized", Value::Null);
        response
    }

    pub fn tools_list_modern(&mut self, id: Value) -> Value {
        self.modern(id, "tools/list", json!({}))
    }

    pub fn tools_list_legacy(&mut self, id: Value) -> Value {
        self.legacy(id, "tools/list", json!({}))
    }

    pub fn send_call_modern(&mut self, id: &Value, tool: &str, arguments: Value) {
        self.send_modern(
            id,
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
    }

    /// A modern `tools/call`; returns the whole response.
    pub fn call_modern(&mut self, id: Value, tool: &str, arguments: Value) -> Value {
        self.send_call_modern(&id, tool, arguments);
        self.wait_response(&id)
    }

    pub fn call_legacy(&mut self, id: Value, tool: &str, arguments: Value) -> Value {
        self.legacy(
            id,
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        )
    }

    /// `notifications/cancelled` for `id` (MCP "Cancellation").
    pub fn cancel(&mut self, id: &Value) {
        self.notify(
            "notifications/cancelled",
            json!({"requestId": id, "reason": "the test cancels it"}),
        );
    }

    /// `subscriptions/listen` (MCP 2026-07-28 "Subscriptions").
    pub fn send_listen(&mut self, id: &Value) {
        self.send_modern(
            id,
            "subscriptions/listen",
            json!({"notifications": {"toolsListChanged": true}}),
        );
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The result of a successful response, or a panic naming what came instead.
pub fn result_of(response: &Value) -> &Map<String, Value> {
    response
        .get("result")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("expected a result: {response}"))
}

/// The error of an error response.
pub fn error_of(response: &Value) -> &Map<String, Value> {
    response
        .get("error")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("expected an error: {response}"))
}

/// Checks the members every modern result carries: `resultType` (MCP "Overview": "The
/// `result` MUST include a `resultType` field") and, per the design 3.3, `_meta` with
/// `io.modelcontextprotocol/serverInfo` (MCP: servers SHOULD include it in every result).
pub fn assert_modern_result(result: &Map<String, Value>) {
    assert_eq!(
        result.get("resultType"),
        Some(&json!("complete")),
        "{result:?}"
    );
    let info = result
        .get("_meta")
        .and_then(|m| m.get("io.modelcontextprotocol/serverInfo"))
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("no _meta serverInfo: {result:?}"));
    assert!(info.get("name").is_some_and(Value::is_string), "{info:?}");
    assert!(
        info.get("version").is_some_and(Value::is_string),
        "{info:?}"
    );
}

/// Checks that a legacy result carries none of the modern members.
pub fn assert_legacy_result(result: &Map<String, Value>) {
    for key in ["resultType", "_meta", "ttlMs", "cacheScope"] {
        assert!(
            !result.contains_key(key),
            "a legacy result carries the modern member {key}: {result:?}"
        );
    }
}
