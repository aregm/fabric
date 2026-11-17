//! Request limits and their audit (design 5.9, D24, D31, section 4 access audit; section 9.3
//! c20).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::ports::AttemptReport;
use fabric_desktop_contracts::MAX_FRAME_BYTES;
use fabric_desktop_contracts::owner::{ProfileRows, ProofPurpose};
use serde_json::{Value, json};

/// The minute the per-agent rate limit counts over, plus one second.
const NEXT_MINUTE: u64 = 61_000;

/// Asserts a `busy` reply naming `limit` and `maximum`; returns its detail.
fn assert_busy(reply: &Value, limit: &str, maximum: u64) -> Value {
    let refusal = error(reply);
    assert_eq!(refusal["code"], json!("busy"), "{reply}");
    assert_eq!(refusal["detail"]["limit"], json!(limit), "{reply}");
    assert_eq!(refusal["detail"]["maximum"], json!(maximum), "{reply}");
    assert!(refusal["detail"]["retry_after_ms"].as_u64().is_some());
    refusal["detail"].clone()
}

/// A request that stays open: allowed on `grant`, permitted, handed off and left with an
/// unknown outcome.
fn open_request(h: &mut Harness, connection: u64, call_id: &str, grant: &str) -> String {
    let (request_id, output) = h.propose(connection, call_id);
    let job = job_for(&output, &request_id, ProofPurpose::Classification);
    let output = h.prove(&job, Outcome::Permit, &[grant]);
    let fresh = job_for(&output, &request_id, ProofPurpose::FreshPermit);
    let order = h.dispatch(&fresh, &[grant]);
    let _ = h.attempt(&order, AttemptReport::NoAnswer);
    request_id
}

/// Gives the agent on `connection` an until-revoked grant and 25 open requests, at most 14
/// calls a minute.
fn fill_agent(h: &mut Harness, connection: u64) {
    let (first, grant, fresh) = h.approved(connection, "call-0", "until_revoked");
    let order = h.dispatch(&fresh, &[&grant]);
    let _ = h.attempt(&order, AttemptReport::NoAnswer);
    assert_eq!(h.state(&first), "outcome_unknown");
    for n in 1..25 {
        if n % 14 == 0 {
            h.advance(NEXT_MINUTE);
        }
        let _ = open_request(h, connection, &format!("call-{n}"), &grant);
    }
    h.advance(NEXT_MINUTE);
}

/// c20. Design 5.9 and CR 8: past any of the four limits a proposal is answered `busy` with the
/// limit, its maximum and a wait; no request and no ledger row are created.
#[test]
fn busy_limits_create_no_request() {
    // Requests per minute per agent: 15 in the last 60 s.
    let mut h = Harness::new("c20a-rate", no_prover());
    h.setup();
    for n in 0..15 {
        let (reply, _) = h.call(
            1,
            &format!("call-{n}"),
            CREATE,
            create_params(ACCOUNT, CALENDAR, "x"),
        );
        let _ = request_id_of(&reply);
    }
    h.advance(1_000);
    let head = h.ledger_head();
    let (reply, output) = h.call(1, "call-15", CREATE, create_params(ACCOUNT, CALENDAR, "x"));
    assert!(output.halt.is_none());
    let detail = assert_busy(&reply, "requests_per_minute_per_agent", 15);
    assert_eq!(detail["retry_after_ms"], json!(59_000));
    assert_eq!(h.ledger_head(), head);
    h.kill();
    assert_eq!(
        h.read(|r| r.request_by_call(&id(AGENT), &id("call-15"))),
        None
    );
    h.verify();

    // Open approval cards per agent: 5.
    let mut h = Harness::new("c20a-cards", isabelle());
    h.setup();
    for n in 0..5 {
        let _ = h.pending(1, &format!("call-{n}"));
    }
    let head = h.ledger_head();
    let (reply, _) = h.call(1, "call-5", CREATE, create_params(ACCOUNT, CALENDAR, "x"));
    let _ = assert_busy(&reply, "open_approval_cards_per_agent", 5);
    assert_eq!(h.ledger_head(), head);
    h.verify();

    // Open requests per agent (25) and in total (100).
    let mut h = Harness::new("c20a-open", isabelle());
    let agents = ["agent-0", "agent-1", "agent-2", "agent-3", "agent-4"];
    let _ = h.seed(&agents, vec![account(ACCOUNT, &[CALENDAR])]);
    for (n, agent) in agents.iter().enumerate() {
        let _ = h.connect(n as u64 + 1, agent);
    }
    fill_agent(&mut h, 1);
    let head = h.ledger_head();
    let (reply, _) = h.call(1, "call-25", CREATE, create_params(ACCOUNT, CALENDAR, "x"));
    let _ = assert_busy(&reply, "open_requests_per_agent", 25);
    assert_eq!(h.ledger_head(), head);

    for connection in 2..=4 {
        fill_agent(&mut h, connection);
    }
    assert_eq!(h.snapshot()["requests"].as_array().map(Vec::len), Some(100));
    let head = h.ledger_head();
    let (reply, _) = h.call(5, "call-0", CREATE, create_params(ACCOUNT, CALENDAR, "x"));
    let _ = assert_busy(&reply, "open_requests_total", 100);
    assert_eq!(h.ledger_head(), head);
    h.kill();
    assert_eq!(
        h.read(|r| r.request_by_call(&id("agent-4"), &id("call-0"))),
        None
    );
    h.verify();
}

/// c20. Design D24 and section 4 (access audit): `busy` refusals are counted in memory and
/// written as one `agent_busy` audit row per agent per one-minute window, at the window's end or
/// at quit, never as a ledger row.
#[test]
fn busy_refusals_are_audited_once_per_agent_per_minute() {
    let mut h = Harness::new("c20b", no_prover());
    h.setup();
    let busy = |h: &mut Harness, call: &str| {
        let (reply, _) = h.call(1, call, CREATE, create_params(ACCOUNT, CALENDAR, "x"));
        let _ = assert_busy(&reply, "requests_per_minute_per_agent", 15);
    };

    for n in 0..15 {
        let _ = h.propose(1, &format!("call-a{n}"));
    }
    h.advance(1_000);
    let first_busy = h.now;
    let head = h.ledger_head();
    for n in 0..3 {
        busy(&mut h, &format!("busy-a{n}"));
        h.advance(1_000);
    }
    assert!(
        h.audit().iter().all(|row| row.kind != "agent_busy"),
        "a busy window was written before its end"
    );
    assert_eq!(h.ledger_head(), head, "a busy refusal wrote to the ledger");

    // The window ends: one row with its count.
    h.now = first_busy + 60_000 + 1;
    let _ = h.tick();
    let rows: Vec<AuditRow> = h
        .audit()
        .into_iter()
        .filter(|r| r.kind == "agent_busy")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].detail["agent_id"], json!(AGENT));
    assert_eq!(rows[0].detail["count"], json!(3));

    // A second window, written at quit.
    for n in 0..15 {
        let _ = h.propose(1, &format!("call-b{n}"));
    }
    for n in 0..2 {
        busy(&mut h, &format!("busy-b{n}"));
    }
    let _ = h.quit();
    let rows: Vec<AuditRow> = h
        .audit()
        .into_iter()
        .filter(|r| r.kind == "agent_busy")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].detail["count"], json!(2));
    h.verify();
}

/// c20. Design 5.9 and R18: the open-card limit is checked only when a request is created; a
/// re-ask by a fresh proof may make a sixth open card.
#[test]
fn the_open_card_limit_is_checked_only_at_creation() {
    let mut h = Harness::new("c20c", isabelle());
    h.setup();
    let (reasked, _, fresh) = h.approved(1, "call-0", "until_revoked");
    for n in 1..=5 {
        let _ = h.pending(1, &format!("call-{n}"));
    }
    let (reply, _) = h.call(1, "call-6", CREATE, create_params(ACCOUNT, CALENDAR, "x"));
    let _ = assert_busy(&reply, "open_approval_cards_per_agent", 5);

    let output = h.prove(&fresh, Outcome::NeedApproval, &[]);
    assert!(output.halt.is_none());
    assert_eq!(h.state(&reasked), "pending_approval");
    let snapshot = h.snapshot();
    let cards = snapshot["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .filter(|r| r["status"]["state"] == json!("pending_approval"))
        .count();
    assert_eq!(cards, 6);
    h.verify();
}

/// The longest owner frame of an output, in bytes without its line feed.
fn longest_owner_frame(output: &fabric_broker_core::Output) -> usize {
    output
        .owner
        .iter()
        .map(|frame| frame.to_line().len() - 1)
        .max()
        .unwrap_or(0)
}

/// A recipient set definition of `members` addresses of 320 bytes and `domains` domains of
/// 253 bytes, the schema's longest, all different by `tag`.
fn long_definition(tag: &str, members: usize, domains: usize) -> Value {
    let label = |c: char| c.to_string().repeat(63);
    let domain_of_255 = format!(
        "{}.{}.{}.{}",
        label('d'),
        label('e'),
        label('f'),
        label('g')
    );
    let members: Vec<String> = (0..members)
        .map(|n| format!("{:a>64}@{domain_of_255}", format!("{tag}{n}")))
        .collect();
    let domains: Vec<String> = (0..domains)
        .map(|n| {
            format!(
                "{:h>61}.{}.{}.{}",
                format!("{tag}{n}"),
                label('i'),
                label('j'),
                label('k')
            )
        })
        .collect();
    assert!(members.iter().all(|m| m.len() == 320));
    assert!(domains.iter().all(|d| d.len() == 253));
    json!({"name": format!("{tag:x>200}"), "members": members, "domains": domains})
}

fn save_group(h: &mut Harness, definition: Value) -> (Value, fabric_broker_core::Output) {
    h.owner(
        "save_group",
        json!({"expected_revision": 0,
               "change": {"kind": "save", "recipient_set_id": null, "definition": definition}}),
    )
}

fn assert_over_budget(response: &Value, pointer: &str) {
    let refusal = error(response);
    assert_eq!(refusal["code"], json!("invalid_params"), "{response}");
    assert_eq!(refusal["detail"]["pointer"], json!(pointer), "{response}");
}

/// README section 1 and plan 9.1: a frame is at most 1,048,576 bytes. The owner's own records
/// (recipient sets, custom profiles, live grants) are kept within a budget, so with each filled
/// to the limit a snapshot and the profiles notice still fit in one frame; the command that
/// would go past it is refused `invalid_params`, nothing is stored and the broker runs on.
#[test]
fn the_owner_records_are_bounded_so_a_snapshot_fits_in_one_frame() {
    let mut h = Harness::new("frame-budget", isabelle());
    h.setup();
    let pending = h.pending(1, "call-1");
    let _ = h.subscribe();

    // The schema's largest recipient set fits; a second one does not.
    let (response, output) = save_group(&mut h, long_definition("a", 2000, 50));
    let _ = result(&response);
    assert!(longest_owner_frame(&output) <= MAX_FRAME_BYTES);
    let (response, output) = save_group(&mut h, long_definition("b", 2000, 50));
    assert_over_budget(&response, "/params/change/definition");
    assert!(output.halt.is_none());
    assert_eq!(
        h.snapshot()["recipient_sets"].as_array().map(Vec::len),
        Some(1),
        "a refused set was stored"
    );

    // Custom profiles with the longest names, until the budget refuses one.
    let rows = serde_json::to_value(ProfileRows::CAREFUL).expect("rows");
    let mut profiles = 0;
    let mut revision = h.snapshot()["profiles"]["revision"].clone();
    loop {
        let (response, output) = h.owner(
            "save_profile",
            json!({"expected_revision": revision,
                   "change": {"kind": "save_custom",
                              "profile": {"profile_id": null,
                                          "name": format!("{profiles:p>200}"), "rows": rows},
                              "acknowledged_expansion": null}}),
        );
        assert!(longest_owner_frame(&output) <= MAX_FRAME_BYTES);
        if response.get("error").is_some() {
            assert_over_budget(&response, "/params/change/profile");
            break;
        }
        assert_eq!(
            notices_of(&output, "profiles").len(),
            1,
            "no profiles notice"
        );
        revision = result(&response)["profiles"]["revision"].clone();
        profiles += 1;
        assert!(profiles < 2_000, "the profiles were never bounded");
    }
    assert!(
        profiles > 0,
        "no custom profile fitted next to the largest set"
    );

    // The smallest sets take what is left.
    for n in 0.. {
        let (response, _) = save_group(&mut h, long_definition(&format!("s{n}"), 1, 0));
        if response.get("error").is_some() {
            assert_over_budget(&response, "/params/change/definition");
            break;
        }
        assert!(n < 2_000, "the sets were never bounded");
    }

    // A grant does not fit either: the approval is refused and the request stays pending.
    let (response, output) = h.decide(&pending, "approve", json!({"kind": "until_revoked"}));
    assert_over_budget(&response, "/params/decision");
    assert!(output.halt.is_none());
    assert_eq!(h.state(&pending), "pending_approval");

    let (response, output) = h.owner("get_snapshot", json!({}));
    let snapshot = result(&response);
    assert!(longest_owner_frame(&output) <= MAX_FRAME_BYTES);
    assert!(snapshot["recipient_sets"].as_array().map_or(0, Vec::len) >= 1);
    assert!(snapshot["grants"].as_array().is_some_and(Vec::is_empty));

    // A command that adds nothing still goes through.
    let (response, _) = h.decide(&pending, "deny", Value::Null);
    let _ = result(&response);
    h.verify();
}

/// README section 1: an answer the broker cannot fit in one frame is refused `internal` with
/// its id and method, never written over the bound, and the broker runs on.
#[test]
fn an_answer_over_the_frame_bound_is_refused_and_the_broker_runs_on() {
    let mut h = Harness::new("frame-guard", no_prover());
    let calendars: Vec<String> = (0..6_000).map(|n| format!("cal-{n:0>120}")).collect();
    let names: Vec<&str> = calendars.iter().map(String::as_str).collect();
    let _ = h.seed(&[AGENT], vec![account(ACCOUNT, &names)]);

    let (response, output) = h.send("snapshot-1", "get_snapshot", json!({}));
    assert!(output.halt.is_none());
    assert!(longest_owner_frame(&output) <= MAX_FRAME_BYTES);
    assert_eq!(response["id"], json!("snapshot-1"));
    assert_eq!(response["method"], json!("get_snapshot"));
    assert_eq!(error(&response)["code"], json!("internal"));

    let hello = h.ok(
        "hello",
        json!({"schema_version": 3, "client": {"name": "Test", "build": "1"}}),
    );
    assert_eq!(hello["schema_version"], json!(3));
    assert!(h.running());
    h.verify();
}
