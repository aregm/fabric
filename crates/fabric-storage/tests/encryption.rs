//! Encryption on macOS (design 3.1, 3.4, 9.2 s02 and s03). Elsewhere the test store is plain
//! SQLite by design (D10), so this file is empty there.
//!
//! The scan of open files is a port of the storage spike's `cmd_encrypt`: it reads every
//! regular file the process holds open through `/dev/fd`, which also sees unlinked temporary
//! files, and every file in the data directory.

#![cfg(target_os = "macos")]

mod common;

use std::fs::File;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::Path;

use common::{
    ScriptedKey, T0, TempDir, apply_all, database_bytes, has_agent, make_data_dir, new_store, open,
    options, raw, seed_meta, unavailable,
};
use fabric_storage::{Actor, AuditEntry, DATABASE_FILE, KeyKind, Mutation, TestKey, Unavailable};
use serde_json::json;

/// A string that must never appear in plaintext on disk.
const MARKER: &str = "FABRIC-W04-PLAINTEXT-MARKER-7f3a9c1e5b2d4086-DO-NOT-LEAK";

/// The first 16 bytes of a plaintext SQLite file.
const PLAINTEXT_HEADER: &[u8; 16] = b"SQLite format 3\0";

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The regular files this process holds open that contain the marker, read with `pread` so a
/// shared offset does not matter.
fn open_files_with_marker() -> Vec<String> {
    let mut hits = Vec::new();
    for fd in 3..1024 {
        let path = format!("/dev/fd/{fd}");
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let Ok(metadata) = file.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let mut buffer = vec![0_u8; usize::try_from(metadata.len()).unwrap()];
        let n = file.read_at(&mut buffer, 0).unwrap_or(0);
        if contains(&buffer[..n], MARKER.as_bytes()) {
            hits.push(format!("fd {fd} ({} bytes)", metadata.len()));
        }
    }
    hits
}

/// The files of `dir` that contain the marker.
fn files_with_marker(dir: &Path) -> Vec<String> {
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() && contains(&std::fs::read(&path).unwrap(), MARKER.as_bytes()) {
            hits.push(path.display().to_string());
        }
    }
    hits
}

/// s02 (3.1; R12, R13): with the store open, after writes and reads that hold the marker, no
/// byte of `broker.db`, `-wal`, `-shm`, any other file of the data directory or any file the
/// process holds open contains it; nor after close. A control shows the scanner sees the marker
/// in a plaintext file, and a keyed second connection reads it back from the store.
#[test]
fn no_plaintext_in_db_wal_shm_or_open_files() {
    let dir = TempDir::new("s02-plain");

    // Control: the scanner finds the marker in a plaintext file this process holds open.
    let control_path = dir.path().join("control.txt");
    // Read and write, as SQLite opens its files: `/dev/fd` reopens a descriptor only in a mode
    // the descriptor allows.
    let mut control = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&control_path)
        .unwrap();
    control.write_all(MARKER.as_bytes()).unwrap();
    control.flush().unwrap();
    assert!(
        !open_files_with_marker().is_empty(),
        "control: the scanner does not see an open plaintext file"
    );
    drop(control);
    std::fs::remove_file(&control_path).unwrap();
    assert!(
        open_files_with_marker().is_empty(),
        "control file still open"
    );

    let data = dir.data_dir();
    let mut store = new_store(&data);
    for i in 0..200 {
        let name = format!("{MARKER} {i}");
        apply_all(
            &mut store,
            seed_meta(T0 + 10 + i),
            vec![Mutation::AgentPut {
                agent: common::agent_row(&format!("agent-{i}"), 1, &name),
            }],
        )
        .unwrap();
    }
    let entries = (0..2_000)
        .map(|i| AuditEntry {
            kind: "access_test",
            actor: Actor::owner(),
            request_id: None,
            detail: json!({ "note": format!("{MARKER} row {i} {}", "x".repeat(200)) }),
        })
        .collect();
    store.write_audit(T0 + 5_000, entries).unwrap();
    let agents = store.read(|r| r.agents()).unwrap();
    assert_eq!(agents.len(), 200);

    assert!(data.join("broker.db-wal").exists());
    assert_eq!(files_with_marker(&data), Vec::<String>::new(), "store open");
    assert_eq!(open_files_with_marker(), Vec::<String>::new(), "store open");

    store.close().unwrap();
    assert_eq!(
        files_with_marker(&data),
        Vec::<String>::new(),
        "store closed"
    );
    assert_ne!(&database_bytes(&data)[..16], PLAINTEXT_HEADER);

    // The marker is in the store: a connection with the key reads it back.
    let connection = raw(&data);
    let found: i64 = connection
        .query_row(
            "SELECT count(*) FROM agents WHERE instr(record, ?1) > 0",
            [MARKER],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(found, 200);
}

/// s02 (3.1): the linked SQLite is SQLCipher with the codec and `SQLITE_TEMP_STORE=3`
/// (temporary data only in memory).
#[test]
fn compile_options_have_codec_and_temp_store_3() {
    let dir = TempDir::new("s02-options");
    let data = dir.data_dir();
    new_store(&data).close().unwrap();
    let connection = raw(&data);
    let mut statement = connection.prepare("PRAGMA compile_options").unwrap();
    let options: Vec<String> = statement
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        options.iter().any(|o| o == "HAS_CODEC"),
        "no HAS_CODEC in {options:?}"
    );
    assert!(
        options.iter().any(|o| o == "TEMP_STORE=3"),
        "no TEMP_STORE=3 in {options:?}"
    );
    let version: Option<String> = connection
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .ok();
    assert!(version.is_some(), "cipher_version returned no row");
}

/// s02 (3.1; SPIKE option (c)): SQLCipher's crypto provider is CommonCrypto, not OpenSSL.
#[test]
fn cipher_provider_is_commoncrypto() {
    let dir = TempDir::new("s02-provider");
    let data = dir.data_dir();
    new_store(&data).close().unwrap();
    let connection = raw(&data);
    let provider: String = connection
        .query_row("PRAGMA cipher_provider", [], |r| r.get(0))
        .unwrap();
    assert_eq!(provider, "commoncrypto");
}

/// s03 (3.4 step 4): a key that does not open the file is `WrongKey`, final for the run; the
/// store never falls back to plaintext and the file is byte-identical.
#[test]
fn a_wrong_key_is_unavailable_never_plaintext() {
    let dir = TempDir::new("s03-wrong");
    let data = dir.data_dir();
    let mut store = new_store(&data);
    common::seed_agent(&mut store, "agent-secret", "Secret", T0 + 10);
    store.close().unwrap();
    let before = database_bytes(&data);

    let wrong = ScriptedKey::holding(KeyKind::Test, [0x47; 32]);
    let reason = unavailable(options(&data, wrong.clone(), T0 + 1_000));
    assert_eq!(reason, Unavailable::WrongKey);
    assert!(!reason.retry_may_help(), "a wrong key is final for the run");
    assert_eq!(wrong.creates(), 0);
    let after = database_bytes(&data);
    assert_eq!(after, before, "the refused open changed broker.db");
    assert_ne!(&after[..16], PLAINTEXT_HEADER);

    let store = open(&data, T0 + 2_000);
    assert!(has_agent(&store, "agent-secret"));
}

/// s03 (3.1, 3.4 step 4): a plaintext SQLite file at the store path is `NotEncrypted` and left
/// untouched; it is never opened as the store.
#[test]
fn a_plaintext_file_at_the_store_path_is_refused() {
    let dir = TempDir::new("s03-plain");
    let data = dir.data_dir();
    make_data_dir(&data);
    let plain = rusqlite::Connection::open(data.join(DATABASE_FILE)).unwrap();
    plain
        .execute_batch(
            "CREATE TABLE notes (body TEXT NOT NULL); INSERT INTO notes VALUES ('kept');",
        )
        .unwrap();
    drop(plain);
    let before = database_bytes(&data);
    assert_eq!(
        &before[..16],
        PLAINTEXT_HEADER,
        "the setup did not write plaintext"
    );

    let reason = unavailable(options(&data, TestKey, T0));
    assert_eq!(reason, Unavailable::NotEncrypted);
    assert!(!reason.retry_may_help());
    assert_eq!(
        database_bytes(&data),
        before,
        "the refused open changed the file"
    );

    // The file is still the plaintext database it was.
    let check = rusqlite::Connection::open(data.join(DATABASE_FILE)).unwrap();
    let body: String = check
        .query_row("SELECT body FROM notes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(body, "kept");
}
