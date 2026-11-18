//! The era rule and the methods (W06 3.3, 3.4).
//!
//! A request whose `params._meta` holds `io.modelcontextprotocol/protocolVersion` is modern;
//! any other is legacy and needs an answered `initialize` first. The only MCP state is whether
//! `initialize` was answered and which `subscriptions/listen` are open.

use fabric_desktop_contracts::common::ToolName;
use serde_json::{Map, Value, json};

use super::jsonrpc::{
    self, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND, Message, RpcId,
    UNSUPPORTED_PROTOCOL_VERSION,
};
use super::render::Rendered;
use super::tools;

/// The modern revision, the only one a request's `_meta` may carry.
pub const MODERN: &str = "2026-07-28";
/// The legacy revision `initialize` answers with.
pub const LEGACY: &str = "2025-11-25";
/// Open `subscriptions/listen` at most (W06 2.5).
pub const MAX_LISTENS: usize = 8;
/// How long a client may keep `server/discover` and `tools/list` (W06 3.3).
pub const TTL_MS: u64 = 300_000;

const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
const SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";

/// The fixed instructions: the two rules first (W06 3.3).
pub const INSTRUCTIONS: &str = "An answer of pending_approval means the owner must approve the request in Fabric Calendar. Never repeat a proposal: every call makes a new request and a new approval card. fabric_request_status with the request_id shows how a request stands. Fabric Calendar guards the owner's calendars.";

/// The revision a request is served in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    Modern,
    Legacy,
}

/// A `tools/call` that names one of the ten tools with an object of arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: RpcId,
    pub era: Era,
    pub tool: ToolName,
    pub arguments: Value,
}

/// What the server makes of one line.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// A message for stdout.
    Line(Value),
    /// A tool call for the bridge.
    Call(ToolCall),
    /// `notifications/cancelled` for an id that is not an open subscription.
    Cancel(RpcId),
}

/// `serverInfo`.
#[must_use]
pub fn server_info() -> Value {
    json!({"name": crate::NAME, "title": "Fabric Calendar", "version": crate::VERSION})
}

fn modern_meta() -> Value {
    json!({ SERVER_INFO: server_info() })
}

/// Adds the modern members to a result object.
fn modern(mut result: Map<String, Value>) -> Value {
    result.insert("resultType".to_owned(), json!("complete"));
    result.insert("_meta".to_owned(), modern_meta());
    Value::Object(result)
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// The result of a `tools/call` in its era (W06 3.3): `content` and `isError`, and in the
/// modern era also `resultType` and `_meta`.
#[must_use]
pub fn tool_result(era: Era, id: &RpcId, rendered: &Rendered) -> Value {
    let base = object(json!({"content": rendered.content(), "isError": rendered.is_error}));
    let result = match era {
        Era::Modern => modern(base),
        Era::Legacy => Value::Object(base),
    };
    jsonrpc::result(id, result)
}

/// The MCP state of one process.
#[derive(Debug, Default)]
pub struct Server {
    initialized: bool,
    listens: Vec<RpcId>,
}

impl Server {
    #[must_use]
    pub fn new() -> Server {
        Server::default()
    }

    /// Serves one line from the client.
    pub fn handle_line(&mut self, bytes: &[u8]) -> Option<Out> {
        match jsonrpc::read_line(bytes) {
            Message::Invalid(error) => Some(Out::Line(error)),
            Message::Response => None,
            Message::Notification { method, params } => self.notification(&method, params),
            Message::Request { id, method, params } => Some(self.request(id, &method, params)),
        }
    }

    fn notification(&mut self, method: &str, params: Option<Value>) -> Option<Out> {
        if method != "notifications/cancelled" {
            return None;
        }
        let id = RpcId::read(params.as_ref()?.get("requestId")?)?;
        if let Some(at) = self.listens.iter().position(|l| *l == id) {
            self.listens.remove(at);
            return None;
        }
        Some(Out::Cancel(id))
    }

    fn request(&mut self, id: RpcId, method: &str, params: Option<Value>) -> Out {
        let fail = |id: &RpcId, code: i64, message: &str| {
            Out::Line(jsonrpc::error(Some(id), code, message, None))
        };
        // Rule 2.
        if method == "initialize" {
            self.initialized = true;
            return Out::Line(jsonrpc::result(
                &id,
                json!({
                    "protocolVersion": LEGACY,
                    "capabilities": {"tools": {}},
                    "serverInfo": server_info(),
                    "instructions": INSTRUCTIONS,
                }),
            ));
        }
        let meta = params
            .as_ref()
            .and_then(|p| p.get("_meta"))
            .and_then(Value::as_object);
        // Rule 3.
        if let Some(version) = meta.and_then(|m| m.get(PROTOCOL_VERSION)) {
            if version != &json!(MODERN) {
                return Out::Line(jsonrpc::error(
                    Some(&id),
                    UNSUPPORTED_PROTOCOL_VERSION,
                    "Unsupported protocol version",
                    Some(json!({"supported": [MODERN], "requested": version})),
                ));
            }
            if !meta
                .and_then(|m| m.get(CLIENT_CAPABILITIES))
                .is_some_and(Value::is_object)
            {
                return fail(&id, INVALID_PARAMS, "Missing client capabilities");
            }
            return self.modern_request(id, method, params.as_ref());
        }
        // Rule 4.
        if method == "ping" {
            return Out::Line(jsonrpc::result(&id, json!({})));
        }
        if !self.initialized {
            return fail(
                &id,
                INVALID_PARAMS,
                "The request carries no protocol version",
            );
        }
        match method {
            "tools/list" => list(&id, params.as_ref(), Era::Legacy),
            "tools/call" => call(id, params, Era::Legacy),
            _ => fail(&id, METHOD_NOT_FOUND, "Method not found"),
        }
    }

    fn modern_request(&mut self, id: RpcId, method: &str, params: Option<&Value>) -> Out {
        match method {
            "server/discover" => Out::Line(jsonrpc::result(
                &id,
                modern(object(json!({
                    "supportedVersions": [MODERN],
                    "capabilities": {"tools": {}},
                    "instructions": INSTRUCTIONS,
                    "ttlMs": TTL_MS,
                    "cacheScope": "public",
                }))),
            )),
            "tools/list" => list(&id, params, Era::Modern),
            "tools/call" => call(id, params.cloned(), Era::Modern),
            "subscriptions/listen" => self.listen(id, params),
            _ => Out::Line(jsonrpc::error(
                Some(&id),
                METHOD_NOT_FOUND,
                "Method not found",
                None,
            )),
        }
    }

    /// `subscriptions/listen`: acknowledged by a notification under the request's id, and
    /// then nothing (W06 3.3).
    fn listen(&mut self, id: RpcId, params: Option<&Value>) -> Out {
        if !params
            .and_then(|p| p.get("notifications"))
            .is_some_and(Value::is_object)
        {
            return Out::Line(jsonrpc::error(
                Some(&id),
                INVALID_PARAMS,
                "Missing notifications",
                None,
            ));
        }
        if self.listens.len() >= MAX_LISTENS {
            return Out::Line(jsonrpc::error(
                Some(&id),
                INTERNAL_ERROR,
                "Too many open subscriptions",
                None,
            ));
        }
        let ack = jsonrpc::notification(
            "notifications/subscriptions/acknowledged",
            json!({
                "_meta": { SUBSCRIPTION_ID: id.value() },
                "notifications": {},
            }),
        );
        self.listens.push(id);
        Out::Line(ack)
    }
}

fn list(id: &RpcId, params: Option<&Value>, era: Era) -> Out {
    if params.and_then(|p| p.get("cursor")).is_some() {
        return Out::Line(jsonrpc::error(
            Some(id),
            INVALID_PARAMS,
            "No cursor was issued",
            None,
        ));
    }
    let base = object(json!({"tools": tools::list_json()}));
    let result = match era {
        Era::Modern => {
            let mut m = base;
            m.insert("ttlMs".to_owned(), json!(TTL_MS));
            m.insert("cacheScope".to_owned(), json!("public"));
            modern(m)
        }
        Era::Legacy => Value::Object(base),
    };
    Out::Line(jsonrpc::result(id, result))
}

/// `tools/call` steps 1 and 2 (W06 3.6).
fn call(id: RpcId, params: Option<Value>, era: Era) -> Out {
    let invalid = |id: &RpcId, message: &str| {
        Out::Line(jsonrpc::error(Some(id), INVALID_PARAMS, message, None))
    };
    let Some(Value::Object(mut params)) = params else {
        return invalid(&id, "Invalid params");
    };
    let Some(tool) = params
        .get("name")
        .and_then(Value::as_str)
        .and_then(tools::find)
    else {
        return invalid(&id, "Unknown tool");
    };
    let arguments = match params.remove("arguments") {
        None => json!({}),
        Some(Value::Object(a)) => Value::Object(a),
        Some(_) => return invalid(&id, "The arguments are not an object"),
    };
    Out::Call(ToolCall {
        id,
        era,
        tool: tool.name,
        arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modern_params(extra: Value) -> Value {
        let mut p = object(extra);
        p.insert(
            "_meta".to_owned(),
            json!({PROTOCOL_VERSION: MODERN, CLIENT_CAPABILITIES: {}}),
        );
        Value::Object(p)
    }

    fn line(server: &mut Server, message: Value) -> Option<Out> {
        server.handle_line(message.to_string().as_bytes())
    }

    fn answer(server: &mut Server, message: Value) -> Value {
        match line(server, message) {
            Some(Out::Line(v)) => v,
            other => panic!("expected a line, got {other:?}"),
        }
    }

    fn request(id: Value, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    // m01: server/discover answers resultType, supportedVersions, the tools capability, ttlMs,
    // cacheScope, serverInfo and instructions of at most 512 characters.
    #[test]
    fn m01_discover() {
        let mut s = Server::new();
        let a = answer(
            &mut s,
            request(json!(1), "server/discover", modern_params(json!({}))),
        );
        let r = &a["result"];
        assert_eq!(r["resultType"], json!("complete"));
        assert_eq!(r["supportedVersions"], json!(["2026-07-28"]));
        assert_eq!(r["capabilities"], json!({"tools": {}}));
        assert_eq!(r["ttlMs"], json!(300_000));
        assert_eq!(r["cacheScope"], json!("public"));
        assert_eq!(
            r["_meta"]["io.modelcontextprotocol/serverInfo"],
            json!({"name": "fabric-mcp-bridge", "title": "Fabric Calendar", "version": crate::VERSION})
        );
        let instructions = r["instructions"].as_str().unwrap();
        assert!(instructions.chars().count() <= 512);
        // The two rules come first.
        assert!(instructions.starts_with(
            "An answer of pending_approval means the owner must approve the request in Fabric Calendar. Never repeat a proposal"
        ));
        assert!(instructions.contains("pending_approval"));
        assert!(instructions.contains("fabric_request_status"));
        assert!(
            instructions.find("pending_approval").unwrap()
                < instructions.find("fabric_request_status").unwrap()
        );
    }

    // m02: a request with no protocol version and no answered initialize gets -32602.
    #[test]
    fn m02_no_version_no_initialize() {
        let mut s = Server::new();
        for method in [
            "tools/list",
            "tools/call",
            "server/discover",
            "nothing/here",
        ] {
            let a = answer(&mut s, request(json!("a"), method, json!({})));
            assert_eq!(a["error"]["code"], json!(-32602), "{method}");
            assert_eq!(a["id"], json!("a"));
        }
    }

    // m03: another version gets -32022 with the supported list and the requested value; a
    // missing clientCapabilities gets -32602.
    #[test]
    fn m03_versions() {
        let mut s = Server::new();
        let a = answer(
            &mut s,
            request(
                json!(5),
                "tools/list",
                json!({"_meta": {PROTOCOL_VERSION: "2099-01-01", CLIENT_CAPABILITIES: {}}}),
            ),
        );
        assert_eq!(a["error"]["code"], json!(-32022));
        assert_eq!(
            a["error"]["data"],
            json!({"supported": ["2026-07-28"], "requested": "2099-01-01"})
        );
        let a = answer(
            &mut s,
            request(
                json!(6),
                "tools/list",
                json!({"_meta": {PROTOCOL_VERSION: MODERN}}),
            ),
        );
        assert_eq!(a["error"]["code"], json!(-32602));
        let a = answer(
            &mut s,
            request(
                json!(7),
                "tools/list",
                json!({"_meta": {PROTOCOL_VERSION: MODERN, CLIENT_CAPABILITIES: "x"}}),
            ),
        );
        assert_eq!(a["error"]["code"], json!(-32602));
    }

    // m04: initialize answers 2025-11-25 for that version and for any other.
    #[test]
    fn m04_initialize() {
        for version in ["2025-11-25", "2024-11-05", "2026-07-28", "anything"] {
            let mut s = Server::new();
            let a = answer(
                &mut s,
                request(
                    json!(1),
                    "initialize",
                    json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "x", "version": "1"}}),
                ),
            );
            let r = &a["result"];
            assert_eq!(r["protocolVersion"], json!("2025-11-25"));
            assert_eq!(r["capabilities"], json!({"tools": {}}));
            assert_eq!(r["serverInfo"]["name"], json!("fabric-mcp-bridge"));
            assert!(r["instructions"].is_string());
            assert!(r.get("resultType").is_none());
            assert!(r.get("_meta").is_none());
            // After it, legacy requests are served.
            let list = answer(&mut s, request(json!(2), "tools/list", json!({})));
            assert!(list["result"]["tools"].is_array());
        }
    }

    // m05: legacy tools/list and tools/call results have no modern member; modern ones have
    // all of them.
    #[test]
    fn m05_era_members() {
        let mut s = Server::new();
        answer(&mut s, request(json!(0), "initialize", json!({})));
        let legacy = answer(&mut s, request(json!(1), "tools/list", json!({})));
        let keys: Vec<&String> = legacy["result"].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["tools"]);
        let modern_list = answer(
            &mut s,
            request(json!(2), "tools/list", modern_params(json!({}))),
        );
        let r = &modern_list["result"];
        assert_eq!(r["resultType"], json!("complete"));
        assert_eq!(r["ttlMs"], json!(300_000));
        assert_eq!(r["cacheScope"], json!("public"));
        assert!(r["_meta"][SERVER_INFO].is_object());
        assert_eq!(r["tools"].as_array().unwrap().len(), 10);

        // tools/call, through the bridge, with no broker.
        let mut b = crate::bridge::Bridge::new(Box::new(|| Some([1u8; 16])));
        for (era, params) in [
            (Era::Legacy, json!({"name": "fabric_calendar_list"})),
            (
                Era::Modern,
                modern_params(json!({"name": "fabric_calendar_list"})),
            ),
        ] {
            let mut lines = Vec::new();
            let first = request(json!(3), "initialize", json!({}));
            lines.extend(b.handle(
                crate::bridge::Event::McpLine(first.to_string().into_bytes()),
                0,
            ));
            let call = request(json!(4), "tools/call", params);
            let effects = b.handle(
                crate::bridge::Event::McpLine(call.to_string().into_bytes()),
                0,
            );
            assert!(effects.contains(&crate::bridge::Effect::Connect));
            let effects = b.handle(
                crate::bridge::Event::Connected(crate::bridge::ConnectOutcome::Refused {
                    error: crate::mcp::render::bridge_error(
                        fabric_desktop_contracts::error::AgentFault::BrokerNotRunning,
                        "Fabric Calendar is not running.",
                    ),
                    pairing_code: None,
                    reason: "test",
                }),
                0,
            );
            let out: Vec<Value> = effects
                .iter()
                .filter_map(|e| match e {
                    crate::bridge::Effect::Stdout(l) => Some(serde_json::from_str(l).unwrap()),
                    _ => None,
                })
                .collect();
            assert_eq!(out.len(), 1);
            let r = out[0]["result"].as_object().unwrap();
            assert_eq!(r["isError"], json!(true));
            assert_eq!(r["content"].as_array().unwrap().len(), 2);
            match era {
                Era::Legacy => {
                    let mut keys: Vec<&String> = r.keys().collect();
                    keys.sort();
                    assert_eq!(keys, vec!["content", "isError"]);
                }
                Era::Modern => {
                    assert_eq!(r["resultType"], json!("complete"));
                    assert!(r["_meta"][SERVER_INFO].is_object());
                }
            }
        }
    }

    // m06: ping is answered in legacy, also before initialize, and unknown in modern.
    #[test]
    fn m06_ping() {
        let mut s = Server::new();
        let a = answer(&mut s, request(json!(1), "ping", json!({})));
        assert_eq!(a["result"], json!({}));
        let a = answer(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
        assert_eq!(a["result"], json!({}));
        let a = answer(&mut s, request(json!(3), "ping", modern_params(json!({}))));
        assert_eq!(a["error"]["code"], json!(-32601));
        // m07 (the unknown-method part): an unknown method after initialize, and in modern.
        answer(&mut s, request(json!(4), "initialize", json!({})));
        let a = answer(&mut s, request(json!(5), "resources/list", json!({})));
        assert_eq!(a["error"]["code"], json!(-32601));
        let a = answer(
            &mut s,
            request(json!(6), "resources/list", modern_params(json!({}))),
        );
        assert_eq!(a["error"]["code"], json!(-32601));
    }

    // m08: subscriptions/listen gets the acknowledgement as a notification under its id and
    // no response; a later notifications/cancelled for it produces nothing; a ninth open one
    // gets -32603; one without notifications gets -32602.
    #[test]
    fn m08_listen() {
        let mut s = Server::new();
        let ack = answer(
            &mut s,
            request(
                json!("sub-1"),
                "subscriptions/listen",
                modern_params(json!({"notifications": {}})),
            ),
        );
        assert_eq!(
            ack,
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/subscriptions/acknowledged",
                "params": {"_meta": {SUBSCRIPTION_ID: "sub-1"}, "notifications": {}},
            })
        );
        assert!(ack.get("id").is_none() && ack.get("result").is_none());
        let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "sub-1"}});
        assert_eq!(line(&mut s, cancel), None);
        for i in 0..8 {
            let a = answer(
                &mut s,
                request(
                    json!(i),
                    "subscriptions/listen",
                    modern_params(json!({"notifications": {}})),
                ),
            );
            assert_eq!(
                a["method"],
                json!("notifications/subscriptions/acknowledged")
            );
        }
        let ninth = answer(
            &mut s,
            request(
                json!(9),
                "subscriptions/listen",
                modern_params(json!({"notifications": {}})),
            ),
        );
        assert_eq!(ninth["error"]["code"], json!(-32603));
        let missing = answer(
            &mut s,
            request(json!(10), "subscriptions/listen", modern_params(json!({}))),
        );
        assert_eq!(missing["error"]["code"], json!(-32602));
        // Legacy: not a method.
        answer(&mut s, request(json!(11), "initialize", json!({})));
        let legacy = answer(
            &mut s,
            request(
                json!(12),
                "subscriptions/listen",
                json!({"notifications": {}}),
            ),
        );
        assert_eq!(legacy["error"]["code"], json!(-32601));
    }

    // m09: no notification is ever answered; tools/list with a cursor gets -32602.
    #[test]
    fn m09_notifications_and_cursor() {
        let mut s = Server::new();
        for method in [
            "notifications/initialized",
            "tools/list",
            "tools/call",
            "initialize",
            "ping",
            "server/discover",
            "anything",
        ] {
            let n = json!({"jsonrpc": "2.0", "method": method, "params": modern_params(json!({}))});
            assert_eq!(line(&mut s, n), None, "{method}");
        }
        // A cancellation for an unknown id is handed to the bridge, which ignores it.
        let c = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 99}});
        assert_eq!(
            line(&mut s, c),
            Some(Out::Cancel(RpcId::read(&json!(99)).unwrap()))
        );
        let a = answer(
            &mut s,
            request(
                json!(1),
                "tools/list",
                modern_params(json!({"cursor": "x"})),
            ),
        );
        assert_eq!(a["error"]["code"], json!(-32602));
        answer(&mut s, request(json!(2), "initialize", json!({})));
        let a = answer(
            &mut s,
            request(json!(3), "tools/list", json!({"cursor": "x"})),
        );
        assert_eq!(a["error"]["code"], json!(-32602));
    }
}
