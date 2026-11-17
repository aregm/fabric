//! A contract answer to a tool result (W06 3.7, A8, A27).
//!
//! A tool result has two text blocks. The first is the bridge's fixed wording with only values
//! that passed the contract's grammar: a request id, a timestamp, a rule's name, a failure
//! code, a limit's name, numbers, the pairing code. The second is the contract's value as
//! canonical JSON: the `tool_outcome`, or `{"error": <agent error>}`. A rule's explanation, an
//! error's message and every result appear only in the second block.

use fabric_desktop_contracts::agent::{AgentLifecycleStatus, ToolOutcome};
use fabric_desktop_contracts::encode;
use fabric_desktop_contracts::error::{AgentError, AgentFault, ContractError, InvalidParams};
use fabric_desktop_contracts::lifecycle::RequestState;
use fabric_desktop_contracts::scalar::{Explanation, Text};
use serde_json::{Value, json};

use crate::broker::key::is_pairing_code;

/// A tool result before the era's members are added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// The first block: fixed wording (A27).
    pub guidance: String,
    /// The second block: the contract's value as canonical JSON.
    pub value: String,
    pub is_error: bool,
}

impl Rendered {
    /// The `content` array of the result.
    #[must_use]
    pub fn content(&self) -> Value {
        json!([
            {"type": "text", "text": self.guidance},
            {"type": "text", "text": self.value},
        ])
    }
}

/// The wording class of a request's state (W06 3.7, first table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wording {
    Result,
    PendingApproval,
    Applied,
    Working,
    OutcomeUnknown,
    Blocked,
    Denied,
    Ended,
    Undecided,
    Failed,
    NeedsOwner,
    AbsenceConfirmed,
}

/// The wording class and `isError` of a state with no result.
#[must_use]
pub const fn state_wording(state: RequestState) -> (Wording, bool) {
    use RequestState as S;
    match state {
        S::PendingApproval => (Wording::PendingApproval, false),
        S::Applied | S::Reconciled => (Wording::Applied, false),
        S::Received
        | S::Validating
        | S::ProvingClassification
        | S::Allowed
        | S::Approved
        | S::ProvingFreshPermit
        | S::Queued
        | S::Dispatching
        | S::RetryWait
        | S::Reconciling => (Wording::Working, false),
        S::OutcomeUnknown => (Wording::OutcomeUnknown, false),
        S::Blocked => (Wording::Blocked, true),
        S::Denied => (Wording::Denied, true),
        S::Expired | S::Cancelled | S::Stale => (Wording::Ended, true),
        S::Undecided => (Wording::Undecided, true),
        S::TerminalFailure => (Wording::Failed, true),
        S::NeedsOwnerResolution => (Wording::NeedsOwner, true),
        S::AbsenceConfirmed => (Wording::AbsenceConfirmed, true),
    }
}

/// The wording class and `isError` of an outcome.
#[must_use]
pub fn outcome_wording(outcome: &ToolOutcome) -> (Wording, bool) {
    if outcome.result.is_some() {
        (Wording::Result, false)
    } else {
        state_wording(outcome.request.status.state())
    }
}

/// The JSON string of a unit enum of the contract, such as a failure code or a limit.
macro_rules! name_of {
    ($value:expr) => {
        match serde_json::to_value($value) {
            Ok(Value::String(s)) => s,
            _ => String::new(),
        }
    };
}

/// The tool result for a broker's `tool_outcome`.
#[must_use]
pub fn outcome(outcome: &ToolOutcome) -> Rendered {
    let request = &outcome.request;
    let state = request.status.state();
    let (wording, is_error) = outcome_wording(outcome);
    let head = format!(
        "Request {} is {}.",
        request.request_id.as_str(),
        state.as_str()
    );
    let body = match (wording, &request.status) {
        (Wording::Result, _) => "The result follows.".to_owned(),
        (
            Wording::PendingApproval,
            AgentLifecycleStatus::PendingApproval {
                approval_expires_at,
            },
        ) => format!(
            "The owner must approve in Fabric Calendar. Do not repeat the call. Ask the owner, then call fabric_request_status with this request_id. The card expires at {}.",
            approval_expires_at.as_str()
        ),
        (Wording::Applied, _) => "The change was applied. fabric_request_result with this request_id gives the event's id and revision.".to_owned(),
        (Wording::Working, _) => "Fabric Calendar is still working. Call fabric_request_status with this request_id in a few seconds. Do not repeat the call.".to_owned(),
        (Wording::OutcomeUnknown, _) => "Fabric Calendar does not know yet whether the change reached the calendar. Do not repeat it. Call fabric_request_status later; the owner can also look in Fabric Calendar.".to_owned(),
        (Wording::Blocked, AgentLifecycleStatus::Blocked { denial }) => format!(
            "A rule blocked the request: {}. Nothing was changed.",
            denial.rule.as_str()
        ),
        (Wording::Denied, _) => "The owner denied the request. Nothing was changed.".to_owned(),
        (Wording::Ended, _) if state == RequestState::Stale => "The request ended without a change. Read again and propose again if it is still wanted.".to_owned(),
        (Wording::Ended, _) => "The request ended without a change.".to_owned(),
        (Wording::Undecided, _) => "Fabric Calendar could not check the request against its rules, so nothing was done. The owner can see why in Fabric Calendar. A new call may be made later.".to_owned(),
        (Wording::Failed, AgentLifecycleStatus::TerminalFailure { failure_code }) => format!(
            "The request failed: {}.",
            name_of!(failure_code)
        ),
        (Wording::NeedsOwner, _) => "Fabric Calendar does not know whether the change reached the calendar. Do not repeat it. The owner must look in Fabric Calendar.".to_owned(),
        (Wording::AbsenceConfirmed, _) => "The change did not reach the calendar.".to_owned(),
        // A status whose shape does not match its state cannot be read from the contract.
        _ => String::new(),
    };
    Rendered {
        guidance: format!("{head} {body}").trim_end().to_owned(),
        value: encode(outcome),
        is_error,
    }
}

const INTERNAL_WORDING: &str = "Fabric Calendar had a fault or did not answer. If a proposal was in flight it may have been recorded: do not repeat it. The owner must look in Fabric Calendar.";

/// The tool result for an agent error, from the broker or made by the bridge. The pairing
/// code is shown for `not_permitted` when the bridge knows it.
#[must_use]
pub fn error(error: &AgentError, pairing_code: Option<&str>) -> Rendered {
    let code = pairing_code.filter(|c| is_pairing_code(c));
    let guidance = match &error.fault {
        AgentFault::BrokerNotRunning => "Fabric Calendar is not running, or its connection ended. Ask the owner to open Fabric Calendar, then call again. If a proposal was in flight it may have been recorded: do not repeat it before the owner has looked.".to_owned(),
        AgentFault::ProverUnavailable => "Fabric Calendar's rule checker is not ready. Nothing was recorded. Try again in a little while.".to_owned(),
        AgentFault::Busy(busy) => format!(
            "Too many requests: the limit {} of at most {} was reached; wait {} seconds. Nothing was recorded.",
            name_of!(&busy.limit),
            busy.maximum.get(),
            busy.retry_after_ms.get().div_ceil(1000)
        ),
        AgentFault::NotPermitted => match code {
            Some(code) => format!("Fabric Calendar does not accept this agent. Its code is {code}. The owner can look in Fabric Calendar, Agents."),
            None => "Fabric Calendar does not accept this agent. The owner can look in Fabric Calendar, Agents.".to_owned(),
        },
        AgentFault::InvalidParams(_) => "The arguments are not valid; the reason is in the error below. Correct them and call again.".to_owned(),
        AgentFault::NotFound => "No request of this agent has that id.".to_owned(),
        AgentFault::UnsupportedSchemaVersion(_) => "Fabric Calendar and its bridge are different versions. The owner must install Fabric Calendar again.".to_owned(),
        AgentFault::Internal
        | AgentFault::MalformedFrame
        | AgentFault::UnsupportedProtocol(_)
        | AgentFault::UnknownMethod => INTERNAL_WORDING.to_owned(),
    };
    with_error(guidance, error)
}

/// The tool result while the agent waits for the owner's approval (W06 4.4, 5.2).
#[must_use]
pub fn pending(pairing_code: &str) -> Rendered {
    let guidance = if is_pairing_code(pairing_code) {
        format!(
            "This agent waits for the owner's approval in Fabric Calendar, Agents. Its code is {pairing_code}. After the approval, call again."
        )
    } else {
        "This agent waits for the owner's approval in Fabric Calendar, Agents. After the approval, call again.".to_owned()
    };
    let error = bridge_error(
        AgentFault::NotPermitted,
        "This agent waits for the owner's approval.",
    );
    with_error(guidance, &error)
}

/// The tool result for the seventeenth call in flight (W06 2.5).
#[must_use]
pub fn too_many_calls() -> Rendered {
    let error = bridge_error(
        AgentFault::Internal,
        "Too many calls at once in the bridge.",
    );
    with_error(
        "Too many calls at once: wait for an answer before the next call. Nothing was sent to Fabric Calendar.".to_owned(),
        &error,
    )
}

/// A fault of the bridge itself, answered as `internal` with its own wording (W06 3.7): the
/// call never reached Fabric Calendar, so the broker's "may have been recorded" does not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeFault {
    /// The system's random source failed.
    RandomSource,
    /// The agent's key could not be read or stored.
    Key,
    /// A handshake answer could not be read.
    HandshakeAnswer,
}

impl BridgeFault {
    /// The first block's fixed sentence.
    #[must_use]
    pub const fn guidance(self) -> &'static str {
        match self {
            BridgeFault::RandomSource => {
                "The bridge had a fault: the system's random source failed. Nothing was sent to Fabric Calendar. Try again in a little while."
            }
            BridgeFault::Key => {
                "The bridge had a fault: it could not read or store this agent's key. Nothing was sent to Fabric Calendar. The owner must look at the bridge's key."
            }
            BridgeFault::HandshakeAnswer => {
                "The bridge had a fault: it could not read Fabric Calendar's answer when it connected. Nothing was sent to Fabric Calendar. Try again in a little while."
            }
        }
    }

    /// The error's message in the second block.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            BridgeFault::RandomSource => "The bridge's random source failed.",
            BridgeFault::Key => "The bridge could not read or store its key.",
            BridgeFault::HandshakeAnswer => "The bridge could not read a handshake answer.",
        }
    }
}

/// The tool result for a fault of the bridge itself.
#[must_use]
pub fn fault(fault: BridgeFault) -> Rendered {
    let error = bridge_error(AgentFault::Internal, fault.message());
    with_error(fault.guidance().to_owned(), &error)
}

fn with_error(guidance: String, error: &AgentError) -> Rendered {
    Rendered {
        guidance,
        value: encode(&json!({ "error": error })),
        is_error: true,
    }
}

/// An error the bridge makes itself, written as the contract's agent error.
#[must_use]
pub fn bridge_error(fault: AgentFault, message: &'static str) -> AgentError {
    ContractError {
        fault,
        message: explanation(message),
        correlation_id: None,
    }
}

/// The bridge's `invalid_params` for arguments the contract refused: the pointer `/params`
/// and the contract's reason, cut to 1000 characters (W06 3.6 step 4).
#[must_use]
pub fn invalid_params(reason: &str) -> AgentError {
    ContractError {
        fault: AgentFault::InvalidParams(InvalidParams {
            pointer: Text::new("/params").unwrap_or_else(|_| unreachable!("a short pointer")),
        }),
        message: explanation(reason),
        correlation_id: None,
    }
}

/// The text cut to an explanation's 1000 code points, or a fixed sentence when it is empty.
fn explanation(text: &str) -> Explanation {
    let cut: String = text.chars().take(1000).collect();
    Explanation::new(cut)
        .or_else(|_| Explanation::new("The arguments were refused."))
        .unwrap_or_else(|_| unreachable!("a fixed sentence is an explanation"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::error::ErrorCode;
    use fabric_desktop_contracts::typed::from_value;

    const MARKER: &str = "MARKER-7f3a";

    fn status(state: RequestState) -> Value {
        match state {
            RequestState::PendingApproval => json!({
                "state": "pending_approval",
                "approval_expires_at": "2026-11-20T16:00:00.000Z"
            }),
            RequestState::Blocked => json!({
                "state": "blocked",
                "denial": {"rule": "agent_not_paused", "explanation": format!("why {MARKER}")}
            }),
            RequestState::TerminalFailure => json!({
                "state": "terminal_failure",
                "failure_code": "provider_rejected"
            }),
            other => json!({"state": other.as_str()}),
        }
    }

    fn outcome_of(state: RequestState, result: Value) -> ToolOutcome {
        from_value(&json!({
            "request": {
                "request_id": "req-1",
                "updated_at": "2026-11-20T15:00:00.000Z",
                "status": status(state),
            },
            "result": result,
        }))
        .expect("a contract outcome")
    }

    fn events_result() -> Value {
        json!({
            "kind": "events",
            "observed_at": "2026-11-20T15:00:00.000Z",
            "events": [{
                "event_id": "e1",
                "revision": "r1",
                "calendar_id": "c1",
                "fabric_created": false,
                "occurrence": null,
                "time": null,
                "title": format!("title {MARKER}"),
                "location": format!("loc {MARKER}"),
                "description": format!("desc {MARKER}"),
                "attendees": null,
                "organizer": null,
                "own_response": null,
            }]
        })
    }

    // r01: each of the 23 states gives the isError and the wording class of 3.7; the second
    // block is the canonical tool_outcome.
    #[test]
    fn r01_states() {
        use RequestState as S;
        let expected = [
            (S::Received, Wording::Working, false),
            (S::Validating, Wording::Working, false),
            (S::ProvingClassification, Wording::Working, false),
            (S::Allowed, Wording::Working, false),
            (S::PendingApproval, Wording::PendingApproval, false),
            (S::Blocked, Wording::Blocked, true),
            (S::Undecided, Wording::Undecided, true),
            (S::Approved, Wording::Working, false),
            (S::Denied, Wording::Denied, true),
            (S::Expired, Wording::Ended, true),
            (S::Cancelled, Wording::Ended, true),
            (S::Stale, Wording::Ended, true),
            (S::ProvingFreshPermit, Wording::Working, false),
            (S::Queued, Wording::Working, false),
            (S::Dispatching, Wording::Working, false),
            (S::Applied, Wording::Applied, false),
            (S::Reconciled, Wording::Applied, false),
            (S::RetryWait, Wording::Working, false),
            (S::OutcomeUnknown, Wording::OutcomeUnknown, false),
            (S::TerminalFailure, Wording::Failed, true),
            (S::Reconciling, Wording::Working, false),
            (S::AbsenceConfirmed, Wording::AbsenceConfirmed, true),
            (S::NeedsOwnerResolution, Wording::NeedsOwner, true),
        ];
        assert_eq!(expected.len(), RequestState::ALL.len());
        for (state, wording, is_error) in expected {
            let o = outcome_of(state, Value::Null);
            assert_eq!(
                outcome_wording(&o),
                (wording, is_error),
                "{}",
                state.as_str()
            );
            let r = outcome(&o);
            assert_eq!(r.is_error, is_error);
            assert_eq!(r.value, encode(&o));
            // The first block is exactly the fixed sentence of the state's class.
            assert_eq!(
                r.guidance,
                format!(
                    "Request req-1 is {}. {}",
                    state.as_str(),
                    expected_body(wording, state)
                ),
                "{}",
                state.as_str()
            );
            // With a result present, any state reads "the result follows".
            let with = outcome_of(state, events_result());
            assert_eq!(outcome_wording(&with), (Wording::Result, false));
            let rw = outcome(&with);
            assert!(!rw.is_error);
            assert_eq!(
                rw.guidance,
                format!("Request req-1 is {}. The result follows.", state.as_str())
            );
        }
        let pending = outcome(&outcome_of(S::PendingApproval, Value::Null));
        assert!(pending.guidance.contains("2026-11-20T16:00:00.000Z"));
        let blocked = outcome(&outcome_of(S::Blocked, Value::Null));
        assert!(blocked.guidance.contains("agent_not_paused"));
        let failed = outcome(&outcome_of(S::TerminalFailure, Value::Null));
        assert!(failed.guidance.contains("provider_rejected"));
        let stale = outcome(&outcome_of(S::Stale, Value::Null));
        assert!(stale.guidance.contains("propose again"));
    }

    /// The fixed sentence of each wording class (W06 3.7), written out here independently of
    /// the code under test, with the values of `status`.
    fn expected_body(wording: Wording, state: RequestState) -> &'static str {
        match wording {
            Wording::Result => "The result follows.",
            Wording::PendingApproval => {
                "The owner must approve in Fabric Calendar. Do not repeat the call. Ask the owner, then call fabric_request_status with this request_id. The card expires at 2026-11-20T16:00:00.000Z."
            }
            Wording::Applied => {
                "The change was applied. fabric_request_result with this request_id gives the event's id and revision."
            }
            Wording::Working => {
                "Fabric Calendar is still working. Call fabric_request_status with this request_id in a few seconds. Do not repeat the call."
            }
            Wording::OutcomeUnknown => {
                "Fabric Calendar does not know yet whether the change reached the calendar. Do not repeat it. Call fabric_request_status later; the owner can also look in Fabric Calendar."
            }
            Wording::Blocked => {
                "A rule blocked the request: agent_not_paused. Nothing was changed."
            }
            Wording::Denied => "The owner denied the request. Nothing was changed.",
            Wording::Ended if state == RequestState::Stale => {
                "The request ended without a change. Read again and propose again if it is still wanted."
            }
            Wording::Ended => "The request ended without a change.",
            Wording::Undecided => {
                "Fabric Calendar could not check the request against its rules, so nothing was done. The owner can see why in Fabric Calendar. A new call may be made later."
            }
            Wording::Failed => "The request failed: provider_rejected.",
            Wording::NeedsOwner => {
                "Fabric Calendar does not know whether the change reached the calendar. Do not repeat it. The owner must look in Fabric Calendar."
            }
            Wording::AbsenceConfirmed => "The change did not reach the calendar.",
        }
    }

    fn agent_error(code: &str, detail: Value) -> AgentError {
        let code_value = ErrorCode::from_name(code).unwrap();
        from_value(&json!({
            "code": code,
            "message": format!("message {MARKER}"),
            "retryable": code_value.retryable(),
            "recovery": code_value.recovery().as_str(),
            "correlation_id": null,
            "detail": detail,
        }))
        .expect("a contract error")
    }

    // r02: each of the agent error codes, named one by one, gives isError true, its wording
    // and the canonical error.
    #[test]
    fn r02_errors() {
        let cases = [
            ("malformed_frame", Value::Null, "had a fault"),
            (
                "unsupported_protocol",
                json!({"received": null, "supported": ["fabric-desktop/1"]}),
                "had a fault",
            ),
            (
                "unsupported_schema_version",
                json!({"received": 3, "supported": 2}),
                "different versions",
            ),
            ("unknown_method", Value::Null, "had a fault"),
            ("invalid_params", json!({"pointer": "/params"}), "not valid"),
            ("not_found", Value::Null, "No request of this agent"),
            ("internal", Value::Null, "had a fault"),
            (
                "busy",
                json!({"limit": "open_requests_per_agent", "maximum": 25, "retry_after_ms": 1500}),
                "open_requests_per_agent of at most 25 was reached; wait 2 seconds",
            ),
            ("not_permitted", Value::Null, "Its code is AB12-CD34-EF56"),
            ("broker_not_running", Value::Null, "is not running"),
            (
                "prover_unavailable",
                Value::Null,
                "rule checker is not ready",
            ),
        ];
        assert_eq!(cases.len(), 11);
        for (code, detail, words) in cases {
            let e = agent_error(code, detail);
            let r = error(&e, Some("AB12-CD34-EF56"));
            assert!(r.is_error, "{code}");
            assert!(r.guidance.contains(words), "{code}: {}", r.guidance);
            assert_eq!(r.value, encode(&json!({ "error": e })));
        }
        let e = agent_error("not_permitted", Value::Null);
        assert!(!error(&e, None).guidance.contains("code is"));
        let p = pending("AB12-CD34-EF56");
        assert!(p.is_error);
        assert!(p.guidance.contains("waits for the owner's approval"));
        assert!(p.guidance.contains("AB12-CD34-EF56"));
        assert!(p.value.contains("\"not_permitted\""));
        let t = too_many_calls();
        assert!(t.is_error && t.guidance.contains("Too many calls"));
        // The bridge's own internal faults: each its own sentence, never the broker's wording.
        for f in [
            BridgeFault::RandomSource,
            BridgeFault::Key,
            BridgeFault::HandshakeAnswer,
        ] {
            let r = fault(f);
            assert!(r.is_error);
            assert_eq!(r.guidance, f.guidance());
            assert!(r.guidance.contains("Nothing was sent to Fabric Calendar."));
            assert!(!r.guidance.contains("may have been recorded"));
            let v: Value = serde_json::from_str(&r.value).unwrap();
            assert_eq!(v["error"]["code"], json!("internal"));
            assert_eq!(v["error"]["message"], json!(f.message()));
        }
        assert!(
            fault(BridgeFault::RandomSource)
                .guidance
                .contains("random source")
        );
        assert!(fault(BridgeFault::Key).guidance.contains("key"));
        assert!(
            fault(BridgeFault::HandshakeAnswer)
                .guidance
                .contains("answer when it connected")
        );
        // internal from the broker (and the bridge's 30 s limit) keeps the broker's wording.
        let broker_internal = error(&agent_error("internal", Value::Null), None);
        assert_eq!(broker_internal.guidance, INTERNAL_WORDING);
        let i = invalid_params(&"x".repeat(1500));
        assert_eq!(i.message.as_str().chars().count(), 1000);
    }

    // r03: the first block holds none of a result's text, a rule's explanation or an error's
    // message (A27).
    #[test]
    fn r03_guidance_is_fixed() {
        let with_result = outcome(&outcome_of(RequestState::Applied, events_result()));
        assert!(!with_result.guidance.contains(MARKER));
        assert!(with_result.value.contains(MARKER));
        let blocked = outcome(&outcome_of(RequestState::Blocked, Value::Null));
        assert!(!blocked.guidance.contains(MARKER));
        assert!(blocked.value.contains(MARKER));
        for code in [
            "internal",
            "not_permitted",
            "not_found",
            "broker_not_running",
        ] {
            let r = error(&agent_error(code, Value::Null), Some("AB12-CD34-EF56"));
            assert!(!r.guidance.contains(MARKER));
            assert!(r.value.contains(MARKER));
        }
        let refused = error(&invalid_params(MARKER), None);
        assert!(!refused.guidance.contains(MARKER));
        assert!(refused.value.contains(MARKER));
        // A pairing code that is not of the code's grammar is never shown.
        let r = error(&agent_error("not_permitted", Value::Null), Some(MARKER));
        assert!(!r.guidance.contains(MARKER.split('-').next().unwrap()));
    }
}
