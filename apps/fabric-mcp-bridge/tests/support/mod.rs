//! Support for the process tests of design 10.2: a scratch directory, the MCP test client and
//! the stand-in broker. Each test file uses a part of it.
#![allow(dead_code)]

pub mod broker;
pub mod client;
pub mod scratch;

use std::path::PathBuf;
use std::process::Command;

use fabric_desktop_contracts::{Entry, PROTOCOL, canonical, decode};
use serde_json::{Map, Value, json};

/// The enrollment id every test uses (design 2.2: a contract `Id`).
pub const AGENT: &str = "agent-0123456789abcdef0123456789abcdef";

/// The directory of the contract's message fixtures.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/desktop/v1/fixtures/messages")
}

/// The `value` of a golden message fixture, by its file name without `.json`.
pub fn fixture_value(name: &str) -> Value {
    let path = fixtures_dir().join(format!("{name}.json"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let file: Value = serde_json::from_str(&text).expect("a fixture is JSON");
    assert_eq!(
        file["expect"]["verdict"],
        json!("accept"),
        "{name} is not an accepted fixture"
    );
    file["value"].clone()
}

/// The `tool_outcome` of an accepted agent response fixture.
pub fn fixture_outcome(name: &str) -> Value {
    fixture_value(name)["result"].clone()
}

/// The agent error of an accepted agent response fixture.
pub fn fixture_error(name: &str) -> Value {
    fixture_value(name)["error"].clone()
}

/// The `params` of an accepted agent request fixture.
pub fn fixture_params(name: &str) -> Value {
    fixture_value(name)["params"].clone()
}

/// A tool outcome with a plain state and no result.
pub fn plain_outcome(request_id: &str, state: &str) -> Value {
    json!({
        "request": {
            "request_id": request_id,
            "updated_at": "2026-11-20T15:05:00.000Z",
            "status": {"state": state},
        },
        "result": null,
    })
}

/// Arguments for `fabric_calendar_propose_create` without `description` and `location`, as a
/// client that follows the emitted schema writes them (design 3.5 rule 2).
pub fn create_arguments(title: &str) -> Value {
    json!({
        "account_id": "acct-1",
        "calendar_id": "cal-drafts",
        "event": {
            "title": title,
            "start": {"date_time": "2026-11-20T10:00:00-06:00", "zone": "America/Chicago"},
            "end": {"date_time": "2026-11-20T11:00:00-06:00", "zone": "America/Chicago"},
        },
        "task": "Block time to plan next week.",
    })
}

/// What a tool result shows: `isError`, the guidance block and the data block (design 3.7).
#[derive(Debug)]
pub struct ToolView {
    pub is_error: bool,
    pub guidance: String,
    pub data_text: String,
    pub data: Value,
}

/// Reads a `tools/call` result as design A8 and 3.7 describe it: exactly two text blocks, the
/// second the contract's value as canonical JSON.
pub fn tool_view(result: &Map<String, Value>) -> ToolView {
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no content array: {result:?}"));
    assert_eq!(content.len(), 2, "a tool result has two blocks: {result:?}");
    for block in content {
        assert_eq!(block["type"], json!("text"), "{block}");
        assert!(block["text"].is_string(), "{block}");
    }
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("no isError: {result:?}"));
    assert!(
        !result.contains_key("structuredContent"),
        "design A8: no structuredContent"
    );
    let guidance = content[0]["text"].as_str().expect("text").to_owned();
    let data_text = content[1]["text"].as_str().expect("text").to_owned();
    let data: Value = serde_json::from_str(&data_text)
        .unwrap_or_else(|e| panic!("the second block is not JSON ({e}): {data_text}"));
    assert_eq!(
        canonical(&data),
        data_text,
        "the second block is not canonical JSON"
    );
    ToolView {
        is_error,
        guidance,
        data_text,
        data,
    }
}

impl ToolView {
    /// The second block as a contract error: checks it with the contract's reader and returns
    /// the code.
    pub fn error_code(&self, tool: &str) -> String {
        let error = self
            .data
            .get("error")
            .unwrap_or_else(|| panic!("the second block is not an error: {}", self.data_text));
        assert_eq!(
            self.data.as_object().map(Map::len),
            Some(1),
            "{}",
            self.data_text
        );
        let frame = json!({"protocol": PROTOCOL, "id": "call-x", "method": tool, "error": error});
        decode(Entry::AgentResponse, canonical(&frame).as_bytes())
            .unwrap_or_else(|r| panic!("the error is not a contract agent error ({r}): {error}"));
        error["code"].as_str().expect("code").to_owned()
    }

    /// The second block as a contract `tool_outcome`, checked with the contract's reader.
    pub fn outcome(&self, tool: &str) -> &Value {
        let frame =
            json!({"protocol": PROTOCOL, "id": "call-x", "method": tool, "result": self.data});
        decode(Entry::AgentResponse, canonical(&frame).as_bytes()).unwrap_or_else(|r| {
            panic!(
                "the second block is not a contract tool_outcome ({r}): {}",
                self.data_text
            )
        });
        &self.data
    }
}

/// Runs `fabric-mcp-bridge code --enrollment <id> --key-dir <dir>` and returns its one line.
pub fn code_command(program: &std::path::Path, key_dir: &std::path::Path) -> String {
    let output = Command::new(program)
        .args(["code", "--enrollment", AGENT, "--key-dir"])
        .arg(key_dir)
        .output()
        .expect("run the code command");
    assert!(output.status.success(), "code failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("UTF-8")
        .trim_end_matches('\n')
        .to_owned()
}

/// Every pairing code (`XXXX-XXXX-XXXX`, capital hexadecimal digits) in `text`.
pub fn pairing_codes(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let hex = |b: u8| b.is_ascii_digit() || (b'A'..=b'F').contains(&b);
    let mut found = Vec::new();
    if bytes.len() < 14 {
        return found;
    }
    for start in 0..=bytes.len() - 14 {
        let w = &bytes[start..start + 14];
        let shape = w
            .iter()
            .enumerate()
            .all(|(i, &b)| if i == 4 || i == 9 { b == b'-' } else { hex(b) });
        let before = start.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(start + 14).copied();
        let bounded = !before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !after.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-');
        if shape && bounded {
            found.push(String::from_utf8_lossy(w).into_owned());
        }
    }
    found
}
