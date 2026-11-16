//! Command deduplication, revisions and the subscription (design 5.3, 5.4, D23; section 9.3
//! c16, c17). Gate G2a row "Double-click, dropped notice, cursor gap" (design 7.3).

#![cfg(debug_assertions)]

mod common;

use common::*;
use fabric_broker_core::Input;
use serde_json::{Value, json};

fn create_group_params(name: &str) -> Value {
    json!({
        "expected_revision": 0,
        "change": {
            "kind": "save",
            "recipient_set_id": null,
            "definition": {"name": name, "members": ["ally@example.net"], "domains": []}
        }
    })
}

/// c16. Design 5.3 and D23: a command repeated with its id after a restart gets the stored
/// answer byte for byte and writes nothing.
#[test]
fn duplicate_commands_after_restart_get_the_same_bytes() {
    let mut h = Harness::new("c16a", no_prover());
    h.setup();
    let line = owner_line("command-1", "save_group", create_group_params("Team"));
    let output = h.handle(Input::OwnerLine(line.clone()));
    let first = response_frame(&output, "command-1").to_line();
    assert!(result(&response(&output, "command-1"))["recipient_set"].is_object());

    h.advance(1_000);
    let _ = h.restart();
    h.advance(1_000);
    let head = h.ledger_head();
    let output = h.handle(Input::OwnerLine(line));
    assert!(output.halt.is_none());
    assert_eq!(response_frame(&output, "command-1").to_line(), first);
    assert_eq!(
        h.ledger_head(),
        head,
        "a repeated command wrote to the ledger"
    );
    assert_eq!(
        h.snapshot()["recipient_sets"].as_array().map(Vec::len),
        Some(1)
    );
    h.verify();
}

/// c16. Design 5.3 and section 4 (access audit): a command id used again for another command is
/// refused `command_reused`, audited, and changes nothing.
#[test]
fn a_reused_command_id_is_command_reused() {
    let mut h = Harness::new("c16b", no_prover());
    h.setup();
    let (response, _) = h.send("command-1", "save_group", create_group_params("Team"));
    let _ = result(&response);
    let head = h.ledger_head();

    let (response, output) = h.send("command-1", "save_group", create_group_params("Other"));
    assert!(output.halt.is_none());
    assert_eq!(error(&response)["code"], json!("command_reused"));
    assert_eq!(h.ledger_head(), head);
    assert!(
        h.audit().iter().any(|row| row.kind == "command_reused"),
        "the reused command id was not audited"
    );
    let sets = h.snapshot()["recipient_sets"].clone();
    assert_eq!(sets.as_array().map(Vec::len), Some(1));
    assert_eq!(sets[0]["definition"]["name"], json!("Team"));
    h.verify();
}

/// c16. Design 5.3 step 6: an `expected_revision` that is not the record's current revision is
/// `conflict` with `current_revision`, for every record kind a command names that way.
#[test]
fn a_stale_expected_revision_is_conflict() {
    let mut h = Harness::new("c16c", no_prover());
    h.setup();
    let set = h.create_group("Team", &["ally@example.net"]);

    let conflict = h.refused(
        "save_group",
        json!({
            "expected_revision": 5,
            "change": {
                "kind": "save",
                "recipient_set_id": set["recipient_set_id"],
                "definition": {"name": "Team", "members": [], "domains": []}
            }
        }),
    );
    assert_eq!(conflict["code"], json!("conflict"));
    assert_eq!(conflict["detail"]["current_revision"], set["revision"]);

    let revision = h.agent_view(AGENT)["revision"]
        .as_u64()
        .expect("a revision");
    let conflict = h.refused(
        "pause_agent",
        json!({
            "target": {"kind": "agent", "agent_id": AGENT, "expected_revision": revision + 7},
            "paused": true
        }),
    );
    assert_eq!(conflict["code"], json!("conflict"));
    assert_eq!(conflict["detail"]["current_revision"], json!(revision));

    let conflict = h.refused(
        "save_preferences",
        json!({"expected_revision": 7, "settings": {"zone": null, "workday": null,
            "protected_times": [], "uninterrupted": null, "buffer": null, "warning_calendars": []}}),
    );
    assert_eq!(conflict["code"], json!("conflict"));
    assert_eq!(conflict["detail"]["current_revision"], json!(0));

    let conflict = h.refused(
        "save_profile",
        json!({"expected_revision": 3, "change": {"kind": "activate", "profile_id": "flexible"}}),
    );
    assert_eq!(conflict["code"], json!("conflict"));
    assert_eq!(conflict["detail"]["current_revision"], json!(0));
    assert_eq!(
        h.snapshot()["profiles"]["active_profile_id"],
        json!("careful")
    );
    h.verify();
}

/// c17. Design 5.4 and D6: a cursor ahead of the store, or older than what the ring keeps, needs
/// a snapshot; one step producing more than 256 notices while subscribed sends one
/// `snapshot_required` change with the first unsent sequence and ends the subscription.
#[test]
fn subscribe_gaps_and_the_256_bound() {
    let mut h = Harness::new("c17a", no_prover());
    let agents: Vec<String> = (0..300).map(|n| format!("agent-{n:03}")).collect();
    let names: Vec<&str> = agents.iter().map(String::as_str).collect();
    let _ = h.seed(&names, vec![account(ACCOUNT, &[CALENDAR])]);
    let cursor = h.snapshot()["cursor"].as_u64().expect("a cursor");
    assert!(
        cursor > 256,
        "the seed should leave more notices than the ring keeps"
    );

    let ahead = h.refused("subscribe", json!({"cursor": cursor + 1}));
    assert_eq!(ahead["code"], json!("snapshot_required"));
    let gap = h.refused("subscribe", json!({"cursor": 1}));
    assert_eq!(gap["code"], json!("snapshot_required"));
    let answer = h.ok("subscribe", json!({"cursor": cursor}));
    assert_eq!(answer["cursor"], json!(cursor));

    let (response, output) = h.owner(
        "pause_agent",
        json!({"target": {"kind": "all_agents"}, "paused": true}),
    );
    let answer = result(&response);
    assert_eq!(answer["agents"].as_array().map(Vec::len), Some(300));
    assert_eq!(answer["commit_sequence"], json!(cursor + 300));
    let sent = notices(&output);
    assert_eq!(sent.len(), 1, "expected one snapshot_required notice");
    assert_eq!(sent[0]["change"], json!({"kind": "snapshot_required"}));
    assert_eq!(sent[0]["sequence"], json!(cursor + 1));

    let (_, output) = h.owner("save_group", create_group_params("Team"));
    assert!(notices(&output).is_empty(), "the subscription did not end");

    let cursor = h.snapshot()["cursor"].as_u64().expect("a cursor");
    let answer = h.ok("subscribe", json!({"cursor": cursor}));
    assert_eq!(answer["cursor"], json!(cursor));
    h.verify();
}

/// c17. Design 5.4 (ring base): after a restart the ring is empty and its base is
/// the store's notice sequence; a page that took its snapshot after the restart subscribes at
/// that cursor, and an older cursor needs a snapshot.
#[test]
fn after_a_restart_a_cursor_at_the_base_is_accepted() {
    let mut h = Harness::new("c17b", no_prover());
    let _ = h.seed(&[AGENT], vec![account(ACCOUNT, &[CALENDAR])]);
    let _ = h.create_group("Team", &[]);
    let before = h.snapshot()["cursor"].as_u64().expect("a cursor");
    assert!(before > 0);

    h.advance(1_000);
    let _ = h.restart();
    let cursor = h.snapshot()["cursor"].as_u64().expect("a cursor");
    assert_eq!(
        cursor, before,
        "a restart with nothing to recover sent notices"
    );
    let answer = h.ok("subscribe", json!({"cursor": cursor}));
    assert_eq!(answer["cursor"], json!(cursor));

    let older = h.refused("subscribe", json!({"cursor": cursor - 1}));
    assert_eq!(older["code"], json!("snapshot_required"));

    let answer = h.ok("subscribe", json!({"cursor": cursor}));
    assert_eq!(answer["cursor"], json!(cursor));
    let (_, output) = h.owner("save_group", create_group_params("Family"));
    let sent = notices(&output);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["sequence"], json!(cursor + 1));
    h.verify();
}
