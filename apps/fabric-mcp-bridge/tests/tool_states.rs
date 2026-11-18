#![cfg(all(unix, debug_assertions))]
//! Process tests p03 to p06 of design 10.2: request states and broker errors as the agent
//! sees them (design 3.7).

mod support;

use std::time::{Duration, Instant};

use serde_json::json;
use support::broker::{Enrollment, Reply, StandIn};
use support::client::{Bridge, result_of};
use support::scratch::Scratch;
use support::{AGENT, create_arguments, fixture_error, fixture_outcome, plain_outcome, tool_view};

// p03: a create answered pending_approval after the stand-in holds its answer; a later status
// call answers applied.
#[test]
fn p03_pending_approval_then_applied() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let pending = fixture_outcome("accept-agent-response-pending-approval");
    let request_id = pending["request"]["request_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let expires = pending["request"]["status"]["approval_expires_at"]
        .as_str()
        .expect("expiry")
        .to_owned();
    broker.script(
        Reply::outcome("fabric_calendar_propose_create", pending.clone())
            .delayed(Duration::from_millis(300)),
    );
    let applied = plain_outcome(&request_id, "applied");
    broker.script(Reply::outcome("fabric_request_status", applied.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    let started = Instant::now();
    let response = bridge.call_modern(
        json!(1),
        "fabric_calendar_propose_create",
        create_arguments("Planning block"),
    );
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "the answer was held"
    );
    let view = tool_view(result_of(&response));
    assert!(
        !view.is_error,
        "design 3.7: pending_approval is not an error: {view:?}"
    );
    assert_eq!(view.outcome("fabric_calendar_propose_create"), &pending);
    assert!(view.guidance.contains(&request_id), "{}", view.guidance);
    assert!(
        view.guidance.contains("fabric_request_status"),
        "{}",
        view.guidance
    );
    assert!(
        view.guidance.contains(&expires),
        "design 3.7: the card's expiry: {}",
        view.guidance
    );

    let response = bridge.call_modern(
        json!(2),
        "fabric_request_status",
        json!({"request_id": request_id}),
    );
    let view = tool_view(result_of(&response));
    assert!(!view.is_error, "{view:?}");
    assert_eq!(view.outcome("fabric_request_status"), &applied);
    assert!(view.guidance.contains(&request_id), "{}", view.guidance);

    let received = broker.received();
    assert_eq!(
        received.len(),
        2,
        "design A11: only the client's calls are sent"
    );
    assert_eq!(received[1].raw["params"], json!({"request_id": request_id}));
    broker.assert_clean();
}

// p04: a blocked request shows its rule.
#[test]
fn p04_blocked_shows_rule() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let blocked = fixture_outcome("accept-agent-response-blocked");
    let mut outcome = blocked.clone();
    // The fixture answers an update; the outcome itself is not tied to a tool (CR 14, 26).
    outcome["request"]["request_id"] = json!("request-22");
    broker.script(Reply::outcome(
        "fabric_calendar_propose_create",
        outcome.clone(),
    ));
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(
        json!(1),
        "fabric_calendar_propose_create",
        create_arguments("Block"),
    );
    let view = tool_view(result_of(&response));
    assert!(view.is_error, "design 3.7: blocked is an error");
    assert_eq!(view.outcome("fabric_calendar_propose_create"), &outcome);
    let rule = outcome["request"]["status"]["denial"]["rule"]
        .as_str()
        .expect("rule");
    let explanation = outcome["request"]["status"]["denial"]["explanation"]
        .as_str()
        .expect("explanation");
    assert!(view.guidance.contains(rule), "{}", view.guidance);
    assert!(
        !view.guidance.contains(explanation),
        "design A27: the explanation is data only: {}",
        view.guidance
    );
    broker.assert_clean();
}

// p05: a paused agent's call shows blocked by agent_not_paused.
#[test]
fn p05_paused_agent_blocked() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let outcome = json!({
        "request": {
            "request_id": "request-40",
            "updated_at": "2026-11-20T15:40:00.000Z",
            "status": {
                "state": "blocked",
                "denial": {"rule": "agent_not_paused", "explanation": "The owner paused this agent."},
            },
        },
        "result": null,
    });
    broker.script(Reply::outcome("fabric_calendar_list", outcome.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    let response = bridge.call_modern(json!(1), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(view.outcome("fabric_calendar_list"), &outcome);
    assert!(
        view.guidance.contains("agent_not_paused"),
        "{}",
        view.guidance
    );
    broker.assert_clean();
}

// p06: undecided, prover_unavailable and busy are relayed as design 3.7 says.
#[test]
fn p06_undecided_prover_unavailable_busy() {
    let scratch = Scratch::new();
    let broker = StandIn::start(&scratch, Enrollment::Approved);
    let undecided = fixture_outcome("accept-agent-response-undecided");
    broker.script(Reply::outcome(
        "fabric_calendar_propose_create",
        undecided.clone(),
    ));
    broker.script(Reply::error(
        "fabric_calendar_list",
        fixture_error("accept-agent-response-error-prover-unavailable"),
    ));
    let mut busy = fixture_error("accept-agent-response-error-busy-open-requests-per-agent");
    busy["detail"]["retry_after_ms"] = json!(7000);
    broker.script(Reply::error("fabric_calendar_propose_create", busy.clone()));
    let mut bridge = Bridge::start(&scratch, AGENT);

    // undecided: an error state, the outcome relayed, no reason added (CR 8).
    let response = bridge.call_modern(
        json!(1),
        "fabric_calendar_propose_create",
        create_arguments("U"),
    );
    let view = tool_view(result_of(&response));
    assert!(view.is_error, "design 3.7: undecided is an error");
    assert_eq!(view.outcome("fabric_calendar_propose_create"), &undecided);

    // prover_unavailable: relayed, not retried (design 3.7).
    let response = bridge.call_modern(json!(2), "fabric_calendar_list", json!({}));
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(
        view.error_code("fabric_calendar_list"),
        "prover_unavailable"
    );
    assert_eq!(
        view.data["error"],
        fixture_error("accept-agent-response-error-prover-unavailable")
    );

    // busy: the limit, its maximum and the wait in seconds.
    let response = bridge.call_modern(
        json!(3),
        "fabric_calendar_propose_create",
        create_arguments("B"),
    );
    let view = tool_view(result_of(&response));
    assert!(view.is_error);
    assert_eq!(view.error_code("fabric_calendar_propose_create"), "busy");
    assert_eq!(view.data["error"], busy);
    assert!(
        view.guidance.contains("open_requests_per_agent"),
        "{}",
        view.guidance
    );
    assert!(view.guidance.contains("25"), "{}", view.guidance);
    assert!(view.guidance.contains('7'), "{}", view.guidance);

    let tools: Vec<String> = broker.received().into_iter().map(|r| r.tool).collect();
    assert_eq!(
        tools,
        [
            "fabric_calendar_propose_create",
            "fabric_calendar_list",
            "fabric_calendar_propose_create"
        ],
        "design A11 and 3.7: no retry"
    );
    broker.assert_clean();
}
