#![cfg(all(unix, debug_assertions))]
//! Process tests p07 to p10 and p15 of design 10.2: the socket, the handshake, enrollment and
//! sessions.

mod support;

use std::path::Path;
use std::thread;
use std::time::Duration;

use serde_json::json;
use support::broker::{Enrollment, Reply, StandIn, pairing_code};
use support::client::{Bridge, bridge_exe, result_of};
use support::scratch::{Scratch, set_mode};
use support::{AGENT, code_command, fixture_outcome, pairing_codes, tool_view};

// p07: with no socket, tools/list works and a call answers broker_not_running; after the
// stand-in starts, the next call works with the same bridge process.
#[test]
fn p07_no_broker_then_broker() {
    let scratch = Scratch::new();
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.tools_list_modern(json!(1));
    assert_eq!(
        result_of(&response)["tools"].as_array().map(Vec::len),
        Some(10)
    );

    let response = bridge.call_modern(json!(2), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(
        view.error_code("fabric_calendar_list"),
        "broker_not_running"
    );

    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let response = bridge.call_modern(json!(3), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(!view.is_error, "{view:?}");
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);
    assert!(
        bridge.wait_exit(Duration::ZERO).is_none(),
        "design A5: the bridge keeps running"
    );
    broker.assert_clean();
}

// p08: a key the stand-in refuses gets not_permitted with the bridge's code; nothing but the
// handshake was sent.
#[test]
fn p08_refused_key_not_permitted() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::NotPermitted);
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(view.error_code("fabric_calendar_list"), "not_permitted");
    let codes = pairing_codes(&view.guidance);
    assert_eq!(codes.len(), 1, "one pairing code in: {}", view.guidance);

    let connections = broker.connections();
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0].signature_ok, Some(true));
    let key = connections[0].hello.as_ref().expect("hello")["public_key"]
        .as_str()
        .expect("key")
        .to_owned();
    assert_eq!(
        codes[0],
        pairing_code(&key),
        "design 5.3: the code of the proven key"
    );
    assert_eq!(
        codes[0],
        code_command(Path::new(bridge_exe()), &scratch.key_dir)
    );

    // Give a stray frame time to arrive, then check that none did.
    thread::sleep(Duration::from_millis(300));
    let connections = broker.connections();
    assert!(
        connections[0].lines_after_handshake.is_empty(),
        "sent after the handshake: {:?}",
        connections[0].lines_after_handshake
    );
    assert!(broker.received().is_empty());
}

// p09: a pending agent answers with the pairing code, the same as `code` prints; after the
// stand-in approves, the next call gets a session.
#[test]
fn p09_pending_then_approved() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Pending);
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error, "design 3.7: the pending answer is an error");
    assert_eq!(view.error_code("fabric_calendar_list"), "not_permitted");
    let codes = pairing_codes(&view.guidance);
    assert_eq!(codes.len(), 1, "one pairing code in: {}", view.guidance);
    let printed = code_command(Path::new(bridge_exe()), &scratch.key_dir);
    assert_eq!(codes[0], printed);
    let key = broker.connections()[0].hello.as_ref().expect("hello")["public_key"]
        .as_str()
        .expect("key")
        .to_owned();
    assert_eq!(printed, pairing_code(&key));

    // Design 4.4: on pending the bridge closes the connection.
    broker.wait_until(
        "the pending connection closed",
        Duration::from_secs(5),
        |c, _| c.first().is_some_and(|c| c.closed),
    );
    assert!(broker.connections()[0].lines_after_handshake.is_empty());

    broker.set_enrollment(Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let response = bridge.call_modern(json!(2), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(!view.is_error, "{view:?}");
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);
    let connections = broker.connections();
    assert_eq!(connections.len(), 2, "a new connection");
    let second_key = &connections[1].hello.as_ref().expect("hello")["public_key"];
    assert_eq!(second_key, &json!(key), "the same key is proven again");
    broker.assert_clean();
}

// p10, first half: one connection per run; end of input closes it.
#[test]
fn p10_one_connection_end_of_input() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    // Design A15: no connection before the first tool call.
    let _ = bridge.tools_list_modern(json!(1));
    thread::sleep(Duration::from_millis(200));
    assert!(
        broker.connections().is_empty(),
        "connected before a tool call"
    );

    for id in [2, 3] {
        let response = bridge.call_modern(json!(id), "fabric_calendar_list", json!({}));
        assert!(!tool_view(result_of(&response)).is_error);
    }
    assert_eq!(
        broker.connections().len(),
        1,
        "one connection for both calls"
    );

    bridge.close_stdin();
    let status = bridge
        .wait_exit(Duration::from_secs(10))
        .expect("the bridge exits");
    assert_eq!(status.code(), Some(0), "design 2.3: exit 0 at end of input");
    broker.wait_until("the connection closed", Duration::from_secs(5), |c, _| {
        c.first().is_some_and(|c| c.closed)
    });
    broker.assert_clean();
}

// p10, second half: a connection the stand-in closes is replaced at the next call.
#[test]
fn p10_closed_connection_replaced() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()).then_close());
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    assert!(!tool_view(result_of(&response)).is_error);
    broker.wait_until("the stand-in's close", Duration::from_secs(5), |c, _| {
        c.first().is_some_and(|c| c.closed)
    });
    // Let the bridge read the end of the connection before the next call.
    thread::sleep(Duration::from_millis(200));

    let response = bridge.call_modern(json!(2), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(!view.is_error, "{view:?}");
    assert_eq!(broker.connections().len(), 2, "a new connection");
    broker.assert_clean();
}

/// Checks that a call answers broker_not_running and the stand-in sees no connection.
fn assert_refused_before_connecting(scratch: &Scratch, broker: &StandIn) {
    let mut bridge = Bridge::start(scratch, AGENT);
    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(
        view.error_code("fabric_calendar_list"),
        "broker_not_running"
    );
    thread::sleep(Duration::from_millis(200));
    assert!(broker.connections().is_empty(), "design 4.2: no connection");
}

// p15, first half: a socket of mode 0666 is refused.
#[test]
fn p15_socket_mode_0666_refused() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    set_mode(&scratch.socket, 0o666);
    assert_refused_before_connecting(&scratch, &broker);
}

// p15, second half: a directory of mode 0755 is refused.
#[test]
fn p15_directory_mode_0755_refused() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    set_mode(&scratch.run_dir, 0o755);
    assert_refused_before_connecting(&scratch, &broker);
}
