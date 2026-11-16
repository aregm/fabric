//! The ledger: the envelope, its hashes, operation ids, verification and replay (design 4).
//!
//! Every transaction that applies a mutation appends exactly one row; the head in `meta` is
//! updated in the same transaction. `payload` is the canonical JSON `{"mutations":[...]}`.

use fabric_desktop_contracts::digest::sha256_hex;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::migrate::Migration;
use crate::mutation::Mutation;
use crate::tx::{OpMeta, StoreFault, invariant, meta_count, meta_value, set_meta, sql_fault};

/// The previous hash of the first row: 64 zeros.
pub(crate) const GENESIS_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// The domain line every envelope hash starts with.
const DOMAIN: &str = "fabric-broker-ledger-v1\n";

/// The only payload version.
pub(crate) const PAYLOAD_VERSION: i64 = 1;

/// The operation types of section 4. A row of another type fails verification.
pub(crate) const OPERATION_TYPES: [&str; 22] = [
    "store.created",
    "schema.migrated",
    "broker.started",
    "broker.quit",
    "broker.seed",
    "owner.decide_request",
    "owner.pause_agent",
    "owner.revoke_grant",
    "owner.save_profile",
    "owner.save_preferences",
    "owner.save_group",
    "owner.disconnect_google",
    "owner.refused",
    "agent.connect",
    "agent.disconnect",
    "agent.call",
    "timer.session_idle",
    "request.advance",
    "request.proof",
    "request.authorize",
    "request.dispatch",
    "request.attempt",
];

/// The tables that are not projection: replay neither builds nor compares them.
pub(crate) const NOT_PROJECTION: [&str; 3] = ["meta", "ledger", "audit"];

/// "op-" + 32 lowercase hex from 128 random bits.
///
/// # Panics
///
/// When the system random source fails; the store itself uses a fallible form.
#[must_use]
pub fn operation_id() -> String {
    random_id("op-").expect("the system random source gives 16 bytes")
}

/// `prefix` + 32 lowercase hex from 128 random bits.
pub(crate) fn random_id(prefix: &str) -> Result<String, StoreFault> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| invariant("the system random source failed"))?;
    Ok(format!(
        "{prefix}{}",
        fabric_desktop_contracts::digest::hex(&bytes)
    ))
}

/// One ledger row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Envelope {
    pub(crate) seq: u64,
    pub(crate) operation_id: String,
    pub(crate) operation_type: String,
    pub(crate) payload_version: i64,
    pub(crate) committed_at_ms: i64,
    pub(crate) run_id: Option<i64>,
    pub(crate) actor_kind: String,
    pub(crate) actor_agent_id: Option<String>,
    pub(crate) idempotency_scope: Option<String>,
    pub(crate) idempotency_key: Option<String>,
    pub(crate) causation_id: Option<String>,
    pub(crate) correlation_id: Option<String>,
    pub(crate) payload: String,
    pub(crate) payload_hash: String,
    pub(crate) previous_hash: String,
    pub(crate) envelope_hash: String,
}

impl Envelope {
    /// The canonical text the envelope hash covers, after the domain line.
    pub(crate) fn canonical_text(&self) -> String {
        let idempotency = match (&self.idempotency_scope, &self.idempotency_key) {
            (Some(scope), Some(key)) => json!({ "scope": scope, "key": key }),
            _ => Value::Null,
        };
        let value = json!({
            "seq": self.seq,
            "operation_id": self.operation_id,
            "operation_type": self.operation_type,
            "payload_version": self.payload_version,
            "committed_at_ms": self.committed_at_ms,
            "run_id": self.run_id,
            "actor": { "kind": self.actor_kind, "agent_id": self.actor_agent_id },
            "idempotency": idempotency,
            "causation_id": self.causation_id,
            "correlation_id": self.correlation_id,
            "payload_hash": self.payload_hash,
            "previous_hash": self.previous_hash,
        });
        fabric_desktop_contracts::canonical(&value)
    }

    /// `sha256_hex("fabric-broker-ledger-v1\n" + canonical(envelope))`.
    pub(crate) fn computed_hash(&self) -> String {
        sha256_hex(&format!("{DOMAIN}{}", self.canonical_text()))
    }
}

/// The payload of a row: the canonical JSON `{"mutations":[...]}`.
pub(crate) fn payload_text(mutations: &[Mutation]) -> Result<String, StoreFault> {
    let mutations =
        serde_json::to_value(mutations).map_err(|_| invariant("a mutation serializes"))?;
    Ok(fabric_desktop_contracts::canonical(
        &json!({ "mutations": mutations }),
    ))
}

/// Refuses an operation type outside the section 4 list.
pub(crate) fn check_operation_type(operation_type: &str) -> Result<(), StoreFault> {
    if OPERATION_TYPES.contains(&operation_type) {
        Ok(())
    } else {
        Err(invariant("an operation type of the section 4 list"))
    }
}

/// Appends the row of a transaction and moves the head. `run_id` is the run that commits.
pub(crate) fn append(
    connection: &Connection,
    meta: &OpMeta,
    mutations: &[Mutation],
    run_id: Option<i64>,
) -> Result<u64, StoreFault> {
    check_operation_type(meta.operation_type)?;
    let last: Option<i64> = connection
        .query_row(
            "SELECT committed_at_ms FROM ledger ORDER BY seq DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })
        .map_err(|e| sql_fault("read ledger", &e))?;
    if last.is_some_and(|last| meta.now_ms < last) {
        return Err(invariant("ledger time never goes back"));
    }
    let head = meta_count(connection, "ledger_head_seq")?;
    let previous_hash =
        meta_value(connection, "ledger_head_hash")?.unwrap_or_else(|| GENESIS_HASH.to_owned());
    let payload = payload_text(mutations)?;
    let (idempotency_scope, idempotency_key) = match &meta.idempotency {
        Some((scope, key)) => (Some(scope.as_str().to_owned()), Some(key.clone())),
        None => (None, None),
    };
    let mut envelope = Envelope {
        seq: head + 1,
        operation_id: random_id("op-")?,
        operation_type: meta.operation_type.to_owned(),
        payload_version: PAYLOAD_VERSION,
        committed_at_ms: meta.now_ms,
        run_id,
        actor_kind: meta.actor.kind_text().to_owned(),
        actor_agent_id: meta
            .actor
            .agent_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        idempotency_scope,
        idempotency_key,
        causation_id: meta.causation_id.clone(),
        correlation_id: meta
            .correlation_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        payload_hash: sha256_hex(&payload),
        payload,
        previous_hash,
        envelope_hash: String::new(),
    };
    envelope.envelope_hash = envelope.computed_hash();
    let seq = i64::try_from(envelope.seq).map_err(|_| invariant("ledger seq fits in 63 bits"))?;
    connection
        .execute(
            "INSERT INTO ledger (seq, operation_id, operation_type, payload_version, \
             committed_at_ms, run_id, actor_kind, actor_agent_id, idempotency_scope, \
             idempotency_key, causation_id, correlation_id, payload, payload_hash, \
             previous_hash, envelope_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                seq,
                envelope.operation_id,
                envelope.operation_type,
                envelope.payload_version,
                envelope.committed_at_ms,
                envelope.run_id,
                envelope.actor_kind,
                envelope.actor_agent_id,
                envelope.idempotency_scope,
                envelope.idempotency_key,
                envelope.causation_id,
                envelope.correlation_id,
                envelope.payload,
                envelope.payload_hash,
                envelope.previous_hash,
                envelope.envelope_hash,
            ],
        )
        .map_err(|e| sql_fault("append ledger", &e))?;
    set_meta(connection, "ledger_head_seq", &envelope.seq.to_string())?;
    set_meta(connection, "ledger_head_hash", &envelope.envelope_hash)?;
    Ok(envelope.seq)
}

/// Every ledger row in order of `seq`.
fn rows(connection: &Connection) -> Result<Vec<Envelope>, StoreFault> {
    let mut statement = connection
        .prepare(
            "SELECT seq, operation_id, operation_type, payload_version, committed_at_ms, run_id, \
             actor_kind, actor_agent_id, idempotency_scope, idempotency_key, causation_id, \
             correlation_id, payload, payload_hash, previous_hash, envelope_hash \
             FROM ledger ORDER BY seq",
        )
        .map_err(|e| sql_fault("read ledger", &e))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                Envelope {
                    seq: 0,
                    operation_id: row.get(1)?,
                    operation_type: row.get(2)?,
                    payload_version: row.get(3)?,
                    committed_at_ms: row.get(4)?,
                    run_id: row.get(5)?,
                    actor_kind: row.get(6)?,
                    actor_agent_id: row.get(7)?,
                    idempotency_scope: row.get(8)?,
                    idempotency_key: row.get(9)?,
                    causation_id: row.get(10)?,
                    correlation_id: row.get(11)?,
                    payload: row.get(12)?,
                    payload_hash: row.get(13)?,
                    previous_hash: row.get(14)?,
                    envelope_hash: row.get(15)?,
                },
            ))
        })
        .map_err(|e| sql_fault("read ledger", &e))?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, mut envelope) = row.map_err(|e| sql_fault("read ledger", &e))?;
        envelope.seq = u64::try_from(seq).map_err(|_| StoreFault::Chain {
            seq: 0,
            what: "a negative seq",
        })?;
        out.push(envelope);
    }
    Ok(out)
}

/// Recomputes the chain from seq 1: gap-free, payload and envelope hashes, links,
/// non-decreasing time, known types, and the head in `meta`. Returns the head seq.
pub(crate) fn verify_chain(connection: &Connection) -> Result<u64, StoreFault> {
    let mut previous = GENESIS_HASH.to_owned();
    let mut last_time = i64::MIN;
    let mut expected = 1_u64;
    for envelope in rows(connection)? {
        let seq = expected;
        let fail = |what: &'static str| StoreFault::Chain { seq, what };
        if envelope.seq != expected {
            return Err(fail("a gap in seq"));
        }
        if !OPERATION_TYPES.contains(&envelope.operation_type.as_str()) {
            return Err(fail("an unknown operation type"));
        }
        if envelope.payload_version != PAYLOAD_VERSION {
            return Err(fail("an unknown payload version"));
        }
        if sha256_hex(&envelope.payload) != envelope.payload_hash {
            return Err(fail("the payload hash"));
        }
        if envelope.previous_hash != previous {
            return Err(fail("the previous hash"));
        }
        if envelope.computed_hash() != envelope.envelope_hash {
            return Err(fail("the envelope hash"));
        }
        if envelope.committed_at_ms < last_time {
            return Err(fail("time went back"));
        }
        last_time = envelope.committed_at_ms;
        previous = envelope.envelope_hash;
        expected += 1;
    }
    let head = expected - 1;
    let stored_seq = meta_count(connection, "ledger_head_seq")?;
    let stored_hash =
        meta_value(connection, "ledger_head_hash")?.unwrap_or_else(|| GENESIS_HASH.to_owned());
    if stored_seq != head || stored_hash != previous {
        return Err(StoreFault::Chain {
            seq: stored_seq,
            what: "the head in meta",
        });
    }
    Ok(head)
}

/// The mutations of a row's payload.
fn mutations_of(envelope: &Envelope) -> Result<Vec<Mutation>, StoreFault> {
    #[derive(serde::Deserialize)]
    struct Payload {
        mutations: Vec<Mutation>,
    }
    serde_json::from_str::<Payload>(&envelope.payload)
        .map(|p| p.mutations)
        .map_err(|_| StoreFault::Decode {
            table: "ledger",
            column: "payload",
        })
}

/// Rebuilds the projection in an in-memory database from the ledger (running the migration SQL
/// at each `SchemaMigrated`) and compares every projection table with `connection`'s.
pub(crate) fn verify_replay(
    connection: &Connection,
    migrations: &[Migration],
) -> Result<(), StoreFault> {
    let replay = Connection::open_in_memory().map_err(|e| sql_fault("open replay", &e))?;
    replay
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA temp_store = MEMORY;")
        .map_err(|e| sql_fault("open replay", &e))?;
    for envelope in rows(connection)? {
        let seq = envelope.seq;
        for mutation in mutations_of(&envelope)? {
            if let Mutation::SchemaMigrated { from, to } = mutation {
                let migration =
                    migrations
                        .iter()
                        .find(|m| m.to == to)
                        .ok_or(StoreFault::Chain {
                            seq,
                            what: "a migration this build does not have",
                        })?;
                if crate::apply::user_version(&replay)? != from {
                    return Err(StoreFault::Chain {
                        seq,
                        what: "a migration out of order",
                    });
                }
                replay
                    .execute_batch(migration.sql)
                    .map_err(|e| sql_fault("replay migration", &e))?;
                replay
                    .execute_batch(&format!("PRAGMA user_version = {to}"))
                    .map_err(|e| sql_fault("replay migration", &e))?;
            }
            crate::apply::apply(&replay, &mutation).map_err(|_| StoreFault::Chain {
                seq,
                what: "replay refused a mutation",
            })?;
        }
    }
    compare_projection(connection, &replay)
}

/// The projection tables of a database, by name.
fn projection_tables(connection: &Connection) -> Result<Vec<String>, StoreFault> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|e| sql_fault("read schema", &e))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| sql_fault("read schema", &e))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| sql_fault("read schema", &e))?;
    Ok(names
        .into_iter()
        .filter(|name| !NOT_PROJECTION.contains(&name.as_str()))
        .collect())
}

/// Every row of a table, ordered by all its columns.
fn table_rows(
    connection: &Connection,
    table: &str,
) -> Result<Vec<Vec<rusqlite::types::Value>>, StoreFault> {
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let columns = connection
        .prepare(&format!("SELECT * FROM {quoted}"))
        .map_err(|e| sql_fault("read projection", &e))?
        .column_count();
    let order = (1..=columns)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection
        .prepare(&format!("SELECT * FROM {quoted} ORDER BY {order}"))
        .map_err(|e| sql_fault("read projection", &e))?;
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|i| row.get::<_, rusqlite::types::Value>(i))
                .collect()
        })
        .map_err(|e| sql_fault("read projection", &e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| sql_fault("read projection", &e))
}

/// Compares the projection of two databases.
fn compare_projection(live: &Connection, replay: &Connection) -> Result<(), StoreFault> {
    let tables = projection_tables(live)?;
    if tables != projection_tables(replay)? {
        return Err(StoreFault::Invariant {
            what: "replay builds other tables",
        });
    }
    for table in &tables {
        if table_rows(live, table)? != table_rows(replay, table)? {
            return Err(StoreFault::Chain {
                seq: meta_count(live, "ledger_head_seq")?,
                what: replay_differs(table),
            });
        }
    }
    Ok(())
}

/// The fault text of a replay that differs in one table.
fn replay_differs(table: &str) -> &'static str {
    match table {
        "accounts" => "replay differs in accounts",
        "agents" => "replay differs in agents",
        "attempts" => "replay differs in attempts",
        "authorizations" => "replay differs in authorizations",
        "commands" => "replay differs in commands",
        "counters" => "replay differs in counters",
        "effects" => "replay differs in effects",
        "grants" => "replay differs in grants",
        "preferences" => "replay differs in preferences",
        "profile_state" => "replay differs in profile_state",
        "profile_versions" => "replay differs in profile_versions",
        "proof_reports" => "replay differs in proof_reports",
        "recipient_set_versions" => "replay differs in recipient_set_versions",
        "recipient_sets" => "replay differs in recipient_sets",
        "requests" => "replay differs in requests",
        "runs" => "replay differs in runs",
        "sessions" => "replay differs in sessions",
        _ => "replay differs in another table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, T0, TempDir};

    const EMPTY_PAYLOAD_HASH: &str =
        "b6b476062d9a30cbda40129b1db0d7698e9d1524bfeda649d48b4513b453dfc9";

    fn agent_call() -> Envelope {
        Envelope {
            seq: 1,
            operation_id: "op-00112233445566778899aabbccddeeff".to_owned(),
            operation_type: "agent.call".to_owned(),
            payload_version: 1,
            committed_at_ms: 1_795_190_400_000,
            run_id: Some(3),
            actor_kind: "agent".to_owned(),
            actor_agent_id: Some("agent-1".to_owned()),
            idempotency_scope: Some("agent_call".to_owned()),
            idempotency_key: Some("agent-1/call-1".to_owned()),
            causation_id: None,
            correlation_id: Some("req-1".to_owned()),
            payload: r#"{"mutations":[]}"#.to_owned(),
            payload_hash: EMPTY_PAYLOAD_HASH.to_owned(),
            previous_hash: GENESIS_HASH.to_owned(),
            envelope_hash: String::new(),
        }
    }

    #[test]
    fn the_envelope_hash_covers_the_canonical_envelope() {
        // Vectors computed outside the crate with `printf | shasum -a 256`.
        assert_eq!(sha256_hex(r#"{"mutations":[]}"#), EMPTY_PAYLOAD_HASH);
        let envelope = agent_call();
        assert_eq!(
            envelope.canonical_text(),
            format!(
                "{{\"actor\":{{\"agent_id\":\"agent-1\",\"kind\":\"agent\"}},\"causation_id\":null,\
                 \"committed_at_ms\":1795190400000,\"correlation_id\":\"req-1\",\
                 \"idempotency\":{{\"key\":\"agent-1/call-1\",\"scope\":\"agent_call\"}},\
                 \"operation_id\":\"op-00112233445566778899aabbccddeeff\",\
                 \"operation_type\":\"agent.call\",\"payload_hash\":\"{EMPTY_PAYLOAD_HASH}\",\
                 \"payload_version\":1,\"previous_hash\":\"{GENESIS_HASH}\",\"run_id\":3,\"seq\":1}}"
            )
        );
        assert_eq!(
            envelope.computed_hash(),
            "f38e52f15d609c15344c5ee417a4adcd573ce77a8e94ef966630fa6e81bfd547"
        );
    }

    #[test]
    fn a_row_without_actor_agent_idempotency_or_run_hashes_nulls() {
        let envelope = Envelope {
            operation_id: "op-ffeeddccbbaa99887766554433221100".to_owned(),
            operation_type: "schema.migrated".to_owned(),
            run_id: None,
            actor_kind: "broker".to_owned(),
            actor_agent_id: None,
            idempotency_scope: None,
            idempotency_key: None,
            correlation_id: None,
            ..agent_call()
        };
        assert_eq!(
            envelope.computed_hash(),
            "4e68905178efc917d166c2ce85b97fca1a2b0630820089876e67c9601c455f4f"
        );
    }

    #[test]
    fn operation_ids_are_op_and_32_lowercase_hex() {
        let id = operation_id();
        assert_eq!(id.len(), 35);
        assert!(id.starts_with("op-"));
        assert!(
            id[3..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_ne!(id, operation_id());
    }

    /// An in-memory ledger whose rows carry these times, every hash correct.
    fn chain(times: &[i64]) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(crate::MIGRATIONS[0].sql).unwrap();
        let mut previous = GENESIS_HASH.to_owned();
        for (seq, at) in (1_u64..).zip(times) {
            let mut envelope = Envelope {
                seq,
                operation_id: format!("op-{seq:032x}"),
                operation_type: "request.advance".to_owned(),
                committed_at_ms: *at,
                run_id: None,
                actor_kind: "broker".to_owned(),
                actor_agent_id: None,
                idempotency_scope: None,
                idempotency_key: None,
                correlation_id: None,
                previous_hash: previous.clone(),
                ..agent_call()
            };
            envelope.envelope_hash = envelope.computed_hash();
            connection
                .execute(
                    "INSERT INTO ledger (seq, operation_id, operation_type, payload_version, \
                     committed_at_ms, actor_kind, payload, payload_hash, previous_hash, \
                     envelope_hash) VALUES (?1, ?2, ?3, 1, ?4, 'broker', ?5, ?6, ?7, ?8)",
                    params![
                        i64::try_from(seq).unwrap(),
                        envelope.operation_id,
                        envelope.operation_type,
                        envelope.committed_at_ms,
                        envelope.payload,
                        envelope.payload_hash,
                        envelope.previous_hash,
                        envelope.envelope_hash
                    ],
                )
                .unwrap();
            previous = envelope.envelope_hash;
            set_meta(&connection, "ledger_head_seq", &seq.to_string()).unwrap();
            set_meta(&connection, "ledger_head_hash", &previous).unwrap();
        }
        connection
    }

    #[test]
    fn verify_chain_accepts_equal_times_and_refuses_a_time_that_goes_back() {
        assert_eq!(verify_chain(&chain(&[T0, T0, T0 + 1])).unwrap(), 3);
        assert_eq!(
            verify_chain(&chain(&[T0, T0 + 5, T0 + 4])),
            Err(StoreFault::Chain {
                seq: 3,
                what: "time went back"
            })
        );
    }

    #[test]
    fn a_changed_ledger_row_fails_the_chain() {
        let dir = TempDir::new();
        let store = testing::world(&dir);
        store
            .connection()
            .execute_batch(
                "DROP TRIGGER ledger_no_update; \
                 UPDATE ledger SET causation_id = 'edited' WHERE seq = 2;",
            )
            .unwrap();
        assert_eq!(
            store.verify_chain(),
            Err(StoreFault::Chain {
                seq: 2,
                what: "the envelope hash"
            })
        );
    }

    #[test]
    fn a_truncated_tail_fails_the_head_check() {
        let dir = TempDir::new();
        let store = testing::world(&dir);
        store
            .connection()
            .execute_batch("DROP TRIGGER ledger_no_delete; DELETE FROM ledger WHERE seq = 2;")
            .unwrap();
        assert_eq!(
            store.verify_chain(),
            Err(StoreFault::Chain {
                seq: 2,
                what: "the head in meta"
            })
        );
    }

    #[test]
    fn ledger_rows_cannot_be_changed_or_deleted() {
        let dir = TempDir::new();
        let store = testing::store(&dir);
        let update = store
            .connection()
            .execute("UPDATE ledger SET causation_id = 'x' WHERE seq = 1", []);
        let delete = store.connection().execute("DELETE FROM ledger", []);
        assert!(update.is_err() && delete.is_err());
        assert_eq!(store.verify_chain().unwrap(), 1);
    }

    #[test]
    fn a_row_of_an_unknown_type_fails_the_chain() {
        let connection = chain(&[T0]);
        connection
            .execute_batch(
                "DROP TRIGGER ledger_no_update; \
                 UPDATE ledger SET operation_type = 'owner.dance' WHERE seq = 1;",
            )
            .unwrap();
        assert_eq!(
            verify_chain(&connection),
            Err(StoreFault::Chain {
                seq: 1,
                what: "an unknown operation type"
            })
        );
    }
}
