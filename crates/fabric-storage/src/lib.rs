//! The broker's encrypted store (work package CD-W04).
//!
//! One SQLite database in the broker's data directory: SQLCipher keyed with a raw 256-bit key
//! on macOS, plain SQLite elsewhere (a test store by design). It holds the ledger, a
//! hash-chained list of every change, and the projection, the current state those changes
//! build. Every projection change is a [`Mutation`] applied inside [`Store::write`]; the
//! transaction appends one ledger row and commits both together.
//!
//! - [`key`]: where the store key comes from (the Keychain or the public test key).
//! - [`store`]: opening the store, its checks and its transactions.
//! - [`tx`]: write transactions, their metadata and their failures.
//! - [`mutation`]: the changes a write may apply.
//! - [`rows`]: the records the store keeps.
//! - [`read`]: one consistent read of the store.
//! - [`ledger`]: the envelope, its hashes, verification and replay.
//! - [`migrate`]: the schema versions and their migrations.
//! - [`time`]: contract timestamps from Unix milliseconds.
//!
//! No error of this crate carries key material or row content: they hold fixed labels, codes
//! and numbers only.
//!
//! Numbers in this crate's comments, such as (3.4), (5.7) or (D17), and test ids such as s08
//! or c18, cite the sections, decisions and tests of the as-built design,
//! `docs/plans/calendar-desktop-w04-design.md`.

pub mod key;
#[cfg(target_os = "macos")]
mod keychain;
pub mod ledger;
pub mod migrate;
pub mod mutation;
pub mod read;
pub mod rows;
pub mod store;
pub mod time;
pub mod tx;

mod apply;
mod backup;
#[cfg(test)]
mod testing;

#[cfg(target_os = "macos")]
pub use key::KeychainKey;
pub use key::{KeyError, KeyKind, KeyProvider, StoreKey, TEST_KEY, TestKey};
pub use migrate::{MIGRATIONS, Migration, SCHEMA_VERSION};
pub use mutation::{Counter, Mutation, SessionEndReason, VoidReason};
pub use read::ReadTx;
pub use rows::*;
pub use store::{OpenError, OpenOptions, Store, Unavailable};
pub use time::{MAX_TIMESTAMP_MS, format_timestamp};
pub use tx::{
    Actor, ActorKind, AuditEntry, Committed, IdempotencyScope, OpMeta, StoreFault, TxError, WriteTx,
};

/// The store's database file in the data directory.
pub const DATABASE_FILE: &str = "broker.db";
/// The lock file in the data directory: one process opens the store at a time.
pub const LOCK_FILE: &str = "broker.lock";
/// The directory of the verified backups taken before a migration.
pub const BACKUP_DIR: &str = "backups";
/// Reserved for the prover's work directory, which the Isabelle wiring sets (L7(e)). W04 neither creates nor reads it.
pub const PROVER_DIR: &str = "prover";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{T0, TempDir};

    /// The test key in hex, as a key statement would hold it.
    fn key_hex() -> String {
        fabric_desktop_contracts::digest::hex(&TEST_KEY)
    }

    fn assert_clean(text: &str) {
        assert!(!text.contains(&key_hex()), "{text}");
        assert!(!text.to_lowercase().contains("pragma"), "{text}");
        assert!(!text.contains("MARKER"), "{text}");
    }

    #[test]
    fn faults_and_errors_print_no_key_and_no_content() {
        let faults = [
            StoreFault::Sql {
                op: "insert audit",
                code: 2067,
            },
            StoreFault::Io {
                op: "create backup",
                kind: std::io::ErrorKind::PermissionDenied,
            },
            StoreFault::Busy,
            StoreFault::Invariant {
                what: "a counter never decreases",
            },
            StoreFault::Chain {
                seq: 4,
                what: "the envelope hash",
            },
            StoreFault::Decode {
                table: "agents",
                column: "record",
            },
        ];
        for fault in &faults {
            assert_clean(&fault.to_string());
            assert_clean(&format!("{fault:?}"));
        }
        let keys = [
            KeyError::Refused { status: -128 },
            KeyError::Locked { status: -25308 },
            KeyError::Failed { status: -4 },
            KeyError::Random,
        ];
        for key in &keys {
            assert_clean(&key.to_string());
            assert_clean(&format!("{key:?}"));
        }
        let reasons = [
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
        ];
        for reason in reasons {
            assert_clean(&reason.message());
            let error = OpenError::Unavailable(reason);
            assert_clean(&error.to_string());
            assert_clean(&format!("{error:?}"));
        }
        let config = OpenError::Config {
            check: "data directory mode",
            path: Some("/tmp/fabric data".into()),
        };
        assert_eq!(
            config.to_string(),
            "store configuration refused: data directory mode (/tmp/fabric data)"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_wrong_key_error_prints_neither_key() {
        let dir = TempDir::new();
        Store::open(crate::testing::options(&dir, T0))
            .unwrap()
            .close()
            .unwrap();
        struct Other;
        impl KeyProvider for Other {
            fn kind(&self) -> KeyKind {
                KeyKind::Test
            }
            fn read(&self) -> Result<Option<StoreKey>, KeyError> {
                Ok(Some(StoreKey::from_bytes([0x5a; 32])))
            }
            fn create(&self, _: &StoreKey) -> Result<(), KeyError> {
                Ok(())
            }
        }
        let error = Store::open(OpenOptions {
            data_dir: dir.data(),
            key: Box::new(Other),
            now_ms: T0,
        })
        .err()
        .unwrap();
        for text in [error.to_string(), format!("{error:?}")] {
            assert_clean(&text);
            assert!(!text.contains(&"5a".repeat(32)));
        }
    }

    #[test]
    fn a_failed_backup_prints_no_key() {
        let dir = TempDir::new();
        Store::open(crate::testing::options(&dir, T0))
            .unwrap()
            .close()
            .unwrap();
        std::fs::write(dir.data().join(BACKUP_DIR), b"not a directory").unwrap();
        static TWO: [Migration; 2] = [
            Migration {
                to: 1,
                sql: include_str!("../migrations/0001_initial.sql"),
            },
            Migration {
                to: 2,
                sql: "CREATE TABLE extra (x INTEGER) STRICT;",
            },
        ];
        let error = Store::open_with_migrations(crate::testing::options(&dir, T0), &TWO)
            .err()
            .unwrap();
        assert_eq!(
            error,
            OpenError::Unavailable(Unavailable::Integrity { check: "backup" })
        );
        assert_clean(&error.to_string());
        assert_clean(&format!("{error:?}"));
        let _ = dir.path();
    }
}
