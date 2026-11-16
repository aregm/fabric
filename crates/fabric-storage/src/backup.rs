//! The verified backup taken before a migration (design 3.6): keyed with the store's key, named
//! by schema version and ledger head, and opened and verified before the migration runs.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::key::StoreKey;
use crate::migrate::Migration;
use crate::read::ReadTx;
use crate::tx::{StoreFault, invariant, sql_fault};

/// The backup file name: `broker-v{n}-{store_id}-h{ledger_head_seq}-{at_ms}.db`.
pub(crate) fn file_name(version: u32, store_id: &str, head: u64, at_ms: i64) -> String {
    format!("broker-v{version}-{store_id}-h{head}-{at_ms}.db")
}

/// Writes the backup of a store at schema `version`, then reopens it read-only with the same
/// key and verifies its chain, its replay, its head and its version. A failure removes the
/// backup file and leaves the store unchanged.
pub(crate) fn take(
    connection: &Connection,
    data_dir: &Path,
    key: &StoreKey,
    version: u32,
    at_ms: i64,
    migrations: &[Migration],
) -> Result<PathBuf, StoreFault> {
    let checkpoint: i64 = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(|e| sql_fault("checkpoint", &e))?;
    if checkpoint != 0 {
        return Err(StoreFault::Busy);
    }
    let meta = ReadTx::new(connection).meta()?;
    let directory = data_dir.join(crate::BACKUP_DIR);
    create_directory(&directory)?;
    let path = directory.join(file_name(
        version,
        &meta.store_id,
        meta.ledger_head_seq,
        at_ms,
    ));
    let result = write_and_verify(
        connection,
        &path,
        key,
        version,
        meta.ledger_head_seq,
        migrations,
    );
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result.map(|()| path)
}

/// Creates the backup directory with mode 0700 when it is missing.
fn create_directory(directory: &Path) -> Result<(), StoreFault> {
    if directory.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(directory).map_err(|e| StoreFault::Io {
        op: "create backup directory",
        kind: e.kind(),
    })
}

fn write_and_verify(
    connection: &Connection,
    path: &Path,
    key: &StoreKey,
    version: u32,
    head: u64,
    migrations: &[Migration],
) -> Result<(), StoreFault> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| StoreFault::Io {
            op: "replace backup",
            kind: e.kind(),
        })?;
    }
    crate::store::create_private_file(path).map_err(|e| StoreFault::Io {
        op: "create backup",
        kind: e.kind(),
    })?;
    let target = path.to_str().ok_or(invariant("the backup path is UTF-8"))?;
    export(connection, target, key, version)?;
    verify(path, key, version, head, migrations)
}

/// Copies the store into `target`, keyed with the same key (macOS: `sqlcipher_export`).
#[cfg(target_os = "macos")]
fn export(
    connection: &Connection,
    target: &str,
    key: &StoreKey,
    version: u32,
) -> Result<(), StoreFault> {
    let literal = key.sql_literal();
    connection
        .execute(
            "ATTACH DATABASE ?1 AS backup KEY ?2",
            rusqlite::params![target, literal.as_str()],
        )
        .map_err(|e| sql_fault("attach backup", &e))?;
    drop(literal);
    let exported = connection
        .query_row("SELECT sqlcipher_export('backup')", [], |_| Ok(()))
        .map_err(|e| sql_fault("export backup", &e))
        .and_then(|()| {
            // sqlcipher_export copies tables, indexes and triggers, not the user version.
            connection
                .execute_batch(&format!("PRAGMA backup.user_version = {version}"))
                .map_err(|e| sql_fault("export backup", &e))
        });
    let detached = connection
        .execute_batch("DETACH DATABASE backup")
        .map_err(|e| sql_fault("detach backup", &e));
    exported.and(detached)
}

/// Copies the store into `target` (`VACUUM INTO`, which keeps the user version).
#[cfg(not(target_os = "macos"))]
fn export(
    connection: &Connection,
    target: &str,
    key: &StoreKey,
    version: u32,
) -> Result<(), StoreFault> {
    let _ = (key, version);
    connection
        .execute("VACUUM INTO ?1", [target])
        .map_err(|e| sql_fault("export backup", &e))?;
    Ok(())
}

/// Opens the backup read-only with the key and checks it.
fn verify(
    path: &Path,
    key: &StoreKey,
    version: u32,
    head: u64,
    migrations: &[Migration],
) -> Result<(), StoreFault> {
    let backup = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| sql_fault("open backup", &e))?;
    crate::store::key_connection(&backup, key)
        .map_err(|_| invariant("the backup opens with the store key"))?;
    if crate::apply::user_version(&backup)? != version {
        return Err(invariant("the backup holds the schema it is named for"));
    }
    if crate::ledger::verify_chain(&backup)? != head {
        return Err(invariant(
            "the backup holds the ledger head it is named for",
        ));
    }
    crate::ledger::verify_replay(&backup, migrations)?;
    backup
        .close()
        .map_err(|(_, e)| sql_fault("close backup", &e))
}
