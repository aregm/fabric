//! s17: failures print no key and no content (design 3.1, D37, 9.2).
//!
//! Each failure the store can meet is provoked: a wrong key (macOS), an injected trigger whose
//! message is a marker, JSON columns corrupted to hold the marker, and a backup that cannot be
//! written. The Display and Debug of every resulting error, and the owner message of every
//! `Unavailable`, must hold neither the key's hex nor the marker.

mod common;

use std::fmt::{Debug, Display};
use std::path::Path;

#[cfg(unix)]
use common::open;
use common::{T0, TempDir, agent_row, apply_all, hex, new_store, raw, seed_agent, seed_meta};
#[cfg(unix)]
use fabric_storage::MIGRATIONS;
use fabric_storage::{
    KeyError, KeyKind, Migration, Mutation, OpenError, Store, StoreFault, TEST_KEY, TestKey,
    TxError, Unavailable,
};

/// Content that must never reach an error text.
const MARKER: &str = "FABRIC-W04-SECRET-MARKER-3c9e1d77a0b5";

/// A key that is not the store's (macOS wrong-key case).
const WRONG_KEY: [u8; 32] = [0xc3; 32];

/// The texts no error may contain: the marker and both keys in lower and upper hex.
fn secrets() -> Vec<String> {
    let mut secrets = vec![MARKER.to_owned()];
    for key in [TEST_KEY, WRONG_KEY] {
        secrets.push(hex(&key));
        secrets.push(hex(&key).to_uppercase());
    }
    secrets
}

fn assert_clean(label: &str, text: &str) {
    for secret in secrets() {
        assert!(
            !text.contains(&secret),
            "{label} holds a secret ({} chars): {text}",
            secret.len()
        );
    }
}

/// Checks the Display and the Debug of `error`.
fn assert_error_clean<E: Display + Debug>(label: &str, error: &E) {
    assert_clean(&format!("{label} (Display)"), &format!("{error}"));
    assert_clean(&format!("{label} (Debug)"), &format!("{error:?}"));
}

/// Checks an open error, and the owner message of its reason when it is `Unavailable`.
fn assert_open_error_clean(label: &str, error: &OpenError) {
    assert_error_clean(label, error);
    if let OpenError::Unavailable(reason) = error {
        assert_clean(&format!("{label} (message)"), &reason.message());
        assert_clean(&format!("{label} (reason Debug)"), &format!("{reason:?}"));
    }
}

fn open_result(
    data_dir: &Path,
    migrations: Option<&'static [Migration]>,
) -> Result<Store, OpenError> {
    let options = common::options(data_dir, TestKey, T0 + 100_000);
    match migrations {
        Some(list) => Store::open_with_migrations(options, list),
        None => Store::open(options),
    }
}

#[cfg(unix)]
static WITH_V2: [Migration; 2] = [
    Migration {
        to: 1,
        sql: MIGRATIONS[0].sql,
    },
    Migration {
        to: 2,
        sql: "CREATE TABLE rehearsal (rehearsal_id INTEGER PRIMARY KEY) STRICT;",
    },
];

/// s17 (D37): the store's errors and owner messages never hold the key's hex or
/// row content, whatever failed.
#[test]
fn failures_print_no_key_and_no_content() {
    // Every reason's owner message and every key error's text.
    for reason in [
        Unavailable::InUse,
        Unavailable::KeychainRefused,
        Unavailable::KeychainLocked,
        Unavailable::KeyGeneration,
        Unavailable::KeyMissing,
        Unavailable::WrongKey,
        Unavailable::KeyKindMismatch {
            stored: KeyKind::Test,
        },
        Unavailable::NotEncrypted,
        Unavailable::NewerSchema {
            found: 2,
            supported: 1,
        },
        Unavailable::Integrity {
            check: "ledger chain",
        },
    ] {
        assert_open_error_clean("unavailable", &OpenError::Unavailable(reason));
    }
    for error in [
        KeyError::Refused { status: -128 },
        KeyError::Locked { status: -25308 },
        KeyError::Failed { status: -1 },
        KeyError::Random,
    ] {
        assert_error_clean("key error", &error);
        assert_clean("key error message", &error.unavailable().message());
    }

    // A wrong key (macOS; elsewhere the test store is plaintext and takes no key).
    #[cfg(target_os = "macos")]
    {
        let dir = TempDir::new("s17-wrong-key");
        let data = dir.data_dir();
        new_store(&data).close().unwrap();
        let wrong = common::ScriptedKey::holding(KeyKind::Test, WRONG_KEY);
        match Store::open(common::options(&data, wrong, T0 + 1_000)) {
            Ok(_) => panic!("a wrong key opened the store"),
            Err(error) => assert_open_error_clean("wrong key", &error),
        }
    }

    // An injected trigger whose message is the marker, on a write that carries the marker.
    let dir = TempDir::new("s17-trigger");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let connection = raw(&data);
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER fault BEFORE INSERT ON ledger BEGIN SELECT RAISE(ABORT, '{MARKER}'); END;"
        ))
        .unwrap();
    let result = apply_all(
        &mut store,
        seed_meta(T0 + 10),
        vec![Mutation::AgentPut {
            agent: agent_row("agent-marked", 1, MARKER),
        }],
    );
    match &result {
        Err(error @ TxError::Fault(fault)) => {
            assert_error_clean("injected trigger", fault);
            assert_clean("injected trigger (TxError)", &format!("{error:?}"));
        }
        other => panic!("expected the injected trigger to fail the write, got {other:?}"),
    }
    connection.execute_batch("DROP TRIGGER fault;").unwrap();
    drop(connection);
    drop(store);

    // JSON columns corrupted to hold the marker: a projection record and a ledger payload. Each
    // is the bare JSON string of the marker, which serde quotes in its error ("invalid type:
    // string \"<marker>\", expected ..."), so an error text that passed serde's message on fails.
    let dir = TempDir::new("s17-json");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    seed_agent(&mut store, "agent-plain", "Plain", T0 + 10);
    let connection = raw(&data);
    connection
        .execute_batch(&format!(
            "UPDATE agents SET record = '\"{MARKER}\"' WHERE agent_id = 'agent-plain';"
        ))
        .unwrap();
    let mut faults: Vec<StoreFault> = Vec::new();
    match store.read(|r| r.agent(&common::id("agent-plain"))) {
        Ok(agent) => panic!("a corrupted record decoded: {agent:?}"),
        Err(fault) => faults.push(fault),
    }
    if let Err(fault) = store.read(|r| r.agents()) {
        faults.push(fault);
    }
    match store.verify_replay() {
        Ok(()) => panic!("replay accepted a corrupted record"),
        Err(fault) => faults.push(fault),
    }
    for fault in &faults {
        assert_error_clean("corrupted record", fault);
    }
    store.close().unwrap();

    let dir = TempDir::new("s17-payload");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    let seq = seed_agent(&mut store, "agent-plain", "Plain", T0 + 10)
        .ledger_seq
        .unwrap();
    store.close().unwrap();
    let connection = raw(&data);
    connection
        .execute_batch(&format!(
            "DROP TRIGGER ledger_no_update;
             UPDATE ledger SET payload = '\"{MARKER}\"' WHERE seq = {seq};"
        ))
        .unwrap();
    drop(connection);
    match open_result(&data, None) {
        Ok(store) => match store.verify_chain() {
            Ok(head) => panic!("a corrupted payload verified up to {head}"),
            Err(fault) => assert_error_clean("corrupted payload", &fault),
        },
        Err(error) => assert_open_error_clean("corrupted payload", &error),
    }

    // A backup that cannot be written (Unix: a read-only backup directory). 3.6: the store is
    // unavailable with `Integrity { check: "backup" }` and the store file is unchanged.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("s17-backup");
        let data = dir.data_dir();
        let mut store = new_store(&data);
        seed_agent(&mut store, "agent-backed", MARKER, T0 + 10);
        store.close().unwrap();
        let backups = data.join(fabric_storage::BACKUP_DIR);
        std::fs::create_dir(&backups).unwrap();
        std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o500)).unwrap();
        let before = common::database_bytes(&data);
        match open_result(&data, Some(&WITH_V2)) {
            Ok(_) => panic!("the migration ran without its backup"),
            Err(error) => {
                assert_open_error_clean("failed backup", &error);
                assert_eq!(
                    error,
                    OpenError::Unavailable(Unavailable::Integrity { check: "backup" })
                );
            }
        }
        std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            common::database_bytes(&data),
            before,
            "a failed backup changed the store file"
        );
        let store = open(&data, T0 + 200_000);
        assert_eq!(store.read(|r| r.meta()).unwrap().schema_version, 1);
    }
}
