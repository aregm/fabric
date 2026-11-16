//! The macOS Keychain round trip of the store key (design 3.2, 9.2; manual check M1 of 7.3).
//!
//! Ignored: it reads and writes the login Keychain, which no automated run may touch. The owner
//! runs it with `cargo test -p fabric-storage --test keychain -- --ignored`. It uses the test
//! service `ai.paracels.fabric.calendar.store.test`, never the product's, and leaves its item in
//! the Keychain, so a second run exercises the reuse rule of 3.2 (a key and no file).

#![cfg(target_os = "macos")]

mod common;

use common::{T0, TempDir, has_agent, open_with, options, seed_agent};
use fabric_storage::{KeyKind, KeychainKey};

const TEST_SERVICE: &str = "ai.paracels.fabric.calendar.store.test";

/// 3.2: a store created with the Keychain key reopens with a fresh provider of the same names,
/// so the key went into the Keychain and came back unchanged.
#[test]
#[ignore = "reads and writes the login Keychain; run with --ignored"]
fn keychain_key_round_trip() {
    let dir = TempDir::new("keychain");
    let data = dir.data_dir();
    let provider = KeychainKey::with_names(TEST_SERVICE, KeychainKey::ACCOUNT);
    let mut store = open_with(options(&data, provider, T0));
    assert_eq!(store.key_kind(), KeyKind::Keychain);
    common::start_run(&mut store, 1, T0);
    seed_agent(&mut store, "agent-keychain", "Keychain", T0 + 10);
    store.close().unwrap();

    let again = KeychainKey::with_names(TEST_SERVICE, KeychainKey::ACCOUNT);
    let store = open_with(options(&data, again, T0 + 1_000));
    assert!(has_agent(&store, "agent-keychain"));
}
