//! Migrations, backups and the frozen schema (design 3.6, 9.2 s08 and s09).
//!
//! The v1 store is built at test time through `Store::open` with the test key and ordinary
//! writes; a test-only migration 2 runs through `Store::open_with_migrations` (no hand-written
//! fixture database; section 9.2 s08).

mod common;

use std::path::{Path, PathBuf};

use common::{
    T0, TempDir, apply_all, has_agent, make_data_dir, new_store, open, preferences, query_i64, raw,
    seed_agent, seed_meta, verify,
};
use fabric_storage::{
    BACKUP_DIR, DATABASE_FILE, MIGRATIONS, Migration, Mutation, OpenOptions, SCHEMA_VERSION, Store,
    TestKey,
};
use serde_json::Value;

/// The released migration 1 followed by a test-only migration 2.
static WITH_V2: [Migration; 2] = [
    Migration {
        to: 1,
        sql: MIGRATIONS[0].sql,
    },
    Migration {
        to: 2,
        sql: "CREATE TABLE rehearsal (rehearsal_id INTEGER PRIMARY KEY, note TEXT NOT NULL) STRICT;",
    },
];

fn options(data_dir: &Path, now_ms: i64) -> OpenOptions {
    common::options(data_dir, TestKey, now_ms)
}

fn open_v2(data_dir: &Path, now_ms: i64) -> Store {
    match Store::open_with_migrations(options(data_dir, now_ms), &WITH_V2) {
        Ok(store) => store,
        Err(e) => panic!("the store did not open with migration 2: {e} ({e:?})"),
    }
}

/// What a v1 store looked like before its migration.
struct V1 {
    store_id: String,
    head: u64,
    last_commit_ms: i64,
}

/// A closed v1 store with a run, two agents and saved preferences; its last commit is at
/// `last_commit_ms`.
fn v1_store(data_dir: &Path, last_commit_ms: i64) -> V1 {
    let mut store = new_store(data_dir);
    seed_agent(&mut store, "agent-one", "Agent One", T0 + 10);
    seed_agent(&mut store, "agent-two", "Agent Two", T0 + 20);
    apply_all(
        &mut store,
        seed_meta(last_commit_ms),
        vec![Mutation::PreferencesPut {
            preferences: preferences(1),
        }],
    )
    .unwrap();
    let meta = store.read(|r| r.meta()).unwrap();
    assert_eq!(meta.schema_version, 1);
    let v1 = V1 {
        store_id: store.store_id().to_owned(),
        head: meta.ledger_head_seq,
        last_commit_ms: store.read(|r| r.last_committed_ms()).unwrap().unwrap(),
    };
    assert_eq!(v1.last_commit_ms, last_commit_ms);
    store.close().unwrap();
    v1
}

/// The files in the data directory's backup directory.
fn backups(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(data_dir.join(BACKUP_DIR)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

fn only_backup(data_dir: &Path) -> PathBuf {
    let files = backups(data_dir);
    assert_eq!(files.len(), 1, "expected one backup, found {files:?}");
    files.into_iter().next().unwrap()
}

/// The last ledger row: its operation type, committed time and payload.
fn last_ledger_row(data_dir: &Path) -> (u64, String, i64, Value) {
    let connection = raw(data_dir);
    connection
        .query_row(
            "SELECT seq, operation_type, committed_at_ms, payload FROM ledger ORDER BY seq DESC LIMIT 1",
            [],
            |r| {
                let payload: String = r.get(3)?;
                Ok((
                    u64::try_from(r.get::<_, i64>(0)?).unwrap(),
                    r.get(1)?,
                    r.get(2)?,
                    serde_json::from_str(&payload).unwrap(),
                ))
            },
        )
        .unwrap()
}

fn table_exists(connection: &rusqlite::Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

/// s08 (3.6): a v1 store migrates to version 2 in one transaction that sets `user_version`
/// and `meta.schema_version` and appends a `schema.migrated` row `{from 1, to 2}`, after one
/// backup; the data is kept and the chain and the replay verify.
#[test]
fn the_v1_store_migrates_with_a_verified_backup() {
    let dir = TempDir::new("s08-migrate");
    let data = dir.data_dir();
    let v1 = v1_store(&data, T0 + 1_000);

    let store = open_v2(&data, T0 + 100_000);
    let meta = store.read(|r| r.meta()).unwrap();
    assert_eq!(meta.schema_version, 2);
    assert_eq!(meta.ledger_head_seq, v1.head + 1);
    assert!(has_agent(&store, "agent-one"));
    assert!(has_agent(&store, "agent-two"));
    assert_eq!(verify(&store), v1.head + 1);

    let (seq, operation_type, _, payload) = last_ledger_row(&data);
    assert_eq!(seq, v1.head + 1);
    assert_eq!(operation_type, "schema.migrated");
    let mutations = payload["mutations"].as_array().unwrap();
    assert!(
        mutations
            .iter()
            .any(|m| m["op"] == "schema_migrated" && m["from"] == 1 && m["to"] == 2),
        "no schema_migrated 1 -> 2 in {payload}"
    );

    let connection = raw(&data);
    assert_eq!(query_i64(&connection, "PRAGMA user_version"), 2);
    assert!(table_exists(&connection, "rehearsal"));
    drop(connection);
    only_backup(&data);
}

/// s08 (3.6; PLAN 11 para 8): the backup is encrypted with the store key (macOS), opens with
/// that key, and as a store verifies its chain up to the pre-migration head and its replay.
#[test]
fn the_backup_opens_with_the_same_key_and_verifies_chain_and_replay() {
    let dir = TempDir::new("s08-backup");
    let data = dir.data_dir();
    let v1 = v1_store(&data, T0 + 1_000);
    open_v2(&data, T0 + 100_000).close().unwrap();
    let backup = only_backup(&data);

    #[cfg(target_os = "macos")]
    {
        let bytes = std::fs::read(&backup).unwrap();
        assert_ne!(
            &bytes[..16],
            b"SQLite format 3\0",
            "the backup is plaintext"
        );
        let wrong = rusqlite::Connection::open(&backup).unwrap();
        common::key_connection(&wrong, &[0x47; 32]);
        assert!(
            wrong
                .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                    .get::<_, i64>(0))
                .is_err(),
            "another key opened the backup"
        );
    }

    let connection =
        rusqlite::Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    common::key_connection(&connection, &fabric_storage::TEST_KEY);
    assert_eq!(query_i64(&connection, "PRAGMA user_version"), 1);
    assert_eq!(
        query_i64(&connection, "SELECT count(*) FROM ledger"),
        i64::try_from(v1.head).unwrap()
    );
    drop(connection);

    // As a store of its own, the backup verifies its chain and its replay.
    let copy = TempDir::new("s08-backup-copy");
    let copy_data = copy.data_dir();
    make_data_dir(&copy_data);
    std::fs::copy(&backup, copy_data.join(DATABASE_FILE)).unwrap();
    let store = open(&copy_data, T0 + 200_000);
    assert_eq!(store.store_id(), v1.store_id);
    assert_eq!(verify(&store), v1.head);
}

/// s08 (3.6): the backup's name is `broker-v{n}-{store_id}-h{ledger_head_seq}-{at_ms}.db`,
/// binding the schema and the ledger watermark it holds.
#[test]
fn the_backup_name_binds_schema_and_ledger_head() {
    let dir = TempDir::new("s08-name");
    let data = dir.data_dir();
    let v1 = v1_store(&data, T0 + 1_000);
    let now_ms = T0 + 100_000;
    open_v2(&data, now_ms).close().unwrap();

    let backup = only_backup(&data);
    let name = backup.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        name,
        format!("broker-v1-{}-h{}-{now_ms}.db", v1.store_id, v1.head)
    );
}

/// s08 (3.6; PLAN 11 para 8): a downgrade is restoring the backup by hand; the restored file
/// opens as version 1 with the data and the ledger it had before the migration.
#[test]
fn restoring_the_backup_reopens_version_1() {
    let dir = TempDir::new("s08-restore");
    let data = dir.data_dir();
    let v1 = v1_store(&data, T0 + 1_000);
    open_v2(&data, T0 + 100_000).close().unwrap();
    let backup = only_backup(&data);

    for suffix in ["", "-wal", "-shm"] {
        let path = data.join(format!("{DATABASE_FILE}{suffix}"));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    std::fs::copy(&backup, data.join(DATABASE_FILE)).unwrap();

    let store = open(&data, T0 + 200_000);
    let meta = store.read(|r| r.meta()).unwrap();
    assert_eq!(meta.schema_version, SCHEMA_VERSION);
    assert_eq!(meta.ledger_head_seq, v1.head);
    assert_eq!(store.store_id(), v1.store_id);
    assert!(has_agent(&store, "agent-one"));
    assert_eq!(verify(&store), v1.head);
    store.close().unwrap();
    assert!(!table_exists(&raw(&data), "rehearsal"));
}

/// s08 (3.6, D34): a migration whose wall clock is behind the last commit dates
/// its `schema.migrated` row and its backup at the last commit, so ledger time never decreases
/// and the next start verifies the chain.
#[test]
fn a_migration_with_a_clock_behind_the_last_commit_keeps_ledger_time_non_decreasing() {
    let dir = TempDir::new("s08-clock");
    let data = dir.data_dir();
    let v1 = v1_store(&data, T0 + 10_000);

    let store = open_v2(&data, T0);
    store.close().unwrap();
    let (seq, operation_type, committed_at_ms, _) = last_ledger_row(&data);
    assert_eq!(seq, v1.head + 1);
    assert_eq!(operation_type, "schema.migrated");
    assert_eq!(committed_at_ms, T0 + 10_000);
    let name = only_backup(&data);
    assert!(
        name.to_str()
            .unwrap()
            .ends_with(&format!("-{}.db", T0 + 10_000)),
        "the backup name is not dated at the last commit: {name:?}"
    );

    let store = open_v2(&data, T0);
    assert_eq!(verify(&store), v1.head + 1);
}

/// The normalized form of one schema statement: SQL comments removed, whitespace collapsed, no
/// space next to a parenthesis or a comma. The fixture was written with the same rule.
fn normalize(sql: &str) -> String {
    let without_comments: Vec<&str> = sql
        .split('\n')
        .map(|line| line.find("--").map_or(line, |i| &line[..i]))
        .collect();
    let collapsed = without_comments
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    let tight = |c: char| matches!(c, '(' | ')' | ',');
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| !(c == ' ' && (tight(chars[i - 1]) || tight(chars[i + 1]))))
        .map(|(_, &c)| c)
        .collect()
}

/// The normalized `sqlite_schema` of `connection`, in the fixture's format.
fn schema_dump(connection: &rusqlite::Connection) -> String {
    let mut statement = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")
        .unwrap();
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .unwrap();
    let mut out = String::new();
    for row in rows {
        let (kind, name, table, sql) = row.unwrap();
        out.push_str(&format!("-- {kind} {name} on {table}\n"));
        if let Some(sql) = sql {
            out.push_str(&normalize(&sql));
            out.push_str(";\n");
        }
    }
    out
}

/// The fixture without its header (everything up to the first blank line).
fn frozen_schema() -> String {
    schema_of(include_str!("fixtures/schema-v1.sql"))
}

/// The schema part of the fixture's text, with line ends made LF: a checkout may turn them into
/// CRLF (Windows, `core.autocrlf`).
fn schema_of(fixture: &str) -> String {
    let fixture = fixture.replace("\r\n", "\n");
    fixture.split_once("\n\n").unwrap().1.to_owned()
}

#[test]
fn the_frozen_schema_reads_the_same_from_a_crlf_checkout() {
    let fixture = include_str!("fixtures/schema-v1.sql");
    assert_eq!(schema_of(&fixture.replace('\n', "\r\n")), frozen_schema());
}

/// s09 (3.5, 3.6): released migration SQL is frozen. Migration 1 on an empty database, and a
/// store created by `Store::open`, have exactly the schema of `tests/fixtures/schema-v1.sql`.
#[test]
fn migration_1_matches_the_frozen_schema() {
    assert_eq!(MIGRATIONS[0].to, 1);
    let empty = rusqlite::Connection::open_in_memory().unwrap();
    empty.execute_batch(MIGRATIONS[0].sql).unwrap();
    assert_eq!(schema_dump(&empty), frozen_schema(), "migration 1 SQL");

    let dir = TempDir::new("s09");
    let data = dir.data_dir();
    open(&data, T0).close().unwrap();
    assert_eq!(schema_dump(&raw(&data)), frozen_schema(), "a created store");
}
