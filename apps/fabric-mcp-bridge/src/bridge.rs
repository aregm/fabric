//! The state machine of the bridge (W06 2.3): calls in flight, their limits, cancellation.
//!
//! `Bridge` does no I/O and has no clock: the process loop hands it events with the time and
//! carries out the effects it returns. One call is one frame out and one answer in; the bridge
//! never polls, retries or keeps a result (A10, A11, A12).

use std::collections::BTreeMap;

use fabric_desktop_contracts::agent::AgentResponse;
use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::error::{AgentError, AgentFault};

use crate::broker::frames;
use crate::mcp::jsonrpc::{self, INVALID_REQUEST, RpcId};
use crate::mcp::render::{self, BridgeFault, Rendered};
use crate::mcp::server::{self, Era, Out, Server, ToolCall};
use crate::mcp::tools;

/// Calls in flight at most (W06 2.5).
pub const MAX_CALLS: usize = 16;
/// How long the bridge waits for the answer to one call (W06 2.5).
pub const ANSWER_LIMIT_MS: u64 = 30_000;

/// What the loop hands the bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// One line from the client, without its line feed.
    McpLine(Vec<u8>),
    /// A line from the client over the 2 MiB bound; its rest was skipped.
    McpLineTooLong,
    /// One line from the broker, without its line feed.
    BrokerFrame(Vec<u8>),
    /// A line from the broker over the frame bound.
    BrokerFrameTooLong,
    /// The broker closed the connection.
    BrokerClosed,
    /// A frame could not be written whole within its limit (W06 2.5); the loop has closed
    /// the connection.
    BrokerWriteFailed,
    /// The outcome of the connection and handshake that `Effect::Connect` asked for.
    Connected(ConnectOutcome),
    /// Time has passed.
    Tick,
}

/// What the loop is asked to do, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Write this JSON-RPC message, then a line feed, to stdout.
    Stdout(String),
    /// Write this frame line (with its line feed) to the broker.
    SendFrame(String),
    /// Connect and run the handshake (W06 4.2 to 4.4), then hand back `Event::Connected`.
    Connect,
    /// Drop the connection: its frames can no longer be trusted (W06 2.5).
    Close,
    /// Write this line to stderr. It holds only fixed words, ids, tool names, state names and
    /// error codes (A25).
    Log(String),
}

/// How a connection attempt ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectOutcome {
    /// The handshake ended `approved`: the session has started.
    Approved { pairing_code: String },
    /// The handshake ended `pending`; the loop has closed the connection.
    Pending { pairing_code: String },
    /// No connection: the error for the waiting calls, the code if a key was had, and a fixed
    /// reason for stderr.
    Refused {
        error: AgentError,
        pairing_code: Option<String>,
        reason: &'static str,
    },
    /// No connection because of a fault of the bridge itself, with the code if a key was had
    /// and a fixed reason for stderr.
    Fault {
        fault: BridgeFault,
        pairing_code: Option<String>,
        reason: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Down,
    Connecting,
    Up,
}

#[derive(Clone, Debug)]
enum Stage {
    /// Waiting for the connection, with the frame line to send.
    Waiting(String),
    /// Sent; the answer is due by this time.
    Sent { due_ms: u64 },
}

#[derive(Clone, Debug)]
struct Call {
    rpc_id: RpcId,
    era: Era,
    tool: ToolName,
    stage: Stage,
}

/// The source of 128-bit random values for call ids.
pub type Ids = Box<dyn FnMut() -> Option<[u8; 16]> + Send>;

/// The bridge's whole state.
pub struct Bridge {
    server: Server,
    calls: BTreeMap<String, Call>,
    link: Link,
    pairing_code: Option<String>,
    ids: Ids,
}

impl Bridge {
    #[must_use]
    pub fn new(ids: Ids) -> Bridge {
        Bridge {
            server: Server::new(),
            calls: BTreeMap::new(),
            link: Link::Down,
            pairing_code: None,
            ids,
        }
    }

    /// The earliest time a call's answer is due, if any call waits for one.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        self.calls
            .values()
            .filter_map(|c| match c.stage {
                Stage::Sent { due_ms } => Some(due_ms),
                Stage::Waiting(_) => None,
            })
            .min()
    }

    /// The number of calls in flight.
    #[must_use]
    pub fn calls_in_flight(&self) -> usize {
        self.calls.len()
    }

    /// Handles one event at time `now_ms` and returns what the loop must do.
    pub fn handle(&mut self, event: Event, now_ms: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match event {
            Event::McpLine(bytes) => match self.server.handle_line(&bytes) {
                None => {}
                Some(Out::Line(message)) => out.push(stdout(&message)),
                Some(Out::Call(call)) => self.call(call, now_ms, &mut out),
                Some(Out::Cancel(id)) => self.cancel(&id, &mut out),
            },
            Event::McpLineTooLong => out.push(stdout(&jsonrpc::error(
                None,
                INVALID_REQUEST,
                "Invalid Request",
                None,
            ))),
            Event::BrokerFrame(bytes) => self.frame(&bytes, &mut out),
            Event::BrokerFrameTooLong => {
                self.drop_connection("broker frame over the bound", &mut out)
            }
            Event::BrokerWriteFailed => {
                self.drop_connection("a write to the broker did not finish", &mut out);
            }
            Event::BrokerClosed => {
                if self.link == Link::Up {
                    out.push(log("the broker closed the connection"));
                    self.link = Link::Down;
                    self.fail_sent(&mut out);
                }
            }
            Event::Connected(outcome) => self.connected(outcome, now_ms, &mut out),
            Event::Tick => self.expire(now_ms, &mut out),
        }
        out
    }

    fn call(&mut self, call: ToolCall, now_ms: u64, out: &mut Vec<Effect>) {
        let ToolCall {
            id,
            era,
            tool,
            mut arguments,
        } = call;
        if self.calls.len() >= MAX_CALLS {
            out.push(log(&format!(
                "tool {} refused: too many calls",
                tool.as_str()
            )));
            out.push(answer(era, &id, &render::too_many_calls()));
            return;
        }
        let Some(random) = (self.ids)() else {
            out.push(log("the random source failed"));
            out.push(answer(era, &id, &render::fault(BridgeFault::RandomSource)));
            return;
        };
        let call_id = format!("call-{}", crate::hex(&random));
        tools::fill_nulls(tool, &mut arguments);
        let request = match frames::request(&call_id, tool, arguments) {
            Ok(request) => request,
            Err(refusal) => {
                // The reason can repeat an argument: it goes into the tool result only (A25).
                out.push(log(&format!(
                    "call {call_id} tool {} error invalid_params",
                    tool.as_str()
                )));
                let error = render::invalid_params(&refusal.reason);
                out.push(answer(era, &id, &render::error(&error, None)));
                return;
            }
        };
        let line = frames::request_line(&request);
        let stage = match self.link {
            Link::Up => {
                out.push(Effect::SendFrame(line));
                Stage::Sent {
                    due_ms: now_ms + ANSWER_LIMIT_MS,
                }
            }
            Link::Down => {
                self.link = Link::Connecting;
                out.push(Effect::Connect);
                Stage::Waiting(line)
            }
            Link::Connecting => Stage::Waiting(line),
        };
        out.push(log(&format!("call {call_id} tool {}", tool.as_str())));
        self.calls.insert(
            call_id,
            Call {
                rpc_id: id,
                era,
                tool,
                stage,
            },
        );
    }

    /// `notifications/cancelled`: the call is removed and gets no response; a late answer
    /// is dropped. The broker is told nothing (W06 3.9).
    fn cancel(&mut self, id: &RpcId, out: &mut Vec<Effect>) {
        let found = self
            .calls
            .iter()
            .find(|(_, c)| c.rpc_id == *id)
            .map(|(k, _)| k.clone());
        if let Some(call_id) = found {
            self.calls.remove(&call_id);
            out.push(log(&format!("call {call_id} cancelled")));
        }
    }

    fn connected(&mut self, outcome: ConnectOutcome, now_ms: u64, out: &mut Vec<Effect>) {
        let waiting: Vec<String> = self
            .calls
            .iter()
            .filter(|(_, c)| matches!(c.stage, Stage::Waiting(_)))
            .map(|(k, _)| k.clone())
            .collect();
        match outcome {
            ConnectOutcome::Approved { pairing_code } => {
                self.pairing_code = Some(pairing_code);
                self.link = Link::Up;
                out.push(log("connected"));
                for call_id in waiting {
                    if let Some(call) = self.calls.get_mut(&call_id)
                        && let Stage::Waiting(line) = std::mem::replace(
                            &mut call.stage,
                            Stage::Sent {
                                due_ms: now_ms + ANSWER_LIMIT_MS,
                            },
                        )
                    {
                        out.push(Effect::SendFrame(line));
                    }
                }
            }
            ConnectOutcome::Pending { pairing_code } => {
                self.link = Link::Down;
                out.push(log("enrollment pending"));
                let rendered = render::pending(&pairing_code);
                self.pairing_code = Some(pairing_code);
                for call_id in waiting {
                    self.finish(&call_id, &rendered, "error not_permitted", out);
                }
            }
            ConnectOutcome::Refused {
                error,
                pairing_code,
                reason,
            } => {
                self.link = Link::Down;
                if pairing_code.is_some() {
                    self.pairing_code = pairing_code;
                }
                out.push(log(&format!(
                    "no connection: {reason}; {}",
                    error.code().as_str()
                )));
                let rendered = render::error(&error, self.pairing_code.as_deref());
                let note = format!("error {}", error.code().as_str());
                for call_id in waiting {
                    self.finish(&call_id, &rendered, &note, out);
                }
            }
            ConnectOutcome::Fault {
                fault,
                pairing_code,
                reason,
            } => {
                self.link = Link::Down;
                if pairing_code.is_some() {
                    self.pairing_code = pairing_code;
                }
                out.push(log(&format!("no connection: {reason}; internal")));
                let rendered = render::fault(fault);
                for call_id in waiting {
                    self.finish(&call_id, &rendered, "error internal", out);
                }
            }
        }
    }

    fn frame(&mut self, bytes: &[u8], out: &mut Vec<Effect>) {
        let response = match frames::response(bytes) {
            Ok(response) => response,
            Err(refusal) => {
                let reason = format!("broker frame refused: {}", refusal.code.as_str());
                self.drop_connection(&reason, out);
                return;
            }
        };
        let (id, rendered, note) = match &response {
            AgentResponse::Result { id, result, .. } => (
                id.as_str().to_owned(),
                render::outcome(result),
                format!("state {}", result.request.status.state().as_str()),
            ),
            AgentResponse::Error {
                id: Some(id),
                error,
                ..
            } => (
                id.as_str().to_owned(),
                render::error(error, self.pairing_code.as_deref()),
                format!("error {}", error.code().as_str()),
            ),
            AgentResponse::Error {
                id: None, error, ..
            } => {
                out.push(log(&format!(
                    "a broker error without an id was dropped: {}",
                    error.code().as_str()
                )));
                return;
            }
        };
        match self.calls.get(&id) {
            Some(call) if matches!(call.stage, Stage::Sent { .. }) => {
                self.finish(&id, &rendered, &note, out);
            }
            _ => out.push(log("an answer for no call in flight was dropped")),
        }
    }

    /// Answers and removes one call.
    fn finish(&mut self, call_id: &str, rendered: &Rendered, note: &str, out: &mut Vec<Effect>) {
        if let Some(call) = self.calls.remove(call_id) {
            out.push(log(&format!(
                "call {call_id} tool {} {note}",
                call.tool.as_str()
            )));
            out.push(answer(call.era, &call.rpc_id, rendered));
        }
    }

    fn drop_connection(&mut self, reason: &str, out: &mut Vec<Effect>) {
        out.push(log(reason));
        if self.link == Link::Up {
            out.push(Effect::Close);
            self.link = Link::Down;
            self.fail_sent(out);
        }
    }

    /// Every sent call is answered `broker_not_running`: its connection ended.
    fn fail_sent(&mut self, out: &mut Vec<Effect>) {
        let error = render::bridge_error(
            AgentFault::BrokerNotRunning,
            "The connection to Fabric Calendar ended.",
        );
        let rendered = render::error(&error, self.pairing_code.as_deref());
        let sent: Vec<String> = self
            .calls
            .iter()
            .filter(|(_, c)| matches!(c.stage, Stage::Sent { .. }))
            .map(|(k, _)| k.clone())
            .collect();
        for call_id in sent {
            self.finish(&call_id, &rendered, "error broker_not_running", out);
        }
    }

    /// A call whose answer is overdue is an `internal` tool error; the connection and the
    /// other calls stay (W06 2.5).
    fn expire(&mut self, now_ms: u64, out: &mut Vec<Effect>) {
        let overdue: Vec<String> = self
            .calls
            .iter()
            .filter(|(_, c)| matches!(c.stage, Stage::Sent { due_ms } if due_ms <= now_ms))
            .map(|(k, _)| k.clone())
            .collect();
        if overdue.is_empty() {
            return;
        }
        let error = render::bridge_error(
            AgentFault::Internal,
            "Fabric Calendar did not answer within 30 seconds.",
        );
        let rendered = render::error(&error, None);
        for call_id in overdue {
            self.finish(&call_id, &rendered, "error internal (no answer)", out);
        }
    }
}

fn stdout(message: &serde_json::Value) -> Effect {
    Effect::Stdout(jsonrpc::to_line(message))
}

fn answer(era: Era, id: &RpcId, rendered: &Rendered) -> Effect {
    stdout(&server::tool_result(era, id, rendered))
}

fn log(text: &str) -> Effect {
    Effect::Log(format!("{}: {text}", crate::NAME))
}

/// The bridge's error when no connection can be had on this system or build.
#[must_use]
pub fn not_running(reason: &'static str) -> ConnectOutcome {
    ConnectOutcome::Refused {
        error: render::bridge_error(
            AgentFault::BrokerNotRunning,
            "Fabric Calendar is not running.",
        ),
        pairing_code: None,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::{Entry, Frame, decode};
    use serde_json::{Value, json};

    const MARKER: &str = "MARKER-b09";
    const CODE: &str = "AB12-CD34-EF56";

    fn bridge() -> Bridge {
        let mut n = 0u8;
        Bridge::new(Box::new(move || {
            n = n.wrapping_add(1);
            Some([n; 16])
        }))
    }

    fn call_line(id: Value, tool: &str, arguments: Value) -> Event {
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": arguments,
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {},
                },
            },
        });
        Event::McpLine(message.to_string().into_bytes())
    }

    fn status_call(id: i64) -> Event {
        call_line(
            json!(id),
            "fabric_request_status",
            json!({"request_id": "req-1"}),
        )
    }

    fn stdout_of(effects: &[Effect]) -> Vec<Value> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Stdout(l) => Some(serde_json::from_str(l).unwrap()),
                _ => None,
            })
            .collect()
    }

    fn frames_of(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::SendFrame(l) => Some(l.clone()),
                _ => None,
            })
            .collect()
    }

    fn frame_id(line: &str) -> String {
        match decode(Entry::AgentRequest, line.trim_end().as_bytes()).unwrap() {
            Frame::AgentRequest(r) => r.id.as_str().to_owned(),
            _ => unreachable!(),
        }
    }

    /// A bridge with an approved connection.
    fn connected() -> Bridge {
        let mut b = bridge();
        b.link = Link::Up;
        b.pairing_code = Some(CODE.to_owned());
        b
    }

    fn outcome_frame(id: &str, state: &str, result: Value) -> Event {
        Event::BrokerFrame(
            json!({
                "protocol": "fabric-desktop/1",
                "id": id,
                "method": "fabric_request_status",
                "result": {
                    "request": {
                        "request_id": "req-1",
                        "updated_at": "2026-11-20T15:00:00.000Z",
                        "status": {"state": state},
                    },
                    "result": result,
                },
            })
            .to_string()
            .into_bytes(),
        )
    }

    fn text(answer: &Value, block: usize) -> String {
        answer["result"]["content"][block]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    // b01: arguments the contract refuses never reach the broker; the answer is
    // invalid_params with the pointer /params.
    #[test]
    fn b01_refused_arguments() {
        let mut b = connected();
        let effects = b.handle(
            call_line(
                json!(1),
                "fabric_calendar_propose_create",
                json!({"account_id": "a", "calendar_id": "c", "task": "t", "event": {"title": ""}}),
            ),
            0,
        );
        assert!(frames_of(&effects).is_empty());
        assert!(!effects.contains(&Effect::Connect));
        let out = stdout_of(&effects);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["result"]["isError"], json!(true));
        let error: Value = serde_json::from_str(&text(&out[0], 1)).unwrap();
        assert_eq!(error["error"]["code"], json!("invalid_params"));
        assert_eq!(error["error"]["detail"], json!({"pointer": "/params"}));
        // The reason is never on stderr: the call's only log line is fixed words, its id,
        // the tool's name and the error code.
        let logs: Vec<&str> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Log(l) => Some(l.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            logs,
            vec![format!(
                "fabric-mcp-bridge: call call-{} tool fabric_calendar_propose_create error invalid_params",
                "01".repeat(16)
            )]
        );
        assert_eq!(b.calls_in_flight(), 0);
    }

    // b02: an unknown tool name, an owner method's name among them, gets -32602 and sends
    // nothing.
    #[test]
    fn b02_unknown_tool() {
        let mut b = connected();
        for name in [
            "approve_request",
            "hello",
            "fabric_calendar_delete",
            "subscribe",
        ] {
            let effects = b.handle(call_line(json!(name), name, json!({})), 0);
            assert!(frames_of(&effects).is_empty());
            let out = stdout_of(&effects);
            assert_eq!(out[0]["error"]["code"], json!(-32602), "{name}");
            assert_eq!(out[0]["id"], json!(name));
        }
        // Arguments that are not an object.
        let effects = b.handle(call_line(json!(9), "fabric_calendar_list", json!([1])), 0);
        assert_eq!(stdout_of(&effects)[0]["error"]["code"], json!(-32602));
    }

    // b03: a call is answered by the one broker answer with its id; no other frame is sent
    // for it (A10, A11).
    #[test]
    fn b03_one_frame_one_answer() {
        let mut b = bridge();
        let first = b.handle(status_call(1), 0);
        assert_eq!(
            first
                .iter()
                .filter(|e| matches!(e, Effect::Connect))
                .count(),
            1
        );
        assert!(frames_of(&first).is_empty());
        let sent = b.handle(
            Event::Connected(ConnectOutcome::Approved {
                pairing_code: CODE.to_owned(),
            }),
            10,
        );
        let lines = frames_of(&sent);
        assert_eq!(lines.len(), 1);
        let id = frame_id(&lines[0]);
        assert!(id.starts_with("call-") && id.len() == 5 + 32);
        let answered = b.handle(outcome_frame(&id, "pending_approval_x", Value::Null), 20);
        // A frame that is not the contract's drops the connection instead.
        assert!(answered.contains(&Effect::Close));
        let mut b = connected();
        let sent = b.handle(status_call(2), 0);
        let id = frame_id(&frames_of(&sent)[0]);
        let answered = b.handle(outcome_frame(&id, "queued", Value::Null), 5);
        assert!(frames_of(&answered).is_empty());
        let out = stdout_of(&answered);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(2));
        assert_eq!(out[0]["result"]["isError"], json!(false));
        assert!(text(&out[0], 0).contains("still working"));
        // Nothing more happens for it: no poll, no retry.
        assert!(b.handle(Event::Tick, 60_000).is_empty());
        assert_eq!(b.calls_in_flight(), 0);
    }

    // b04: after 30 s without an answer that call alone is the tool error of 3.7; the
    // connection and the other calls stay.
    #[test]
    fn b04_answer_limit() {
        let mut b = connected();
        let first = frame_id(&frames_of(&b.handle(status_call(1), 0))[0]);
        let second = frame_id(&frames_of(&b.handle(status_call(2), 10_000))[0]);
        assert_eq!(b.next_deadline(), Some(30_000));
        assert!(b.handle(Event::Tick, 29_999).is_empty());
        let effects = b.handle(Event::Tick, 30_000);
        assert!(!effects.contains(&Effect::Close));
        let out = stdout_of(&effects);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(1));
        assert_eq!(out[0]["result"]["isError"], json!(true));
        assert!(text(&out[0], 1).contains("\"internal\""));
        assert_eq!(b.link, Link::Up);
        // The late answer of the first is dropped; the second is still answered.
        assert!(
            stdout_of(&b.handle(outcome_frame(&first, "queued", Value::Null), 31_000)).is_empty()
        );
        let out = stdout_of(&b.handle(outcome_frame(&second, "queued", Value::Null), 31_000));
        assert_eq!(out[0]["id"], json!(2));
    }

    // b05: a cancelled call gets no response, and its late answer is dropped.
    #[test]
    fn b05_cancel() {
        let mut b = connected();
        let id = frame_id(&frames_of(&b.handle(status_call(7), 0))[0]);
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 7}});
        let effects = b.handle(Event::McpLine(cancel.to_string().into_bytes()), 1);
        assert!(stdout_of(&effects).is_empty());
        assert!(frames_of(&effects).is_empty());
        assert_eq!(b.calls_in_flight(), 0);
        assert!(stdout_of(&b.handle(outcome_frame(&id, "queued", Value::Null), 2)).is_empty());
        // A call cancelled while waiting for the connection is not sent.
        let mut b = bridge();
        b.handle(status_call(8), 0);
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 8}});
        b.handle(Event::McpLine(cancel.to_string().into_bytes()), 1);
        let effects = b.handle(
            Event::Connected(ConnectOutcome::Approved {
                pairing_code: CODE.to_owned(),
            }),
            2,
        );
        assert!(frames_of(&effects).is_empty());
        assert!(stdout_of(&effects).is_empty());
    }

    // b06: a closed connection answers the calls in flight; the next call connects again.
    #[test]
    fn b06_closed_connection() {
        let mut b = connected();
        b.handle(status_call(1), 0);
        b.handle(status_call(2), 0);
        let effects = b.handle(Event::BrokerClosed, 5);
        let out = stdout_of(&effects);
        assert_eq!(out.len(), 2);
        for a in &out {
            assert!(text(a, 1).contains("broker_not_running"));
        }
        assert_eq!(b.calls_in_flight(), 0);
        let effects = b.handle(status_call(3), 6);
        assert!(effects.contains(&Effect::Connect));
        // A pending handshake answers with the pairing code.
        let effects = b.handle(
            Event::Connected(ConnectOutcome::Pending {
                pairing_code: CODE.to_owned(),
            }),
            7,
        );
        let out = stdout_of(&effects);
        assert!(text(&out[0], 0).contains(CODE));
        assert!(text(&out[0], 1).contains("not_permitted"));
        // A refused connection answers broker_not_running, and the next call tries again.
        b.handle(status_call(4), 8);
        let out = stdout_of(&b.handle(Event::Connected(not_running("test")), 9));
        assert!(text(&out[0], 1).contains("broker_not_running"));
        assert!(b.handle(status_call(5), 10).contains(&Effect::Connect));
        // A write that did not finish drops the connection the same way.
        let mut b = connected();
        b.handle(status_call(1), 0);
        let effects = b.handle(Event::BrokerWriteFailed, 1);
        assert!(effects.contains(&Effect::Close));
        let out = stdout_of(&effects);
        assert_eq!(out.len(), 1);
        assert!(text(&out[0], 1).contains("broker_not_running"));
        assert!(b.handle(status_call(2), 2).contains(&Effect::Connect));
        // A fault of the bridge answers with its own wording.
        let effects = b.handle(
            Event::Connected(ConnectOutcome::Fault {
                fault: BridgeFault::Key,
                pairing_code: None,
                reason: "test",
            }),
            3,
        );
        let out = stdout_of(&effects);
        assert_eq!(text(&out[0], 0), BridgeFault::Key.guidance());
        // An oversized frame drops the connection.
        let mut b = connected();
        b.handle(status_call(1), 0);
        let effects = b.handle(Event::BrokerFrameTooLong, 1);
        assert!(effects.contains(&Effect::Close));
        assert_eq!(stdout_of(&effects).len(), 1);
    }

    // b07: answers that come out of order reach their own calls.
    #[test]
    fn b07_out_of_order() {
        let mut b = connected();
        let a = frame_id(&frames_of(&b.handle(status_call(1), 0))[0]);
        let c = frame_id(&frames_of(&b.handle(status_call(2), 0))[0]);
        let out = stdout_of(&b.handle(outcome_frame(&c, "denied", Value::Null), 1));
        assert_eq!(out[0]["id"], json!(2));
        assert!(text(&out[0], 0).contains("denied"));
        let out = stdout_of(&b.handle(outcome_frame(&a, "applied", Value::Null), 2));
        assert_eq!(out[0]["id"], json!(1));
        assert!(text(&out[0], 0).contains("applied"));
    }

    // b08: the seventeenth call in flight is a tool error.
    #[test]
    fn b08_seventeenth_call() {
        let mut b = connected();
        for i in 0..16 {
            assert_eq!(frames_of(&b.handle(status_call(i), 0)).len(), 1);
        }
        let effects = b.handle(status_call(16), 0);
        assert!(frames_of(&effects).is_empty());
        let out = stdout_of(&effects);
        assert_eq!(out[0]["id"], json!(16));
        assert_eq!(out[0]["result"]["isError"], json!(true));
        assert!(text(&out[0], 0).contains("Too many calls"));
        assert_eq!(b.calls_in_flight(), 16);
    }

    // b09: a result shown in one call's answer appears in no later answer (A12).
    #[test]
    fn b09_no_cache() {
        let mut b = connected();
        let id = frame_id(&frames_of(&b.handle(status_call(1), 0))[0]);
        let result = json!({
            "kind": "calendars",
            "accounts": [{"account_id": "acc", "calendars": [{
                "calendar_id": "cal",
                "name": MARKER,
                "access": "owner",
                "agent_drafts": false,
            }]}],
        });
        let out = stdout_of(&b.handle(outcome_frame(&id, "reconciled", result), 1));
        assert!(text(&out[0], 1).contains(MARKER));
        let id = frame_id(&frames_of(&b.handle(status_call(2), 2))[0]);
        let later = b.handle(outcome_frame(&id, "reconciled", Value::Null), 3);
        for e in &later {
            let s = format!("{e:?}");
            assert!(!s.contains(MARKER));
        }
        // A broker that closes, and a call that times out, show nothing of it either.
        b.handle(status_call(3), 4);
        for e in b.handle(Event::BrokerClosed, 5) {
            assert!(!format!("{e:?}").contains(MARKER));
        }
    }
}
