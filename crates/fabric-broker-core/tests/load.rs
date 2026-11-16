//! One in-process writer under sequential load (design 5.1, section 4; section 9.3 c24): gate
//! G2a row R09 (ledger correlation), and the gap-free sequences of the DEF-1 row (design 7.3).
//! It drives one `Broker` from one thread, so it cannot show a second writer; that is ruled out
//! by the store lock (s06 in fabric-storage, p02 in fabric-broker) and by the single owning
//! `Core`.

#![cfg(debug_assertions)]

mod common;

use std::collections::HashMap;

use common::*;
use fabric_broker_core::Output;
use serde_json::{Value, json};

const AGENTS: usize = 10;
const CALLS: usize = 500;

/// Checks a step did not halt and keeps the sequences of its notices.
fn record(output: &Output, sequences: &mut Vec<u64>) {
    assert!(output.halt.is_none(), "the writer halted under load");
    sequences.extend(
        notices(output)
            .iter()
            .map(|n| n["sequence"].as_u64().expect("a sequence")),
    );
}

/// c24. PLAN 5 R09 (design 7.3), 5.1 and section 4: 500 agent calls interleaved with owner commands go, in
/// sequence, through one in-process writer: every call is answered with its own request, every
/// notice sequence rises by one, every request's ledger rows carry its id as correlation starting
/// with its `agent.call` row, and the chain and the replay verify.
#[test]
fn one_writer_under_load_every_record_verifies() {
    let mut h = Harness::new("c24", no_prover());
    let agents: Vec<String> = (0..AGENTS).map(|n| format!("agent-{n}")).collect();
    let names: Vec<&str> = agents.iter().map(String::as_str).collect();
    let _ = h.seed(&names, vec![account(ACCOUNT, &[CALENDAR])]);
    for (n, agent) in names.iter().enumerate() {
        let _ = h.connect(n as u64 + 1, agent);
    }
    let cursor = h.subscribe();

    let mut sequences: Vec<u64> = Vec::new();
    let mut requests: Vec<String> = Vec::new();
    let mut groups: Vec<Value> = Vec::new();
    for n in 0..CALLS {
        // Ten agents at 15 calls a minute each.
        if n > 0 && n % (AGENTS * 15) == 0 {
            h.advance(61_000);
        }
        let connection = (n % AGENTS) as u64 + 1;
        let (reply, output) = h.call(
            connection,
            &format!("call-{n}"),
            CREATE,
            create_params(ACCOUNT, CALENDAR, &format!("event {n}")),
        );
        record(&output, &mut sequences);
        requests.push(request_id_of(&reply));

        if n % 10 == 9 {
            let (response, output) = match groups.pop() {
                Some(set) => h.owner(
                    "save_group",
                    json!({
                        "expected_revision": set["revision"],
                        "change": {
                            "kind": "save",
                            "recipient_set_id": set["recipient_set_id"],
                            "definition": {"name": format!("Group {n}"), "members": [], "domains": []}
                        }
                    }),
                ),
                None => h.owner(
                    "save_group",
                    json!({
                        "expected_revision": 0,
                        "change": {
                            "kind": "save",
                            "recipient_set_id": null,
                            "definition": {"name": format!("Group {n}"), "members": [], "domains": []}
                        }
                    }),
                ),
            };
            record(&output, &mut sequences);
            groups.push(result(&response)["recipient_set"].clone());
        }
    }

    let mut distinct = requests.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), CALLS, "two calls share a request");
    let expected: Vec<u64> = (cursor + 1..=cursor + sequences.len() as u64).collect();
    assert_eq!(sequences, expected, "notice sequences skipped or repeated");

    h.kill();
    let ledger = h.ledger();
    let seqs: Vec<i64> = ledger.iter().map(|row| row.seq).collect();
    let gap_free: Vec<i64> = (1..=ledger.len() as i64).collect();
    assert_eq!(seqs, gap_free);
    let mut first_row: HashMap<&str, &str> = HashMap::new();
    for row in &ledger {
        if let Some(correlation) = row.correlation_id.as_deref() {
            first_row
                .entry(correlation)
                .or_insert(row.operation_type.as_str());
        }
    }
    for request_id in &requests {
        assert_eq!(
            first_row.get(request_id.as_str()).copied(),
            Some("agent.call"),
            "request {request_id} has no ledger row correlated to it, or its first is not the call"
        );
    }
    let stored = h.read(|r| {
        let mut stored = 0;
        for request_id in &requests {
            if r.request(&id(request_id))?.is_some() {
                stored += 1;
            }
        }
        Ok(stored)
    });
    assert_eq!(stored, CALLS);
    h.verify();
}
