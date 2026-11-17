#![cfg(all(unix, debug_assertions))]
//! Process tests p11 to p13 of design 10.2: paths with spaces, what stderr and stdout may
//! hold, and that a result is shown once.

mod support;

use std::fs;
use std::time::Duration;

use serde_json::{Value, json};
use support::broker::{Enrollment, Reply, StandIn};
use support::client::{Bridge, result_of};
use support::scratch::{Scratch, private_dir};
use support::{
    AGENT, code_command, create_arguments, fixture_error, fixture_outcome, fixture_params,
    plain_outcome, tool_view,
};

/// A string that appears nowhere unless a test put it there.
const MARKER: &str = "QZXMARK-5d1e-77";

/// An events outcome whose first event's title holds `MARKER`.
fn events_with_marker() -> Value {
    let mut outcome = fixture_outcome("accept-agent-response-events");
    outcome["result"]["events"][0]["title"] = json!(format!("Meeting {MARKER}"));
    outcome
}

fn read_arguments() -> Value {
    fixture_params("accept-agent-request-read-few-fields")
}

// p11: the socket's directory, the key directory and the bridge's own path hold spaces.
#[test]
fn p11_paths_with_spaces() {
    let scratch = Scratch::spaced();
    let bin_dir = scratch.root.join("b in");
    private_dir(&bin_dir);
    let program = bin_dir.join("fabric-mcp-bridge");
    fs::copy(support::client::bridge_exe(), &program).expect("copy the bridge");
    for path in [&scratch.run_dir, &scratch.key_dir, &program] {
        assert!(path.to_string_lossy().contains(' '), "{}", path.display());
    }

    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start_program(&program, &scratch, AGENT);
    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(!view.is_error, "{view:?}; stderr: {}", bridge.stderr_text());
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);

    // The key in the spaced key directory is the one the code command reads.
    let key = broker.connections()[0].hello.as_ref().expect("hello")["public_key"]
        .as_str()
        .expect("key")
        .to_owned();
    assert_eq!(
        code_command(&program, &scratch.key_dir),
        support::broker::pairing_code(&key)
    );
    broker.assert_clean();
}

// p12: stdout holds only JSON-RPC lines; stderr never holds a marker placed in an argument, in
// a refused argument, in a result and in a broker message.
#[test]
fn p12_no_content_in_logs() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    broker.script(Reply::outcome(
        "fabric_calendar_propose_create",
        fixture_outcome("accept-agent-response-pending-approval"),
    ));
    broker.script(Reply::outcome("fabric_calendar_read", events_with_marker()));
    let mut internal = fixture_error("accept-agent-response-error-internal");
    internal["message"] = json!(format!("The broker failed: {MARKER}"));
    broker.script(Reply::error("fabric_calendar_list", internal));
    let mut bridge = Bridge::start(&scratch, AGENT);

    // (a) A valid argument.
    let response = bridge.call_modern(
        json!(1),
        "fabric_calendar_propose_create",
        create_arguments(&format!("Title {MARKER}")),
    );
    assert!(!tool_view(result_of(&response)).is_error);
    assert!(
        broker.received()[0].raw.to_string().contains(MARKER),
        "the marker reached the stand-in"
    );

    // (b) An argument the contract refuses: an id with a space (design A7, 3.6 step 4).
    let mut refused = create_arguments("Fine title");
    refused["account_id"] = json!(format!("acct {MARKER}"));
    let response = bridge.call_modern(json!(2), "fabric_calendar_propose_create", refused);
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(
        view.error_code("fabric_calendar_propose_create"),
        "invalid_params"
    );
    assert_eq!(view.data["error"]["detail"]["pointer"], json!("/params"));

    // (c) A result.
    let response = bridge.call_modern(json!(3), "fabric_calendar_read", read_arguments());
    let view = tool_view(result_of(&response));
    assert!(!view.is_error);
    assert!(view.data_text.contains(MARKER));
    assert!(
        !view.guidance.contains(MARKER),
        "design A27: {}",
        view.guidance
    );

    // (d) The message of a broker error.
    let response = bridge.call_modern(json!(4), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(view.error_code("fabric_calendar_list"), "internal");
    assert!(view.data_text.contains(MARKER));
    assert!(
        !view.guidance.contains(MARKER),
        "design A27: {}",
        view.guidance
    );

    // Only the three valid calls reached the stand-in.
    let tools: Vec<String> = broker.received().into_iter().map(|r| r.tool).collect();
    assert_eq!(
        tools,
        [
            "fabric_calendar_propose_create",
            "fabric_calendar_read",
            "fabric_calendar_list"
        ]
    );

    // End the process so that stderr is complete; every stdout line was checked on reading.
    bridge.close_stdin();
    let _ = bridge.drain_for(Duration::from_millis(300));
    assert!(
        bridge.wait_exit(Duration::from_secs(10)).is_some(),
        "the bridge exits"
    );
    let stderr = bridge.stderr_text();
    assert!(!stderr.is_empty(), "the bridge logs something (design 2.4)");
    assert!(
        !stderr.contains(MARKER),
        "design A25: the marker is in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("Title "),
        "an argument is in stderr:\n{stderr}"
    );
    assert_eq!(
        bridge.seen().len(),
        4,
        "one message per request: {:?}",
        bridge.seen()
    );
    broker.assert_clean();
}

// p13: a read's result is shown once; a later status call, which the stand-in answers without
// a result, shows none of it.
#[test]
fn p13_result_shown_once() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let events = events_with_marker();
    let request_id = events["request"]["request_id"]
        .as_str()
        .expect("id")
        .to_owned();
    broker.script(Reply::outcome("fabric_calendar_read", events));
    broker.script(Reply::outcome(
        "fabric_request_status",
        plain_outcome(&request_id, "reconciled"),
    ));
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(json!(1), "fabric_calendar_read", read_arguments());
    let view = tool_view(result_of(&response));
    assert!(view.data_text.contains(MARKER));

    let response = bridge.call_modern(
        json!(2),
        "fabric_request_status",
        json!({"request_id": request_id}),
    );
    assert!(
        !response.to_string().contains(MARKER),
        "design A12: the status answer shows the earlier result: {response}"
    );
    let view = tool_view(result_of(&response));
    assert_eq!(view.data["result"], Value::Null);
    broker.assert_clean();
}
