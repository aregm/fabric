//! Behaviour of the transport types, decoding by layer, the digest helpers and the
//! authorization's expiry check.

use fabric_desktop_contracts::agent::{Action, AgentCall, AgentLifecycleStatus, RequestStatus};
use fabric_desktop_contracts::common::{CalendarRef, EventTime, SendUpdates, Window};
use fabric_desktop_contracts::error::{ErrorCode, OwnerFault};
use fabric_desktop_contracts::owner::{
    Authorization, BuiltInProfile, Duration, DurationKind, LifecycleStatus, OwnerCall, Profile,
    RecipientRule, Request, Scope,
};
use fabric_desktop_contracts::scalar::{Email, Id, List, Timestamp};
use fabric_desktop_contracts::typed::from_value;
use fabric_desktop_contracts::{
    Decision, Entry, Frame, Layer, MAX_FRAME_BYTES, approval_digest, canonical, decode,
    decode_record, encode, request_digest,
};
use serde_json::{Value, json};

fn bytes(value: &Value) -> Vec<u8> {
    canonical(value).into_bytes()
}

fn refusal(entry: Entry, value: &Value) -> (Layer, ErrorCode) {
    let r = decode(entry, &bytes(value)).unwrap_err();
    (r.layer, r.code)
}

fn hello() -> Value {
    json!({
        "protocol": "fabric-desktop/1",
        "id": "hello-1",
        "method": "hello",
        "params": {"client": {"name": "Fabric", "build": "0.1.0"}, "schema_version": 2}
    })
}

fn authorization(issued: &str, expires: &str) -> Value {
    json!({
        "profile": "fabric-broker-authorization/1",
        "authorization_id": "auth-1",
        "request_id": "request-1",
        "agent_id": "agent-1",
        "action_digest": format!("sha256:{}", "0".repeat(64)),
        "proof_id": "proof-1",
        "grant_id": "grant-1",
        "issued_at": issued,
        "expires_at": expires,
        "lifetime_seconds": 60,
        "consumed_at": null
    })
}

#[test]
fn decodes_a_hello_request() {
    let frame = decode(Entry::OwnerRequest, &bytes(&hello())).unwrap();
    let Frame::OwnerRequest(request) = &frame else {
        panic!("{frame:?}")
    };
    assert!(matches!(request.call, OwnerCall::Hello(_)));
    assert_eq!(request.command_digest(), None);
    assert_eq!(frame.to_line(), format!("{}\n", canonical(&hello())));
}

#[test]
fn a_frame_over_the_bound_is_malformed() {
    let pad = "x".repeat(MAX_FRAME_BYTES);
    let mut value = hello();
    value["params"]["client"]["name"] = Value::String(pad);
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Frame, ErrorCode::MalformedFrame)
    );
}

#[test]
fn a_record_has_no_size_bound() {
    let mut value = authorization("2026-11-20T16:00:00.000Z", "2026-11-20T16:01:00.000Z");
    value["authorization_id"] = Value::String("a".repeat(129));
    let r = decode_record::<Authorization>(&bytes(&value)).unwrap_err();
    assert_eq!((r.layer, r.code), (Layer::Schema, ErrorCode::InvalidParams));
}

#[test]
fn a_non_object_is_refused_at_the_frame_layer() {
    for value in [json!([]), json!("x"), json!(null), json!(1)] {
        assert_eq!(
            refusal(Entry::OwnerNotice, &value),
            (Layer::Frame, ErrorCode::MalformedFrame)
        );
        let r = decode_record::<Request>(&bytes(&value)).unwrap_err();
        assert_eq!(r.layer, Layer::Frame);
    }
}

#[test]
fn a_missing_or_wrong_protocol_is_refused_at_the_protocol_layer() {
    let mut value = hello();
    value["protocol"] = json!("fabric-desktop/2");
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Protocol, ErrorCode::UnsupportedProtocol)
    );
    value.as_object_mut().unwrap().remove("protocol");
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Protocol, ErrorCode::UnsupportedProtocol)
    );
}

#[test]
fn an_unknown_method_is_unknown_method() {
    let mut value = hello();
    value["method"] = json!("fabric_calendar_list");
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Schema, ErrorCode::UnknownMethod)
    );
    value["method"] = json!(5);
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Schema, ErrorCode::UnknownMethod)
    );
}

#[test]
fn bad_params_of_a_known_method_are_invalid_params() {
    let mut value = hello();
    value["params"]["schema_version"] = json!(-1);
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Schema, ErrorCode::InvalidParams)
    );
}

#[test]
fn an_unknown_member_is_refused() {
    let mut value = hello();
    value["params"]["client"]["extra"] = json!(null);
    assert_eq!(
        refusal(Entry::OwnerRequest, &value),
        (Layer::Schema, ErrorCode::InvalidParams)
    );
}

#[test]
fn a_nullable_member_must_be_present() {
    let mut value = authorization("2026-11-20T16:00:00.000Z", "2026-11-20T16:01:00.000Z");
    assert!(decode_record::<Authorization>(&bytes(&value)).is_ok());
    value.as_object_mut().unwrap().remove("consumed_at");
    assert!(decode_record::<Authorization>(&bytes(&value)).is_err());
}

#[test]
fn authorization_lifetime_must_be_sixty() {
    let mut value = authorization("2026-11-20T16:00:00.000Z", "2026-11-20T16:01:00.000Z");
    value["lifetime_seconds"] = json!(61);
    assert!(decode_record::<Authorization>(&bytes(&value)).is_err());
}

fn auth(issued: &str, expires: &str) -> Authorization {
    from_value(&authorization(issued, expires)).unwrap()
}

#[test]
fn expiry_is_issue_time_plus_sixty_seconds() {
    assert!(auth("2026-11-20T16:00:00.000Z", "2026-11-20T16:01:00.000Z").expiry_matches_lifetime());
}

#[test]
fn expiry_crosses_midnight_and_the_year() {
    assert!(auth("2026-12-31T23:59:30.500Z", "2027-01-01T00:00:30.500Z").expiry_matches_lifetime());
}

#[test]
fn expiry_crosses_a_leap_day() {
    assert!(auth("2028-02-28T23:59:59.999Z", "2028-02-29T00:00:59.999Z").expiry_matches_lifetime());
    assert!(
        !auth("2027-02-28T23:59:59.999Z", "2027-02-29T00:00:59.999Z").expiry_matches_lifetime()
    );
}

#[test]
fn expiry_off_by_a_millisecond_is_wrong() {
    assert!(
        !auth("2026-11-20T16:00:00.000Z", "2026-11-20T16:01:00.001Z").expiry_matches_lifetime()
    );
    assert!(
        !auth("2026-11-20T16:00:00.000Z", "2026-11-20T16:00:00.000Z").expiry_matches_lifetime()
    );
}

#[test]
fn unix_millis_counts_from_the_epoch() {
    let t = |s: &str| Timestamp::new(s).unwrap().unix_millis();
    assert_eq!(t("1970-01-01T00:00:00.000Z"), Some(0));
    assert_eq!(t("2000-03-01T00:00:00.001Z"), Some(951_868_800_001));
    assert_eq!(t("1969-12-31T23:59:59.999Z"), Some(-1));
    assert_eq!(t("2026-02-30T00:00:00.000Z"), None);
    assert_eq!(t("2026-04-31T00:00:00.000Z"), None);
    assert!(t("0000-02-29T00:00:00.000Z").is_some());
    assert_eq!(t("1900-02-29T00:00:00.000Z"), None);
}

#[test]
fn built_in_profiles_are_exact() {
    let careful = BuiltInProfile::Careful.to_value();
    assert_eq!(
        from_value::<Profile>(&careful).unwrap(),
        Profile::BuiltIn(BuiltInProfile::Careful)
    );
    let mut changed = careful.clone();
    changed["rows"]["read_event_details"] = json!("allow");
    assert!(from_value::<Profile>(&changed).is_err());
    let mut renamed = careful;
    renamed["name"] = json!("careful");
    assert!(from_value::<Profile>(&renamed).is_err());
}

#[test]
fn a_custom_profile_cannot_unfix_delete_event() {
    let mut rows = BuiltInProfile::Flexible.to_value()["rows"].clone();
    let custom = |rows: &Value| json!({"kind": "custom", "profile_id": "custom-a", "name": "Mine", "version": 1, "rows": rows});
    assert!(from_value::<Profile>(&custom(&rows)).is_ok());
    rows["delete_event"] = json!("allow");
    assert!(from_value::<Profile>(&custom(&rows)).is_err());
}

#[test]
fn a_blocked_request_carries_its_ruling() {
    let request = |status: Value| {
        json!({
            "request_id": "request-1", "revision": 1, "agent_id": "agent-1",
            "tool": "fabric_calendar_read", "account_id": null,
            "received_at": "2026-11-20T16:00:00.000Z", "updated_at": "2026-11-20T16:00:00.000Z",
            "request_digest": format!("sha256:{}", "0".repeat(64)), "status": status
        })
    };
    let ruling = json!({"rule": "strangers_always_ask", "explanation": "x", "proof_id": "proof-1"});
    let blocked: Request =
        from_value(&request(json!({"state": "blocked", "denial": ruling}))).unwrap();
    assert!(matches!(blocked.status, LifecycleStatus::Blocked { .. }));
    assert_eq!(blocked.status.state().as_str(), "blocked");
    assert!(from_value::<Request>(&request(json!({"state": "blocked"}))).is_err());
    assert!(from_value::<Request>(&request(json!({"state": "pending_approval"}))).is_err());
    let queued: Request = from_value(&request(json!({"state": "queued"}))).unwrap();
    assert_eq!(queued.status.state().as_str(), "queued");
}

#[test]
fn an_agent_status_keeps_the_proof_from_the_agent() {
    let status = |denial: Value| {
        json!({"request_id": "request-1", "updated_at": "2026-11-20T16:00:00.000Z",
               "status": {"state": "blocked", "denial": denial}})
    };
    let ok: RequestStatus = from_value(&status(json!({"rule": "a", "explanation": "x"}))).unwrap();
    assert!(matches!(ok.status, AgentLifecycleStatus::Blocked { .. }));
    assert!(
        from_value::<RequestStatus>(&status(
            json!({"rule": "a", "explanation": "x", "proof_id": "p"})
        ))
        .is_err()
    );
}

#[test]
fn the_status_tool_is_not_an_action() {
    let frame = json!({
        "protocol": "fabric-desktop/1", "id": "call-1",
        "method": "fabric_request_status", "params": {"request_id": "request-1"}
    });
    let Frame::AgentRequest(request) = decode(Entry::AgentRequest, &bytes(&frame)).unwrap() else {
        panic!()
    };
    assert!(matches!(request.call, AgentCall::RequestStatus(_)));
    assert_eq!(request.action_digest(), None);
    let action = json!({"tool": "fabric_request_status", "arguments": {"request_id": "request-1"}});
    assert!(from_value::<Action>(&action).is_err());
}

#[test]
fn an_action_digest_binds_the_request_frame() {
    let frame = json!({
        "protocol": "fabric-desktop/1", "id": "call-1",
        "method": "fabric_calendar_list", "params": {}
    });
    let Frame::AgentRequest(request) = decode(Entry::AgentRequest, &bytes(&frame)).unwrap() else {
        panic!()
    };
    let action: Action =
        from_value(&json!({"tool": "fabric_calendar_list", "arguments": {}})).unwrap();
    assert_eq!(request.action_digest(), Some(action.digest()));
}

#[test]
fn an_empty_params_object_is_not_an_array() {
    let frame = json!({
        "protocol": "fabric-desktop/1", "id": "call-1",
        "method": "fabric_calendar_list", "params": []
    });
    assert_eq!(
        refusal(Entry::AgentRequest, &frame),
        (Layer::Schema, ErrorCode::InvalidParams)
    );
}

fn digest_fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/desktop/v1/fixtures/digests.json");
    let all: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    all.as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .cloned()
        .unwrap()
}

#[test]
fn typed_request_digest_matches_the_fixture() {
    let e = digest_fixture("request-invitation");
    let subject = &e["subject"];
    let agent: Id = from_value(&subject["agent_id"]).unwrap();
    let action: Action = from_value(&subject["action"]).unwrap();
    let scope: Scope = from_value(&subject["scope"]).unwrap();
    assert_eq!(
        request_digest(&agent, &action, &scope).as_str(),
        e["digest"]
    );
}

#[test]
fn typed_approval_digests_match_the_fixtures() {
    for name in [
        "approval-approve-one-request",
        "approval-approve-until-revoked",
        "approval-deny",
    ] {
        let e = digest_fixture(name);
        let s = &e["subject"];
        let decision = if s["decision"] == "approve" {
            Decision::Approve {
                scope: from_value(&s["scope"]).unwrap(),
                duration: from_value(&s["duration"]).unwrap(),
            }
        } else {
            Decision::Deny
        };
        let d = approval_digest(
            &from_value(&s["request_id"]).unwrap(),
            &from_value(&s["request_digest"]).unwrap(),
            &decision,
        );
        assert_eq!(d.as_str(), e["digest"], "{name}");
    }
}

#[test]
fn typed_command_digest_matches_the_fixture() {
    let e = digest_fixture("command-pause-all");
    let frame = json!({
        "protocol": "fabric-desktop/1", "id": "command-1",
        "method": e["subject"]["method"], "params": e["subject"]["params"]
    });
    let Frame::OwnerRequest(request) = decode(Entry::OwnerRequest, &bytes(&frame)).unwrap() else {
        panic!()
    };
    assert_eq!(request.command_digest().unwrap().as_str(), e["digest"]);
}

#[test]
fn scope_round_trips_with_nulls_written() {
    let scope = Scope {
        calendars: List::new(vec![CalendarRef {
            account_id: Id::new("acct-1").unwrap(),
            calendar_id: Id::new("cal-1").unwrap(),
        }])
        .unwrap(),
        window: Some(Window {
            start: Timestamp::new("2026-11-20T16:00:00.000Z").unwrap(),
            end: Timestamp::new("2026-11-20T17:00:00.000Z").unwrap(),
        }),
        fields: None,
        recipients: Some(RecipientRule::Exact(
            List::new(vec![Email::new("bob@example.com").unwrap()]).unwrap(),
        )),
        send_updates: Some(SendUpdates::None),
    };
    let text = encode(&scope);
    assert_eq!(
        text,
        r#"{"calendars":[{"account_id":"acct-1","calendar_id":"cal-1"}],"fields":null,"recipients":{"addresses":["bob@example.com"],"kind":"exact"},"send_updates":"none","window":{"end":"2026-11-20T17:00:00.000Z","start":"2026-11-20T16:00:00.000Z"}}"#
    );
    let back: Scope = decode_record(text.as_bytes()).unwrap();
    assert_eq!(back, scope);
}

#[test]
fn duplicate_calendars_in_a_scope_are_refused() {
    let one = json!({"account_id": "a", "calendar_id": "c"});
    let scope = json!({"calendars": [one, one], "window": null, "fields": null,
                       "recipients": null, "send_updates": null});
    assert!(from_value::<Scope>(&scope).is_err());
}

#[test]
fn an_error_must_carry_its_fixed_retryable_and_recovery() {
    use fabric_desktop_contracts::error::OwnerError;
    let error = |retryable: bool, recovery: &str| {
        json!({"code": "conflict", "message": "Changed.", "retryable": retryable,
               "recovery": recovery, "correlation_id": null, "detail": {"current_revision": 4}})
    };
    let ok: OwnerError = from_value(&error(false, "refresh_snapshot")).unwrap();
    assert!(matches!(ok.fault, OwnerFault::Conflict(_)));
    assert!(from_value::<OwnerError>(&error(true, "refresh_snapshot")).is_err());
    assert!(from_value::<OwnerError>(&error(false, "report_problem")).is_err());
}

#[test]
fn busy_is_not_an_owner_error() {
    use fabric_desktop_contracts::error::{AgentError, OwnerError};
    let busy = json!({"code": "busy", "message": "Later.", "retryable": true, "recovery": "retry_later",
                      "correlation_id": null,
                      "detail": {"limit": "open_requests_total", "maximum": 100, "retry_after_ms": 0}});
    assert!(from_value::<AgentError>(&busy).is_ok());
    assert!(from_value::<OwnerError>(&busy).is_err());
}

#[test]
fn event_time_needs_an_offset_and_a_zone() {
    let ok = json!({"date_time": "2026-11-20T10:00:00-06:00", "zone": "America/Chicago"});
    assert!(from_value::<EventTime>(&ok).is_ok());
    let z = json!({"date_time": "2026-11-20T16:00:00Z", "zone": "UTC"});
    assert!(from_value::<EventTime>(&z).is_err());
}

#[test]
fn duration_is_an_object_with_a_kind() {
    let d: Duration = from_value(&json!({"kind": "session"})).unwrap();
    assert_eq!(d.kind, DurationKind::Session);
    assert!(from_value::<Duration>(&json!("session")).is_err());
}
