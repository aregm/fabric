//! The ledger: one row per write, the envelope and its hash chain, append-only tables, failed
//! commits and notice sequences (design 4, 7.2, 9.2 s10-s12 and s14). Replay (s13) is in
//! `replay.rs`.

mod common;

use std::path::Path;

use common::{
    T0, TempDir, agent_row, apply_all, count_rows, expect_fault, has_agent, head_seq, meta,
    new_store, open, preferences, preferences_change, raw, seed_agent, seed_meta, unavailable,
    verify,
};
use fabric_desktop_contracts::canonical;
use fabric_desktop_contracts::digest::sha256_hex;
use fabric_desktop_contracts::owner::Change;
use fabric_storage::{
    Actor, AuditEntry, Mutation, Store, StoreFault, TestKey, TxError, Unavailable, WriteTx,
};
use serde_json::{Value, json};

/// One ledger row as stored.
#[derive(Clone, Debug)]
struct LedgerRow {
    seq: i64,
    operation_id: String,
    operation_type: String,
    payload_version: i64,
    committed_at_ms: i64,
    run_id: Option<i64>,
    actor_kind: String,
    actor_agent_id: Option<String>,
    idempotency_scope: Option<String>,
    idempotency_key: Option<String>,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    payload: String,
    payload_hash: String,
    previous_hash: String,
    envelope_hash: String,
}

fn ledger_rows(connection: &rusqlite::Connection) -> Vec<LedgerRow> {
    let mut statement = connection
        .prepare(
            "SELECT seq, operation_id, operation_type, payload_version, committed_at_ms, run_id,
                    actor_kind, actor_agent_id, idempotency_scope, idempotency_key, causation_id,
                    correlation_id, payload, payload_hash, previous_hash, envelope_hash
             FROM ledger ORDER BY seq",
        )
        .unwrap();
    statement
        .query_map([], |r| {
            Ok(LedgerRow {
                seq: r.get(0)?,
                operation_id: r.get(1)?,
                operation_type: r.get(2)?,
                payload_version: r.get(3)?,
                committed_at_ms: r.get(4)?,
                run_id: r.get(5)?,
                actor_kind: r.get(6)?,
                actor_agent_id: r.get(7)?,
                idempotency_scope: r.get(8)?,
                idempotency_key: r.get(9)?,
                causation_id: r.get(10)?,
                correlation_id: r.get(11)?,
                payload: r.get(12)?,
                payload_hash: r.get(13)?,
                previous_hash: r.get(14)?,
                envelope_hash: r.get(15)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The envelope hash of section 4, computed from the row's columns.
fn envelope_hash(row: &LedgerRow) -> String {
    let idempotency = match (&row.idempotency_scope, &row.idempotency_key) {
        (Some(scope), Some(key)) => json!({ "scope": scope, "key": key }),
        _ => Value::Null,
    };
    let envelope = json!({
        "seq": row.seq,
        "operation_id": row.operation_id,
        "operation_type": row.operation_type,
        "payload_version": row.payload_version,
        "committed_at_ms": row.committed_at_ms,
        "run_id": row.run_id,
        "actor": { "kind": row.actor_kind, "agent_id": row.actor_agent_id },
        "idempotency": idempotency,
        "causation_id": row.causation_id,
        "correlation_id": row.correlation_id,
        "payload_hash": row.payload_hash,
        "previous_hash": row.previous_hash,
    });
    sha256_hex(&format!(
        "fabric-broker-ledger-v1\n{}",
        canonical(&envelope)
    ))
}

/// Runs `sql` on a ledger or audit table with its append-only triggers dropped, then puts the
/// triggers back exactly as they were: the tampering tests change content, not the schema.
fn tamper(connection: &rusqlite::Connection, table: &str, sql: &str) {
    let mut statement = connection
        .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1")
        .unwrap();
    let triggers: Vec<(String, String)> = statement
        .query_map([table], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!triggers.is_empty(), "{table} has no append-only trigger");
    for (name, _) in &triggers {
        connection
            .execute_batch(&format!("DROP TRIGGER {name};"))
            .unwrap();
    }
    connection.execute_batch(sql).unwrap();
    for (_, trigger) in &triggers {
        connection.execute_batch(trigger).unwrap();
    }
}

/// A store with run 1 and three agents written in three writes; returns the store and the
/// sequence of each agent's write.
fn store_with_agents(data_dir: &Path) -> (Store, Vec<u64>) {
    let mut store = new_store(data_dir);
    let seqs = ["agent-one", "agent-two", "agent-three"]
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let name = format!("Agent {}", ["One", "Two", "Three"][i]);
            let at = T0 + 10 * (i64::try_from(i).unwrap() + 1);
            seed_agent(&mut store, agent, &name, at).ledger_seq.unwrap()
        })
        .collect();
    (store, seqs)
}

/// s10 (4, D3): a write that applies mutations appends exactly one ledger row holding all of
/// them, and moves the head by one; a write whose closure refuses rolls back and appends none.
#[test]
fn a_write_appends_one_ledger_row_and_a_refused_write_appends_none() {
    let dir = TempDir::new("s10-write");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let head = head_seq(&store);
    let rows = count_rows(&data, "ledger");

    let committed = apply_all(
        &mut store,
        seed_meta(T0 + 10),
        vec![
            Mutation::AgentPut {
                agent: agent_row("agent-a", 1, "A"),
            },
            Mutation::AgentPut {
                agent: agent_row("agent-b", 1, "B"),
            },
        ],
    )
    .unwrap();
    assert_eq!(committed.ledger_seq, Some(head + 1));
    assert_eq!(head_seq(&store), head + 1);
    assert_eq!(count_rows(&data, "ledger"), rows + 1);
    let last = ledger_rows(&raw(&data)).pop().unwrap();
    let payload: Value = serde_json::from_str(&last.payload).unwrap();
    assert_eq!(payload["mutations"].as_array().unwrap().len(), 2);

    let refused = store.write(seed_meta(T0 + 20), |tx: &mut WriteTx<'_>| {
        tx.apply(Mutation::AgentPut {
            agent: agent_row("agent-c", 1, "C"),
        })?;
        Err::<(), _>(TxError::Refused("refused by the caller"))
    });
    assert!(
        matches!(refused, Err(TxError::Refused("refused by the caller"))),
        "{refused:?}"
    );
    assert_eq!(head_seq(&store), head + 1);
    assert_eq!(count_rows(&data, "ledger"), rows + 1);
    assert!(
        !has_agent(&store, "agent-c"),
        "a refused write left its mutation"
    );
    assert_eq!(verify(&store), head + 1);
}

/// s10 (4): a write that applies no mutation appends no ledger row.
#[test]
fn a_write_without_mutations_appends_no_ledger_row() {
    let dir = TempDir::new("s10-empty");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let head = head_seq(&store);
    let committed = store
        .write(seed_meta(T0 + 10), |tx: &mut WriteTx<'_>| {
            Ok::<_, TxError<()>>(tx.now_ms())
        })
        .unwrap();
    assert_eq!(committed.value, T0 + 10);
    assert_eq!(committed.ledger_seq, None);
    assert_eq!(head_seq(&store), head);
}

/// s10 (4, D24): audit-only writes, and audit rows added inside a write, append no ledger row of
/// their own.
#[test]
fn an_audit_only_write_appends_no_ledger_row() {
    let dir = TempDir::new("s10-audit");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let head = head_seq(&store);
    let rows = count_rows(&data, "ledger");
    let audits = count_rows(&data, "audit");

    let entry = |kind| AuditEntry {
        kind,
        actor: Actor::owner(),
        request_id: None,
        detail: json!({ "redaction": "full" }),
    };
    store
        .write_audit(T0 + 10, vec![entry("access_full"), entry("command_reused")])
        .unwrap();
    assert_eq!(count_rows(&data, "audit"), audits + 2);
    assert_eq!(count_rows(&data, "ledger"), rows);
    assert_eq!(head_seq(&store), head);

    let committed = store
        .write(seed_meta(T0 + 20), |tx: &mut WriteTx<'_>| {
            tx.audit(entry("access_full"))?;
            tx.apply(Mutation::AgentPut {
                agent: agent_row("agent-audited", 1, "Audited"),
            })?;
            Ok::<_, TxError<()>>(())
        })
        .unwrap();
    assert_eq!(committed.ledger_seq, Some(head + 1));
    assert_eq!(count_rows(&data, "audit"), audits + 3);
    assert_eq!(count_rows(&data, "ledger"), rows + 1);
    assert_eq!(verify(&store), head + 1);
}

/// Section 4: each row carries the envelope fields of its write, and its hashes follow the
/// rule: `payload_hash` is SHA-256 of the payload, `previous_hash` links to the row before
/// ("0" x 64 for seq 1), `envelope_hash` covers the envelope.
#[test]
fn ledger_rows_carry_the_envelope_and_hashes_of_section_4() {
    let dir = TempDir::new("s10-envelope");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let mut m = meta(
        "agent.connect",
        Actor::agent(common::id("agent-env")),
        T0 + 10,
    );
    m.causation_id = Some("job-7".to_owned());
    m.correlation_id = Some(common::id("request-7"));
    let seq = apply_all(
        &mut store,
        m,
        vec![Mutation::AgentPut {
            agent: agent_row("agent-env", 1, "Envelope"),
        }],
    )
    .unwrap()
    .ledger_seq
    .unwrap();
    store.close().unwrap();

    let rows = ledger_rows(&raw(&data));
    let mut previous = "0".repeat(64);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.seq, i64::try_from(i).unwrap() + 1, "gap in the ledger");
        assert_eq!(row.previous_hash, previous, "seq {}", row.seq);
        assert_eq!(
            row.payload_hash,
            sha256_hex(&row.payload),
            "seq {}",
            row.seq
        );
        assert_eq!(row.envelope_hash, envelope_hash(row), "seq {}", row.seq);
        let id = row.operation_id.strip_prefix("op-").unwrap();
        assert!(
            id.len() == 32
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "operation id {}",
            row.operation_id
        );
        previous = row.envelope_hash.clone();
    }
    let row = &rows[usize::try_from(seq).unwrap() - 1];
    assert_eq!(row.operation_type, "agent.connect");
    assert_eq!(row.payload_version, 1);
    assert_eq!(row.committed_at_ms, T0 + 10);
    assert_eq!(row.run_id, Some(1));
    assert_eq!(row.actor_kind, "agent");
    assert_eq!(row.actor_agent_id.as_deref(), Some("agent-env"));
    assert_eq!(row.idempotency_scope, None);
    assert_eq!(row.causation_id.as_deref(), Some("job-7"));
    assert_eq!(row.correlation_id.as_deref(), Some("request-7"));
}

/// s11 (3.4 step 8, 4): a ledger row whose content changed breaks its hash; the open store's
/// `verify_chain` names that row, and the next open is `Integrity`, final for the run.
#[test]
fn a_changed_ledger_row_makes_the_store_unavailable() {
    let dir = TempDir::new("s11-changed");
    let data = dir.data_dir();
    let (store, seqs) = store_with_agents(&data);
    let changed = seqs[1];
    let connection = raw(&data);
    tamper(
        &connection,
        "ledger",
        &format!(
            "UPDATE ledger SET payload = replace(payload, 'Agent Two', 'Agent 2') WHERE seq = {changed};"
        ),
    );
    drop(connection);

    match store.verify_chain() {
        Err(StoreFault::Chain { seq, .. }) => assert_eq!(seq, changed),
        other => panic!("expected a chain fault at seq {changed}, got {other:?}"),
    }
    store.close().unwrap();
    let reason = unavailable(common::options(&data, TestKey, T0 + 1_000));
    assert!(
        matches!(reason, Unavailable::Integrity { .. }),
        "{reason:?}"
    );
    assert!(!reason.retry_may_help());
}

/// s11 (3.4 step 8): a ledger cut short at its tail still chains, but no longer reaches the
/// head recorded in `meta`; verification fails and the next open is `Integrity`.
#[test]
fn a_truncated_tail_fails_the_head_check() {
    let dir = TempDir::new("s11-truncated");
    let data = dir.data_dir();
    let (store, seqs) = store_with_agents(&data);
    let last = *seqs.last().unwrap();
    tamper(
        &raw(&data),
        "ledger",
        &format!("DELETE FROM ledger WHERE seq = {last};"),
    );

    assert!(store.verify_chain().is_err(), "a truncated ledger verified");
    store.close().unwrap();
    let reason = unavailable(common::options(&data, TestKey, T0 + 1_000));
    assert!(
        matches!(reason, Unavailable::Integrity { .. }),
        "{reason:?}"
    );
}

/// s11 (3.4 step 8, 4, D34): `committed_at_ms` never decreases along the chain. A last row
/// whose time is set before its predecessor's, with every hash recomputed so that only the time
/// is wrong, fails verification, and the next open is `Integrity`.
#[test]
fn a_ledger_time_that_goes_backward_fails_verification() {
    let dir = TempDir::new("s11-time");
    let data = dir.data_dir();
    let (store, _) = store_with_agents(&data);
    store.close().unwrap();

    let connection = raw(&data);
    let rows = ledger_rows(&connection);
    for row in &rows {
        assert_eq!(
            row.envelope_hash,
            envelope_hash(row),
            "seq {}: the stored envelope hash does not follow section 4",
            row.seq
        );
    }
    let mut last = rows.last().unwrap().clone();
    let before = &rows[rows.len() - 2];
    last.committed_at_ms = before.committed_at_ms - 1;
    last.envelope_hash = envelope_hash(&last);
    tamper(
        &connection,
        "ledger",
        &format!(
            "UPDATE ledger SET committed_at_ms = {}, envelope_hash = '{}' WHERE seq = {};
             UPDATE meta SET value = '{}' WHERE key = 'ledger_head_hash';",
            last.committed_at_ms, last.envelope_hash, last.seq, last.envelope_hash
        ),
    );
    drop(connection);

    let reason = unavailable(common::options(&data, TestKey, T0 + 1_000));
    assert!(
        matches!(reason, Unavailable::Integrity { .. }),
        "{reason:?}"
    );
}

/// s11 (3.5; ADR107 D1): ledger and audit rows cannot be updated or deleted, even by another
/// connection holding the key.
#[test]
fn ledger_and_audit_rows_cannot_be_updated_or_deleted() {
    let dir = TempDir::new("s11-append-only");
    let data = dir.data_dir();
    let (mut store, _) = store_with_agents(&data);
    store
        .write_audit(
            T0 + 100,
            vec![AuditEntry {
                kind: "access_full",
                actor: Actor::owner(),
                request_id: None,
                detail: json!({}),
            }],
        )
        .unwrap();
    store.close().unwrap();

    let connection = raw(&data);
    for sql in [
        "UPDATE ledger SET committed_at_ms = committed_at_ms + 1",
        "DELETE FROM ledger",
        "UPDATE audit SET at_ms = at_ms + 1",
        "DELETE FROM audit",
    ] {
        let error = connection
            .execute_batch(sql)
            .expect_err(&format!("`{sql}` succeeded"));
        assert!(
            error.to_string().contains("append-only"),
            "`{sql}` failed for another reason: {error}"
        );
    }
    drop(connection);
    let store = open(&data, T0 + 1_000);
    verify(&store);
}

/// s12 (5.2, 7.1, 7.2): a commit that fails (an injected trigger on the ledger) rolls the whole
/// write back: no projection change, no ledger row, the head unchanged; once the fault is gone
/// the next write takes the next sequence and the chain verifies.
#[test]
fn a_failed_commit_leaves_no_row_and_the_head_unchanged() {
    let dir = TempDir::new("s12");
    let data = dir.data_dir();
    let (mut store, _) = store_with_agents(&data);
    let head = head_seq(&store);
    let rows = count_rows(&data, "ledger");

    let connection = raw(&data);
    connection
        .execute_batch(
            "CREATE TRIGGER fault BEFORE INSERT ON ledger WHEN NEW.operation_type = 'broker.seed'
             BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();

    let result = apply_all(
        &mut store,
        seed_meta(T0 + 100),
        vec![Mutation::AgentPut {
            agent: agent_row("agent-lost", 1, "Lost"),
        }],
    );
    let fault = expect_fault(result);
    assert!(matches!(fault, StoreFault::Sql { .. }), "{fault:?}");
    assert!(!has_agent(&store, "agent-lost"));
    assert_eq!(head_seq(&store), head);
    assert_eq!(count_rows(&data, "ledger"), rows);

    connection.execute_batch("DROP TRIGGER fault;").unwrap();
    drop(connection);
    let committed = seed_agent(&mut store, "agent-kept", "Kept", T0 + 200);
    assert_eq!(committed.ledger_seq, Some(head + 1));
    assert_eq!(verify(&store), head + 1);
}

/// A write that applies `PreferencesPut` at `revision` and announces it.
fn save_preferences(store: &mut Store, revision: u64, now_ms: i64) -> Vec<u64> {
    let saved = preferences(revision);
    let committed = store
        .write(
            meta("owner.save_preferences", Actor::owner(), now_ms),
            |tx: &mut WriteTx<'_>| {
                tx.apply(Mutation::PreferencesPut {
                    preferences: saved.clone(),
                })?;
                let sequence = tx.notify(preferences_change(&saved))?;
                Ok::<_, TxError<()>>(sequence.get())
            },
        )
        .unwrap();
    let sequences: Vec<u64> = committed.notices.iter().map(|n| n.sequence.get()).collect();
    assert_eq!(sequences, vec![committed.value]);
    sequences
}

/// s14 (5.4, D5): notice sequences rise by one, gap-free, in the order a write announced them;
/// a refused write uses none; the counter is durable and continues after a reopen.
#[test]
fn notice_sequences_rise_by_one_and_survive_reopen() {
    let dir = TempDir::new("s14");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let start = store.read(|r| r.meta()).unwrap().notice_sequence;

    // Two notices in one write, in order.
    let committed = store
        .write(
            meta("owner.save_preferences", Actor::owner(), T0 + 10),
            |tx: &mut WriteTx<'_>| {
                tx.apply(Mutation::PreferencesPut {
                    preferences: preferences(1),
                })?;
                let first = tx.notify(preferences_change(&preferences(1)))?.get();
                let second = tx.notify(Change::RecipientSetDeleted(common::id("rset-gone")))?;
                Ok::<_, TxError<()>>((first, second.get()))
            },
        )
        .unwrap();
    assert_eq!(committed.value, (start + 1, start + 2));
    let sequences: Vec<u64> = committed.notices.iter().map(|n| n.sequence.get()).collect();
    assert_eq!(sequences, vec![start + 1, start + 2]);
    assert!(matches!(
        committed.notices[0].change,
        Change::Preferences(_)
    ));
    assert!(matches!(
        committed.notices[1].change,
        Change::RecipientSetDeleted(_)
    ));

    // A refused write announces nothing and uses no sequence.
    let refused = store.write(
        meta("owner.save_preferences", Actor::owner(), T0 + 20),
        |tx: &mut WriteTx<'_>| {
            tx.apply(Mutation::PreferencesPut {
                preferences: preferences(2),
            })?;
            tx.notify(preferences_change(&preferences(2)))?;
            Err::<(), _>(TxError::Refused(()))
        },
    );
    assert!(matches!(refused, Err(TxError::Refused(()))));

    assert_eq!(save_preferences(&mut store, 2, T0 + 30), vec![start + 3]);
    assert_eq!(store.read(|r| r.meta()).unwrap().notice_sequence, start + 3);
    store.close().unwrap();

    let mut store = open(&data, T0 + 1_000);
    assert_eq!(store.read(|r| r.meta()).unwrap().notice_sequence, start + 3);
    assert_eq!(save_preferences(&mut store, 3, T0 + 1_010), vec![start + 4]);
    verify(&store);
}
