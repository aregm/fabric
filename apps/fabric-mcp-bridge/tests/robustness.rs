#![cfg(all(unix, debug_assertions))]
//! Process tests p14 and p16 to p18 of design 10.2: bounds and malformed input, answers out of
//! order, cancellation, and the arguments the stand-in receives.

mod support;

use std::time::Duration;

use fabric_desktop_contracts::Frame;
use fabric_desktop_contracts::agent::{Action, AgentCall, ResponseTarget};
use serde_json::{Value, json};
use support::broker::{Answer, Enrollment, Reply, StandIn};
use support::client::{Bridge, error_of, modern_meta, result_of};
use support::scratch::Scratch;
use support::{AGENT, create_arguments, fixture_outcome, fixture_params, tool_view};

// p14, first half: a line over the bound from the client gets a JSON-RPC error and the bridge
// keeps serving (design 2.5: -32600 without an id).
#[test]
fn p14_client_line_over_bound() {
    let scratch = Scratch::new();
    let mut bridge = Bridge::start(&scratch, AGENT);
    let pad = "a".repeat(2 * 1024 * 1024 + 16);
    let line = serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": 99, "method": "tools/list",
        "params": {"_meta": modern_meta(), "pad": pad},
    }))
    .expect("serialize");
    assert!(line.len() > 2 * 1024 * 1024);
    bridge.send_raw(&line);
    let error = bridge.wait_for(
        "the error for the long line",
        Duration::from_secs(10),
        |m| m.get("error").is_some(),
    );
    assert_eq!(error_of(&error)["code"], json!(-32600));
    assert!(error.get("id").is_none(), "design 2.5: no id: {error}");

    let response = bridge.tools_list_modern(json!(100));
    assert_eq!(
        result_of(&response)["tools"].as_array().map(Vec::len),
        Some(10)
    );
    let late = bridge.drain_for(Duration::from_millis(300));
    assert!(
        bridge
            .seen()
            .iter()
            .all(|m| m.get("id") != Some(&json!(99))),
        "the long line was answered as a request: {late:?}"
    );
}

// p14, second half: a malformed frame from the stand-in makes the call in flight an error; the
// connection is dropped and the next call connects again (design 2.5, 3.7).
#[test]
fn p14_malformed_frame_from_broker() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    broker.script(Reply::new("fabric_calendar_list", Answer::Malformed));
    let outcome = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(
        view.error_code("fabric_calendar_list"),
        "broker_not_running",
        "design 3.7: a connection that closed with the call in flight"
    );
    broker.wait_until(
        "the bridge dropped the connection",
        Duration::from_secs(5),
        |c, _| c.first().is_some_and(|c| c.closed),
    );

    let response = bridge.call_modern(json!(2), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(!view.is_error, "{view:?}");
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);
    assert_eq!(broker.connections().len(), 2);
    broker.assert_clean();
}

// p16: two calls in flight on one connection are both answered, the second answer first.
#[test]
fn p16_answers_out_of_order() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let pending = fixture_outcome("accept-agent-response-pending-approval");
    let calendars = fixture_outcome("accept-agent-response-calendars");
    broker.script(
        Reply::outcome("fabric_calendar_propose_create", pending.clone())
            .after("fabric_calendar_list"),
    );
    broker.script(Reply::outcome("fabric_calendar_list", calendars.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    bridge.send_call_modern(
        &json!("first"),
        "fabric_calendar_propose_create",
        create_arguments("A"),
    );
    broker.wait_until("the first call", Duration::from_secs(10), |_, r| {
        r.len() == 1
    });
    bridge.send_call_modern(&json!("second"), "fabric_calendar_list", json!({}));

    let second = bridge.wait_response(&json!("second"));
    let first = bridge.wait_response(&json!("first"));
    let order: Vec<&Value> = bridge.seen().iter().filter_map(|m| m.get("id")).collect();
    assert_eq!(
        order,
        [&json!("second"), &json!("first")],
        "the second answer came first"
    );
    assert_eq!(
        tool_view(result_of(&first)).outcome("fabric_calendar_propose_create"),
        &pending
    );
    assert_eq!(
        tool_view(result_of(&second)).outcome("fabric_calendar_list"),
        &calendars
    );
    assert_eq!(broker.connections().len(), 1, "one connection");
    broker.assert_clean();
}

// p17: a cancelled call gets no response (MCP "Cancellation"; design 3.9).
#[test]
fn p17_cancelled_call_gets_no_response() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    broker.script(Reply::new("fabric_calendar_propose_create", Answer::Never));
    let calendars = fixture_outcome("accept-agent-response-calendars");
    broker.script(Reply::outcome("fabric_calendar_list", calendars.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    bridge.send_call_modern(
        &json!(1),
        "fabric_calendar_propose_create",
        create_arguments("C"),
    );
    broker.wait_until("the call", Duration::from_secs(10), |_, r| r.len() == 1);
    bridge.cancel(&json!(1));
    let response = bridge.call_modern(json!(2), "fabric_calendar_list", json!({}));
    assert_eq!(
        tool_view(result_of(&response)).outcome("fabric_calendar_list"),
        &calendars
    );
    let _ = bridge.drain_for(Duration::from_secs(1));
    assert!(
        bridge.seen().iter().all(|m| m.get("id") != Some(&json!(1))),
        "a response for the cancelled call: {:?}",
        bridge.seen()
    );
    broker.assert_clean();
}

// p17, the same for a subscription: acknowledged by a notification under its id, and nothing
// after it is cancelled (MCP "Subscriptions"; design 3.3, 3.9).
#[test]
fn p17_cancelled_subscription() {
    let scratch = Scratch::new();
    let mut bridge = Bridge::start(&scratch, AGENT);
    bridge.send_listen(&json!("sub-1"));
    let ack = bridge.wait_for("the acknowledgement", Duration::from_secs(10), |m| {
        m.get("method") == Some(&json!("notifications/subscriptions/acknowledged"))
    });
    assert_eq!(
        ack["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        json!("sub-1")
    );
    bridge.cancel(&json!("sub-1"));
    let _ = bridge.tools_list_modern(json!(2));
    let _ = bridge.drain_for(Duration::from_secs(1));
    assert!(
        bridge
            .seen()
            .iter()
            .all(|m| m.get("id") != Some(&json!("sub-1"))),
        "a response for the cancelled subscription: {:?}",
        bridge.seen()
    );
}

// p18, first half: an omitted description and location reach the stand-in as null.
#[test]
fn p18_omitted_members_arrive_as_null() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    broker.script(Reply::outcome(
        "fabric_calendar_propose_create",
        fixture_outcome("accept-agent-response-pending-approval"),
    ));
    let mut bridge = Bridge::start(&scratch, AGENT);
    let arguments = create_arguments("Planning block");
    assert!(arguments["event"].get("description").is_none());
    let response = bridge.call_modern(
        json!(1),
        "fabric_calendar_propose_create",
        arguments.clone(),
    );
    assert!(!tool_view(result_of(&response)).is_error);

    let received = broker.received();
    let params = &received[0].raw["params"];
    let mut expected = arguments;
    expected["event"]["description"] = Value::Null;
    expected["event"]["location"] = Value::Null;
    assert_eq!(params, &expected, "design 3.6: only the nulls are added");
    let Frame::AgentRequest(request) = &received[0].frame else {
        panic!("not an agent request");
    };
    let AgentCall::Action(Action::ProposeCreate(create)) = &request.call else {
        panic!("not a create: {request:?}");
    };
    assert!(create.event.description.is_none() && create.event.location.is_none());
    broker.assert_clean();
}

// p18, second half: a response target written as one object reaches the stand-in as the
// contract's union, for both kinds.
#[test]
fn p18_response_target_as_union() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    for _ in 0..2 {
        broker.script(Reply::outcome(
            "fabric_calendar_propose_response",
            fixture_outcome("accept-agent-response-pending-approval"),
        ));
    }
    let mut bridge = Bridge::start(&scratch, AGENT);

    let cases = [
        fixture_params("accept-agent-request-propose-response-event"),
        fixture_params("accept-agent-request-propose-response-occurrence"),
    ];
    for (i, arguments) in cases.iter().enumerate() {
        let response = bridge.call_modern(
            json!(i),
            "fabric_calendar_propose_response",
            arguments.clone(),
        );
        let view = tool_view(result_of(&response));
        assert!(!view.is_error, "{view:?}");
    }
    let received = broker.received();
    assert_eq!(received.len(), 2);
    for (got, sent) in received.iter().zip(&cases) {
        assert_eq!(got.raw["params"], *sent, "the arguments arrive unchanged");
        let Frame::AgentRequest(request) = &got.frame else {
            panic!("not an agent request");
        };
        let AgentCall::Action(Action::ProposeResponse(args)) = &request.call else {
            panic!("not a response proposal: {request:?}");
        };
        match (&args.target, sent["target"]["kind"].as_str()) {
            (ResponseTarget::Event(_), Some("event"))
            | (ResponseTarget::Occurrence(_), Some("occurrence")) => {}
            other => panic!("the wrong branch: {other:?}"),
        }
    }
    broker.assert_clean();
}
