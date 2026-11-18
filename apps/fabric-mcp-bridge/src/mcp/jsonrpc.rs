//! One line from the client to one JSON-RPC message, and the messages the bridge writes
//! (W06 3.4 rule 1). MCP's lines are plain JSON, read with `serde_json`; no contract value is
//! read here.

use serde_json::{Map, Value, json};

/// Not JSON.
pub const PARSE_ERROR: i64 = -32700;
/// JSON, but not a JSON-RPC request.
pub const INVALID_REQUEST: i64 = -32600;
/// No such method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The method exists but the request does not fit it.
pub const INVALID_PARAMS: i64 = -32602;
/// The bridge cannot serve the request.
pub const INTERNAL_ERROR: i64 = -32603;
/// The request's protocol version is not supported (MCP 2026-07-28).
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// A JSON-RPC id: a string or an integer, kept exactly as the client sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcId(Value);

impl RpcId {
    /// Reads an id: a string, or a number that is an integer. Anything else is no id.
    #[must_use]
    pub fn read(value: &Value) -> Option<RpcId> {
        match value {
            Value::String(_) => Some(RpcId(value.clone())),
            Value::Number(n) if n.is_i64() || n.is_u64() => Some(RpcId(value.clone())),
            _ => None,
        }
    }

    /// The id as it is written back.
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.0
    }
}

/// One line from the client.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A request, which is answered.
    Request {
        id: RpcId,
        method: String,
        params: Option<Value>,
    },
    /// A notification, which is never answered.
    Notification {
        method: String,
        params: Option<Value>,
    },
    /// A response from the client; the bridge sends no request, so it is ignored.
    Response,
    /// Not a JSON-RPC message: the error to write.
    Invalid(Value),
}

/// Reads one line, without its line feed.
#[must_use]
pub fn read_line(bytes: &[u8]) -> Message {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Message::Invalid(error(None, PARSE_ERROR, "Parse error", None));
    };
    let Value::Object(members) = value else {
        return Message::Invalid(error(None, INVALID_REQUEST, "Invalid Request", None));
    };
    read_object(&members)
}

fn read_object(members: &Map<String, Value>) -> Message {
    let id = match members.get("id") {
        None => None,
        Some(raw) => match RpcId::read(raw) {
            Some(id) => Some(id),
            None => {
                return Message::Invalid(error(None, INVALID_REQUEST, "Invalid Request", None));
            }
        },
    };
    let invalid =
        |id: Option<&RpcId>| Message::Invalid(error(id, INVALID_REQUEST, "Invalid Request", None));
    if members.get("jsonrpc") != Some(&Value::String("2.0".to_owned())) {
        return invalid(id.as_ref());
    }
    match members.get("method") {
        Some(Value::String(method)) => {
            let params = members.get("params").cloned();
            match id {
                Some(id) => Message::Request {
                    id,
                    method: method.clone(),
                    params,
                },
                None => Message::Notification {
                    method: method.clone(),
                    params,
                },
            }
        }
        Some(_) => invalid(id.as_ref()),
        None => {
            if id.is_some() && (members.contains_key("result") || members.contains_key("error")) {
                Message::Response
            } else {
                invalid(id.as_ref())
            }
        }
    }
}

/// A result for the request `id`.
#[must_use]
pub fn result(id: &RpcId, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.value(), "result": result})
}

/// An error. Without an id the message has no `id` member: MCP's schema has no null id
/// (W06 3.4 rule 1).
#[must_use]
pub fn error(id: Option<&RpcId>, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".to_owned(), json!(code));
    error.insert("message".to_owned(), json!(message));
    if let Some(data) = data {
        error.insert("data".to_owned(), data);
    }
    let mut out = Map::new();
    out.insert("jsonrpc".to_owned(), json!("2.0"));
    if let Some(id) = id {
        out.insert("id".to_owned(), id.value().clone());
    }
    out.insert("error".to_owned(), Value::Object(error));
    Value::Object(out)
}

/// A notification from the bridge.
#[must_use]
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// One message as one line, without its line feed. `serde_json` escapes every line feed
/// inside a string, so the line holds none (W06 2.4).
#[must_use]
pub fn to_line(message: &Value) -> String {
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(line: &str) -> Value {
        match read_line(line.as_bytes()) {
            Message::Invalid(v) => v,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    // m07: bad JSON -32700 and an array -32600, both without an id member; a bad id -32600;
    // an unknown method -32601 (see also mcp/server); ids come back as sent.
    #[test]
    fn m07_lines_and_ids() {
        let bad = invalid("{not json");
        assert_eq!(bad["error"]["code"], json!(PARSE_ERROR));
        assert!(bad.get("id").is_none());

        let array = invalid(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
        assert_eq!(array["error"]["code"], json!(INVALID_REQUEST));
        assert!(array.get("id").is_none());

        for bad_id in [r#"null"#, r#"1.5"#, r#"true"#, r#"{"a":1}"#, r#"[1]"#] {
            let line = format!(r#"{{"jsonrpc":"2.0","id":{bad_id},"method":"ping"}}"#);
            let e = invalid(&line);
            assert_eq!(e["error"]["code"], json!(INVALID_REQUEST), "{bad_id}");
            assert!(e.get("id").is_none());
        }

        let wrong_version = invalid(r#"{"jsonrpc":"1.0","id":"x","method":"ping"}"#);
        assert_eq!(wrong_version["error"]["code"], json!(INVALID_REQUEST));
        assert_eq!(wrong_version["id"], json!("x"));

        for (raw, expected) in [
            (r#""abc""#, json!("abc")),
            ("7", json!(7)),
            ("-3", json!(-3)),
            ("18446744073709551615", json!(18_446_744_073_709_551_615u64)),
        ] {
            let line = format!(r#"{{"jsonrpc":"2.0","id":{raw},"method":"x"}}"#);
            let Message::Request { id, .. } = read_line(line.as_bytes()) else {
                panic!("not a request");
            };
            assert_eq!(id.value(), &expected);
            let answer = to_line(&result(&id, json!({})));
            assert!(answer.contains(&format!(r#""id":{raw}"#)), "{answer}");
        }

        assert_eq!(
            read_line(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
            Message::Response
        );
        assert!(matches!(
            read_line(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Message::Notification { .. }
        ));
    }
}
