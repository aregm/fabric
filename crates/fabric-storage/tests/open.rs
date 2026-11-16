//! Opening the store: pragmas, keys, the lock and the schema version (design 3.1-3.4, 9.2
//! s01, s04-s07). The encryption tests of macOS (s02, s03) are in `encryption.rs`.

mod common;

use std::time::{Duration, Instant};

use common::{
    ScriptedKey, T0, TempDir, agent_row, apply_all, count_rows, database_bytes, expect_fault,
    has_agent, new_store, open, open_with, options, raw, seed_agent, seed_meta, unavailable,
};
use fabric_storage::{
    BACKUP_DIR, KeyKind, LOCK_FILE, Mutation, SCHEMA_VERSION, StoreFault, TEST_KEY, TestKey,
    Unavailable,
};

/// s01 (3.1): the read-write connection runs in WAL mode with a busy timeout of 250 ms, and
/// every open sets them again. The other pragmas of 3.1 are per connection and not visible from
/// outside; the store's unit tests assert them.
#[test]
fn pragmas_hold_on_every_connection() {
    let dir = TempDir::new("s01");
    let data = dir.data_dir();
    for round in 0..2_i64 {
        let mut store = if round == 0 {
            new_store(&data)
        } else {
            open(&data, T0 + 60_000)
        };
        let now = T0 + 60_000 * round + 10;
        seed_agent(&mut store, &format!("agent-{round}"), "Agent", now);

        // journal_mode = WAL: the write went to the -wal file, and the database says wal.
        assert!(
            data.join("broker.db-wal").exists(),
            "open {round}: no -wal file while the store is open"
        );
        let other = raw(&data);
        let mode: String = other
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal", "open {round}");

        // busy_timeout = 250: a write waits about 250 ms on a held write lock, then fails as Busy.
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let started = Instant::now();
        let result = apply_all(
            &mut store,
            seed_meta(now + 1),
            vec![Mutation::AgentPut {
                agent: agent_row(&format!("agent-busy-{round}"), 1, "Busy"),
            }],
        );
        let waited = started.elapsed();
        other.execute_batch("ROLLBACK").unwrap();
        assert_eq!(expect_fault(result), StoreFault::Busy, "open {round}");
        assert!(
            waited >= Duration::from_millis(200),
            "open {round}: the write gave up after {waited:?}, before the 250 ms busy timeout"
        );
        assert!(
            waited < Duration::from_secs(5),
            "open {round}: the write waited {waited:?}"
        );
        drop(other);
        store.close().unwrap();
    }
}

/// s04 (3.2, 3.4 steps 3-5): an existing store whose key is gone is `KeyMissing`; no new key is
/// made over it and the file is byte-identical.
#[test]
fn an_existing_store_without_its_key_is_key_missing_and_unchanged() {
    let dir = TempDir::new("s04");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    seed_agent(&mut store, "agent-kept", "Kept", T0 + 10);
    store.close().unwrap();
    let before = database_bytes(&data);

    let key = ScriptedKey::empty(KeyKind::Test);
    assert_eq!(
        unavailable(options(&data, key.clone(), T0 + 1_000)),
        Unavailable::KeyMissing
    );
    assert_eq!(key.creates(), 0, "a key was created over an existing store");
    assert_eq!(
        database_bytes(&data),
        before,
        "the refused open changed broker.db"
    );

    // With its key the store opens as it was.
    let store = open(&data, T0 + 2_000);
    assert!(has_agent(&store, "agent-kept"));
}

/// s04b (3.2): a key left by a crash between creating the key and creating the
/// file is reused; `create` is never called, neither then nor on later opens.
#[test]
fn a_key_left_without_a_store_is_reused_and_never_created_again() {
    let dir = TempDir::new("s04b");
    let data = dir.data_dir();
    let left = [0x5a; 32];

    let key = ScriptedKey::holding(KeyKind::Test, left);
    let mut store = open_with(options(&data, key.clone(), T0));
    assert_eq!(
        key.creates(),
        0,
        "a new key was created although one existed"
    );
    common::start_run(&mut store, 1, T0);
    seed_agent(&mut store, "agent-first", "First", T0 + 10);
    store.close().unwrap();

    let again = ScriptedKey::holding(KeyKind::Test, left);
    let store = open_with(options(&data, again.clone(), T0 + 1_000));
    assert_eq!(again.creates(), 0);
    assert!(has_agent(&store, "agent-first"));
    store.close().unwrap();

    // The store is keyed with the key that was left: another key does not open it.
    #[cfg(target_os = "macos")]
    assert_eq!(
        unavailable(options(
            &data,
            ScriptedKey::holding(KeyKind::Test, TEST_KEY),
            T0 + 2_000
        )),
        Unavailable::WrongKey
    );
}

/// s05 (3.2, 3.4 step 8): a provider of another kind than the one the store was created with is
/// refused, and the store is left as it was.
#[test]
fn a_store_refuses_a_key_of_another_kind() {
    let dir = TempDir::new("s05");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    seed_agent(&mut store, "agent-kind", "Kind", T0 + 10);
    store.close().unwrap();

    let keychain_kind = ScriptedKey::holding(KeyKind::Keychain, TEST_KEY);
    assert_eq!(
        unavailable(options(&data, keychain_kind.clone(), T0 + 1_000)),
        Unavailable::KeyKindMismatch {
            stored: KeyKind::Test
        }
    );
    assert_eq!(keychain_kind.creates(), 0);

    let store = open_with(options(&data, TestKey, T0 + 2_000));
    assert_eq!(store.key_kind(), KeyKind::Test);
    assert!(has_agent(&store, "agent-kind"));
}

/// s05b (3.2): with the bytes real providers hold (a Keychain key is random, never the test
/// key), a test-key store opened by a Keychain-kind provider is `KeyKindMismatch` both when the
/// provider has no key and when it holds its own; a Keychain-kind store whose key is gone stays
/// `KeyMissing`. Nothing is created and the files are unchanged.
#[test]
fn a_test_store_refuses_the_keychain_kind_whatever_key_it_holds() {
    let dir = TempDir::new("s05b");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    seed_agent(&mut store, "agent-kind", "Kind", T0 + 10);
    store.close().unwrap();
    let before = database_bytes(&data);
    for keychain in [
        ScriptedKey::empty(KeyKind::Keychain),
        ScriptedKey::holding(KeyKind::Keychain, [0x5a; 32]),
    ] {
        assert_eq!(
            unavailable(options(&data, keychain.clone(), T0 + 1_000)),
            Unavailable::KeyKindMismatch {
                stored: KeyKind::Test
            }
        );
        assert_eq!(keychain.creates(), 0);
    }
    assert_eq!(database_bytes(&data), before);

    let dir = TempDir::new("s05b-keychain");
    let data = dir.data_dir();
    let mut store = open_with(options(
        &data,
        ScriptedKey::holding(KeyKind::Keychain, [0x5a; 32]),
        T0,
    ));
    seed_agent(&mut store, "agent-kind", "Kind", T0 + 10);
    store.close().unwrap();
    let gone = ScriptedKey::empty(KeyKind::Keychain);
    assert_eq!(
        unavailable(options(&data, gone.clone(), T0 + 1_000)),
        Unavailable::KeyMissing
    );
    assert_eq!(gone.creates(), 0);
}

/// s06 (3.3, 3.4 step 2): one process opens a data directory at a time; a second open while the
/// first holds `broker.lock` is `InUse`, and the lock is free once the first store is gone.
#[test]
fn a_second_open_of_the_same_directory_is_in_use() {
    let dir = TempDir::new("s06");
    let data = dir.data_dir();
    let mut first = new_store(&data);
    seed_agent(&mut first, "agent-holder", "Holder", T0 + 10);
    assert!(
        data.join(LOCK_FILE).exists(),
        "no lock file in the data directory"
    );

    assert_eq!(
        unavailable(options(&data, TestKey, T0 + 1_000)),
        Unavailable::InUse
    );
    assert!(Unavailable::InUse.retry_may_help());

    // The refused open did not disturb the first store.
    seed_agent(&mut first, "agent-after", "After", T0 + 2_000);
    first.close().unwrap();

    let second = open(&data, T0 + 3_000);
    assert!(has_agent(&second, "agent-holder"));
    assert!(has_agent(&second, "agent-after"));
}

/// s07 (3.4 step 5): a store written by a newer schema is refused before anything is read or
/// written: the file is byte-identical and no backup is taken.
#[test]
fn a_newer_schema_is_refused_and_the_file_is_unchanged() {
    let dir = TempDir::new("s07");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    seed_agent(&mut store, "agent-old", "Old", T0 + 10);
    store.close().unwrap();

    // A newer build's store: a raw connection sets user_version 2 and closes cleanly.
    let connection = raw(&data);
    connection
        .execute_batch("PRAGMA user_version = 2;")
        .unwrap();
    drop(connection);
    let rows_before = count_rows(&data, "ledger");
    let before = database_bytes(&data);

    assert_eq!(
        unavailable(options(&data, TestKey, T0 + 1_000)),
        Unavailable::NewerSchema {
            found: 2,
            supported: SCHEMA_VERSION
        }
    );
    assert_eq!(
        database_bytes(&data),
        before,
        "the refused open changed broker.db"
    );
    assert_eq!(count_rows(&data, "ledger"), rows_before);
    assert!(
        !data.join(BACKUP_DIR).exists()
            || std::fs::read_dir(data.join(BACKUP_DIR))
                .unwrap()
                .next()
                .is_none(),
        "a backup was taken of a store this build cannot read"
    );
}
