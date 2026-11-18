//! The stand-in broker: a second reading of design 4.4 (the handshake) and of the contract's
//! agent channel (`contracts/desktop/v1/README.md` sections 1 to 5 and 8), written from those
//! texts and not from the bridge's code.
//!
//! It listens on `<scratch>/run/agent.sock` (mode 0600 in a directory of mode 0700), answers
//! `agent_hello` with a fresh nonce, verifies the Ed25519 signature of `agent_prove` over the
//! signed text of 4.4, and then reads every line with the contract's
//! `decode(Entry::AgentRequest, ...)` and answers from a script of contract values.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, VerifyingKey};
use fabric_desktop_contracts::{
    Entry, Frame, PROTOCOL, SCHEMA_VERSION, canonical, decode, parse_strict,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::scratch::{Scratch, set_mode};

/// What the stand-in answers at `agent_prove` after the signature verifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enrollment {
    Approved,
    Pending,
    NotPermitted,
}

/// What the stand-in does with one call.
#[derive(Clone, Debug)]
pub enum Answer {
    /// Answer with `result`: a contract `tool_outcome`.
    Outcome(Value),
    /// Answer with `error`: a contract agent error.
    Error(Value),
    /// Never answer.
    Never,
    /// Write a line that is not a contract frame.
    Malformed,
    /// Close the connection at once.
    Close,
}

/// The stand-in's script entry for one call, matched by tool name in order.
#[derive(Clone, Debug)]
pub struct Reply {
    pub tool: String,
    pub answer: Answer,
    /// Wait this long before answering.
    pub delay: Duration,
    /// Before answering, wait until an answer for a call of this tool has been written.
    pub after_tool: Option<String>,
    /// Close the connection after answering.
    pub close_after: bool,
}

/// Wraps an answer in a response frame with a placeholder id and checks it with the
/// contract's own reader, so that the stand-in never sends what the contract refuses.
fn check_answer(tool: &str, answer: &Answer) {
    let frame = match answer {
        Answer::Outcome(v) => response_frame("call-check", tool, "result", v),
        Answer::Error(v) => response_frame("call-check", tool, "error", v),
        _ => return,
    };
    let line = canonical(&frame);
    if let Err(refusal) = decode(Entry::AgentResponse, line.as_bytes()) {
        panic!("the stand-in's answer is not a contract agent_response ({refusal}): {line}");
    }
}

fn response_frame(id: &str, method: &str, member: &str, value: &Value) -> Value {
    let mut frame = json!({"protocol": PROTOCOL, "id": id, "method": method});
    frame[member] = value.clone();
    frame
}

impl Reply {
    pub fn new(tool: &str, answer: Answer) -> Reply {
        check_answer(tool, &answer);
        Reply {
            tool: tool.to_owned(),
            answer,
            delay: Duration::ZERO,
            after_tool: None,
            close_after: false,
        }
    }

    pub fn outcome(tool: &str, outcome: Value) -> Reply {
        Reply::new(tool, Answer::Outcome(outcome))
    }

    pub fn error(tool: &str, error: Value) -> Reply {
        Reply::new(tool, Answer::Error(error))
    }

    pub fn delayed(mut self, delay: Duration) -> Reply {
        self.delay = delay;
        self
    }

    pub fn after(mut self, tool: &str) -> Reply {
        self.after_tool = Some(tool.to_owned());
        self
    }

    pub fn then_close(mut self) -> Reply {
        self.close_after = true;
        self
    }
}

/// One connection as the stand-in saw it.
#[derive(Clone, Debug, Default)]
pub struct Connection {
    /// The `params` of `agent_hello`, as received.
    pub hello: Option<Value>,
    /// The raw `agent_hello` and `agent_prove` frames.
    pub hello_frame: Option<Value>,
    pub prove_frame: Option<Value>,
    /// Whether the signature verified.
    pub signature_ok: Option<bool>,
    /// What the stand-in answered at `agent_prove`.
    pub enrollment: Option<Enrollment>,
    /// Why the handshake failed, when it did.
    pub handshake_failure: Option<String>,
    /// Every line after the handshake, raw.
    pub lines_after_handshake: Vec<String>,
    /// Lines after the handshake that the contract refused, with the reason.
    pub refused: Vec<String>,
    /// Whether the stand-in saw the end of the connection (read end or its own close).
    pub closed: bool,
}

/// One decoded call.
#[derive(Clone, Debug)]
pub struct Received {
    pub connection: usize,
    /// The frame as received (a JSON value).
    pub raw: Value,
    /// The frame read by the contract's `decode`.
    pub frame: Frame,
    pub tool: String,
    pub id: String,
}

#[derive(Default)]
struct State {
    enrollment: Option<Enrollment>,
    script: VecDeque<Reply>,
    connections: Vec<Connection>,
    received: Vec<Received>,
    /// Tools whose answer has been written, in order.
    answered: Vec<String>,
    /// Calls that matched no script entry.
    unscripted: Vec<String>,
    /// Streams to shut down on drop or on request.
    streams: Vec<(usize, UnixStream)>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The running stand-in.
pub struct StandIn {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

/// 64 lowercase hexadecimal digits from 256 random bits (design 4.4).
fn nonce() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("the system's random source");
    hex(&buf)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The id form of 4.4: the prefix and 32 hexadecimal digits.
fn is_frame_id(id: &Value, prefix: &str) -> bool {
    id.as_str()
        .and_then(|s| s.strip_prefix(prefix))
        .is_some_and(|rest| is_lower_hex(rest, 32))
}

/// The text the bridge signs (design 4.4).
pub fn signed_text(
    nonce: &str,
    enrollment_id: &str,
    public_key: &str,
    schema_version: u64,
) -> String {
    let subject = json!({
        "broker_nonce": nonce,
        "enrollment_id": enrollment_id,
        "public_key": public_key,
        "schema_version": schema_version,
    });
    format!("fabric-desktop/1\nagent_handshake\n{}", canonical(&subject))
}

/// The pairing code of a public key written `ed25519:<64 hex>` (design 5.3).
pub fn pairing_code(public_key: &str) -> String {
    let digest = Sha256::digest(format!("fabric-desktop/1\nagent_key\n{public_key}").as_bytes());
    let digits = hex(&digest).to_ascii_uppercase();
    format!("{}-{}-{}", &digits[0..4], &digits[4..8], &digits[8..12])
}

/// The agent error of a golden fixture, with its code.
pub fn not_permitted_error() -> Value {
    super::fixture_value("accept-agent-response-error-not-permitted")["error"].clone()
}

impl StandIn {
    /// Binds the socket of `scratch`, sets it to mode 0600 and starts accepting.
    pub fn start(scratch: &Scratch, enrollment: Enrollment) -> StandIn {
        let listener = UnixListener::bind(&scratch.socket)
            .unwrap_or_else(|e| panic!("bind {}: {e}", scratch.socket.display()));
        set_mode(&scratch.socket, 0o600);
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                enrollment: Some(enrollment),
                ..State::default()
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let accept_shared = Arc::clone(&shared);
        let accept = thread::spawn(move || accept_loop(&listener, &accept_shared));
        StandIn {
            shared,
            accept: Some(accept),
        }
    }

    pub fn script(&self, reply: Reply) {
        self.shared.lock().script.push_back(reply);
    }

    pub fn set_enrollment(&self, enrollment: Enrollment) {
        self.shared.lock().enrollment = Some(enrollment);
    }

    pub fn connections(&self) -> Vec<Connection> {
        self.shared.lock().connections.clone()
    }

    pub fn received(&self) -> Vec<Received> {
        self.shared.lock().received.clone()
    }

    pub fn unscripted(&self) -> Vec<String> {
        self.shared.lock().unscripted.clone()
    }

    /// Waits until `check` holds for the state, or panics after `within`.
    pub fn wait_until(
        &self,
        what: &str,
        within: Duration,
        check: impl Fn(&[Connection], &[Received]) -> bool,
    ) {
        let deadline = Instant::now() + within;
        let mut state = self.shared.lock();
        loop {
            if check(&state.connections, &state.received) {
                return;
            }
            let now = Instant::now();
            assert!(
                now < deadline,
                "the stand-in did not see {what} within {within:?}: {:?}",
                state.connections
            );
            state = self
                .shared
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Asserts that nothing the stand-in received was refused or unscripted, and that every
    /// handshake verified.
    pub fn assert_clean(&self) {
        let state = self.shared.lock();
        assert!(
            state.unscripted.is_empty(),
            "unscripted calls: {:?}",
            state.unscripted
        );
        for c in &state.connections {
            assert!(c.refused.is_empty(), "refused frames: {:?}", c.refused);
            assert!(
                c.handshake_failure.is_none(),
                "handshake failed: {:?}",
                c.handshake_failure
            );
        }
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for (_, s) in &self.shared.lock().streams {
            let _ = s.shutdown(Shutdown::Both);
        }
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

fn accept_loop(listener: &UnixListener, shared: &Arc<Shared>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).expect("blocking stream");
                let index = {
                    let mut state = shared.lock();
                    state.connections.push(Connection::default());
                    let index = state.connections.len() - 1;
                    if let Ok(clone) = stream.try_clone() {
                        state.streams.push((index, clone));
                    }
                    index
                };
                shared.changed.notify_all();
                let conn_shared = Arc::clone(shared);
                thread::spawn(move || serve(stream, index, &conn_shared));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
            Err(_) => thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Reads lines with a short timeout so the thread notices `stop`.
struct Lines {
    reader: BufReader<UnixStream>,
    buf: Vec<u8>,
}

impl Lines {
    /// The next line without its line feed, or `None` at the end of the connection.
    fn next(&mut self, shared: &Shared) -> Option<Vec<u8>> {
        loop {
            if shared.stop.load(Ordering::SeqCst) {
                return None;
            }
            match self.reader.read_until(b'\n', &mut self.buf) {
                Ok(0) => return None,
                Ok(_) => {
                    if self.buf.last() == Some(&b'\n') {
                        let mut line = std::mem::take(&mut self.buf);
                        line.pop();
                        return Some(line);
                    }
                    return None;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(_) => return None,
            }
        }
    }
}

type Writer = Arc<Mutex<UnixStream>>;

fn write_line(writer: &Writer, value: &Value) {
    let mut line = canonical(value);
    line.push('\n');
    let mut w = writer.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = w.write_all(line.as_bytes()).and_then(|()| w.flush());
}

fn close(writer: &Writer) {
    let _ = writer
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .shutdown(Shutdown::Both);
}

fn update(shared: &Shared, index: usize, f: impl FnOnce(&mut Connection)) {
    f(&mut shared.lock().connections[index]);
    shared.changed.notify_all();
}

fn fail(shared: &Shared, index: usize, writer: &Writer, why: String) {
    update(shared, index, |c| {
        c.handshake_failure = Some(why);
        c.closed = true;
    });
    close(writer);
}

fn serve(stream: UnixStream, index: usize, shared: &Arc<Shared>) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout");
    let writer: Writer = Arc::new(Mutex::new(stream.try_clone().expect("clone stream")));
    let mut lines = Lines {
        reader: BufReader::new(stream),
        buf: Vec::new(),
    };

    // agent_hello
    let Some(line) = lines.next(shared) else {
        update(shared, index, |c| c.closed = true);
        return;
    };
    let hello = match parse_strict(&line) {
        Ok(v) => v,
        Err(e) => {
            return fail(
                shared,
                index,
                &writer,
                format!("hello is not strict JSON: {e}"),
            );
        }
    };
    update(shared, index, |c| c.hello_frame = Some(hello.clone()));
    let params = hello.get("params").cloned().unwrap_or(Value::Null);
    let keys_ok = hello.as_object().is_some_and(|o| {
        o.len() == 4
            && ["protocol", "id", "method", "params"]
                .iter()
                .all(|k| o.contains_key(*k))
    }) && params.as_object().is_some_and(|o| {
        o.len() == 3
            && ["schema_version", "enrollment_id", "public_key"]
                .iter()
                .all(|k| o.contains_key(*k))
    });
    if !keys_ok
        || hello["protocol"] != json!(PROTOCOL)
        || hello["method"] != json!("agent_hello")
        || !is_frame_id(&hello["id"], "hello-")
    {
        return fail(
            shared,
            index,
            &writer,
            format!("not an agent_hello of 4.4: {hello}"),
        );
    }
    let public_key = params["public_key"].as_str().unwrap_or_default().to_owned();
    let enrollment_id = params["enrollment_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let Some(key_bytes) = public_key
        .strip_prefix("ed25519:")
        .filter(|h| is_lower_hex(h, 64))
        .and_then(unhex)
    else {
        return fail(
            shared,
            index,
            &writer,
            format!("bad public key: {public_key}"),
        );
    };
    if params["schema_version"] != json!(SCHEMA_VERSION) || enrollment_id.is_empty() {
        return fail(
            shared,
            index,
            &writer,
            format!("bad hello params: {params}"),
        );
    }
    update(shared, index, |c| c.hello = Some(params.clone()));
    let broker_nonce = nonce();
    write_line(
        &writer,
        &json!({
            "protocol": PROTOCOL,
            "id": hello["id"],
            "method": "agent_hello",
            "result": {"schema_version": SCHEMA_VERSION, "broker_nonce": broker_nonce},
        }),
    );

    // agent_prove
    let Some(line) = lines.next(shared) else {
        return fail(shared, index, &writer, "closed before agent_prove".into());
    };
    let prove = match parse_strict(&line) {
        Ok(v) => v,
        Err(e) => {
            return fail(
                shared,
                index,
                &writer,
                format!("prove is not strict JSON: {e}"),
            );
        }
    };
    update(shared, index, |c| c.prove_frame = Some(prove.clone()));
    let prove_ok = prove.as_object().is_some_and(|o| o.len() == 4)
        && prove["protocol"] == json!(PROTOCOL)
        && prove["method"] == json!("agent_prove")
        && is_frame_id(&prove["id"], "prove-")
        && prove["params"].as_object().is_some_and(|o| o.len() == 1);
    let signature = prove["params"]["signature"].as_str().unwrap_or_default();
    if !prove_ok || !is_lower_hex(signature, 128) {
        return fail(
            shared,
            index,
            &writer,
            format!("not an agent_prove of 4.4: {prove}"),
        );
    }
    let message = signed_text(&broker_nonce, &enrollment_id, &public_key, SCHEMA_VERSION);
    let verified = (|| {
        let key: [u8; 32] = key_bytes.as_slice().try_into().ok()?;
        let key = VerifyingKey::from_bytes(&key).ok()?;
        let sig: [u8; 64] = unhex(signature)?.as_slice().try_into().ok()?;
        key.verify_strict(message.as_bytes(), &Signature::from_bytes(&sig))
            .ok()
    })()
    .is_some();
    let decided = if verified {
        shared.lock().enrollment.unwrap_or(Enrollment::NotPermitted)
    } else {
        Enrollment::NotPermitted
    };
    update(shared, index, |c| {
        c.signature_ok = Some(verified);
        c.enrollment = Some(decided);
    });
    let mut answer = json!({"protocol": PROTOCOL, "id": prove["id"], "method": "agent_prove"});
    match decided {
        Enrollment::Approved => answer["result"] = json!({"enrollment": "approved"}),
        Enrollment::Pending => answer["result"] = json!({"enrollment": "pending"}),
        Enrollment::NotPermitted => answer["error"] = not_permitted_error(),
    }
    write_line(&writer, &answer);

    // Every later line.
    while let Some(line) = lines.next(shared) {
        let text = String::from_utf8_lossy(&line).into_owned();
        update(shared, index, |c| {
            c.lines_after_handshake.push(text.clone())
        });
        if decided != Enrollment::Approved {
            continue;
        }
        let frame = match decode(Entry::AgentRequest, &line) {
            Ok(f) => f,
            Err(refusal) => {
                update(shared, index, |c| {
                    c.refused.push(format!("{refusal}: {text}"))
                });
                continue;
            }
        };
        let raw = parse_strict(&line).expect("decoded frames are strict JSON");
        let tool = raw["method"].as_str().unwrap_or_default().to_owned();
        let id = raw["id"].as_str().unwrap_or_default().to_owned();
        let reply = {
            let mut state = shared.lock();
            state.received.push(Received {
                connection: index,
                raw: raw.clone(),
                frame,
                tool: tool.clone(),
                id: id.clone(),
            });
            let position = state.script.iter().position(|r| r.tool == tool);
            let reply = position.and_then(|p| state.script.remove(p));
            if reply.is_none() {
                state.unscripted.push(tool.clone());
            }
            reply
        };
        shared.changed.notify_all();
        let reply = reply.unwrap_or_else(|| Reply {
            tool: tool.clone(),
            answer: Answer::Error(
                super::fixture_value("accept-agent-response-error-internal")["error"].clone(),
            ),
            delay: Duration::ZERO,
            after_tool: None,
            close_after: false,
        });
        let answer_shared = Arc::clone(shared);
        let answer_writer = Arc::clone(&writer);
        thread::spawn(move || answer_call(&answer_shared, index, &answer_writer, &id, reply));
    }
    update(shared, index, |c| c.closed = true);
}

fn answer_call(shared: &Shared, index: usize, writer: &Writer, id: &str, reply: Reply) {
    if let Some(tool) = &reply.after_tool {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut state = shared.lock();
        while !state.answered.contains(tool) && Instant::now() < deadline {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
    if !reply.delay.is_zero() {
        thread::sleep(reply.delay);
    }
    match &reply.answer {
        Answer::Outcome(v) => write_line(writer, &response_frame(id, &reply.tool, "result", v)),
        Answer::Error(v) => write_line(writer, &response_frame(id, &reply.tool, "error", v)),
        Answer::Never => return,
        Answer::Malformed => {
            let mut w = writer.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = w
                .write_all(b"{this is not a contract frame\n")
                .and_then(|()| w.flush());
        }
        Answer::Close => {
            close(writer);
            update(shared, index, |c| c.closed = true);
            return;
        }
    }
    shared.lock().answered.push(reply.tool.clone());
    shared.changed.notify_all();
    if reply.close_after {
        close(writer);
        update(shared, index, |c| c.closed = true);
    }
}
