#![cfg(all(unix, debug_assertions))]
//! Process tests p01 and p02 of design 10.2: the two MCP revisions, end to end.

mod support;

use fabric_desktop_contracts::common::ToolName;
use serde_json::{Map, Value, json};
use support::broker::{Enrollment, Reply, StandIn};
use support::client::{
    Bridge, LEGACY, MODERN, assert_legacy_result, assert_modern_result, error_of, result_of,
};
use support::scratch::Scratch;
use support::{AGENT, fixture_outcome, tool_view};

/// Checks the ten tools: names in the contract's order, a description, an object
/// `inputSchema` (MCP "Tools": "MUST be a valid JSON Schema object").
fn assert_ten_tools(result: &Map<String, Value>) {
    let tools = result["tools"].as_array().expect("tools is an array");
    assert_eq!(tools.len(), 10, "{tools:?}");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    let expected: Vec<&str> = ToolName::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(names, expected);
    for tool in tools {
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{tool}"
        );
        assert_eq!(tool["inputSchema"]["type"], json!("object"), "{tool}");
    }
}

// p01: modern: server/discover, tools/list, a list call, its result.
#[test]
fn p01_modern_discover_list_and_call() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    // server/discover (MCP 2026-07-28 "Discovery"; design 3.3).
    let response = bridge.discover(json!("discover-1"));
    let result = result_of(&response);
    assert_modern_result(result);
    assert_eq!(result["supportedVersions"], json!([MODERN]));
    assert!(result["capabilities"]["tools"].is_object(), "{result:?}");
    assert!(result["ttlMs"].is_u64(), "{result:?}");
    assert!(result["cacheScope"].is_string(), "{result:?}");
    assert_eq!(result["ttlMs"], json!(300_000));
    assert_eq!(result["cacheScope"], json!("public"));
    let instructions = result["instructions"].as_str().expect("instructions");
    assert!(
        instructions.chars().count() <= 512,
        "design 3.3: at most 512 characters"
    );

    // tools/list.
    let response = bridge.tools_list_modern(json!(2));
    let result = result_of(&response);
    assert_modern_result(result);
    assert!(result["ttlMs"].is_u64(), "{result:?}");
    assert!(result["cacheScope"].is_string(), "{result:?}");
    assert_ten_tools(result);

    // A list call.
    let response = bridge.call_modern(json!(3), "fabric_calendar_list", json!({}));
    let result = result_of(&response);
    assert_modern_result(result);
    let view = tool_view(result);
    assert!(!view.is_error, "{view:?}");
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);

    // The stand-in saw the right hello and the call.
    let connections = broker.connections();
    assert_eq!(connections.len(), 1);
    let hello = connections[0].hello.as_ref().expect("hello");
    assert_eq!(hello["enrollment_id"], json!(AGENT));
    assert_eq!(connections[0].signature_ok, Some(true));
    let received = broker.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].tool, "fabric_calendar_list");
    assert_eq!(received[0].raw["params"], json!({}));
    assert!(received[0].id.starts_with("call-"), "{}", received[0].id);
    broker.assert_clean();
}

// p01, the version rule: another version gets -32022 with the supported list (MCP
// "Versioning": "it MUST respond with an UnsupportedProtocolVersionError").
#[test]
fn p01_modern_unsupported_version() {
    let scratch = Scratch::new();
    let mut bridge = Bridge::start(&scratch, AGENT);
    bridge.send(&json!({
        "jsonrpc": "2.0", "id": 7, "method": "server/discover",
        "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "1900-01-01",
            "io.modelcontextprotocol/clientCapabilities": {},
        }},
    }));
    let response = bridge.wait_response(&json!(7));
    let error = error_of(&response);
    assert_eq!(error["code"], json!(-32022));
    assert_eq!(error["data"]["requested"], json!("1900-01-01"));
    let supported = error["data"]["supported"].as_array().expect("supported");
    assert!(supported.contains(&json!(MODERN)), "{error:?}");
}

// p02: legacy: initialize, notifications/initialized, tools/list, a call.
#[test]
fn p02_legacy_initialize_list_and_call() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    // Design 3.4 rule 4: a legacy request before an answered initialize gets -32602.
    let response = bridge.tools_list_legacy(json!(0));
    assert_eq!(error_of(&response)["code"], json!(-32602));

    let response = bridge.initialize(json!(1));
    let result = result_of(&response);
    assert_eq!(result["protocolVersion"], json!(LEGACY));
    assert!(result["capabilities"]["tools"].is_object(), "{result:?}");
    assert!(result["serverInfo"]["name"].is_string(), "{result:?}");
    assert!(result["serverInfo"]["version"].is_string(), "{result:?}");
    assert_legacy_result(result);

    let response = bridge.tools_list_legacy(json!("list"));
    let result = result_of(&response);
    assert_legacy_result(result);
    assert_ten_tools(result);

    let response = bridge.call_legacy(json!(3), "fabric_calendar_list", json!({}));
    let result = result_of(&response);
    assert_legacy_result(result);
    let view = tool_view(result);
    assert!(!view.is_error, "{view:?}");
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);

    // notifications/initialized is never answered: every message seen answers a request.
    for message in bridge.seen() {
        assert!(message.get("id").is_some(), "unexpected message {message}");
    }
    broker.assert_clean();
}
