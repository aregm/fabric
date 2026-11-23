//! Opening the store, its start-up checks, and its transactions (design 3.1, 3.4).
//!
//! One read-write connection, owned by the caller's thread; every write is
//! `BEGIN IMMEDIATE ... COMMIT`. Start-up also uses one short read-only probe connection.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::key::{KeyKind, KeyProvider, StoreKey};
use crate::migrate::{MIGRATIONS, Migration};
use crate::read::ReadTx;
use crate::tx::{AuditEntry, Committed, OpMeta, StoreFault, TxError, WriteTx, sql_fault};

/// `now_ms` is the raw wall clock: it dates a creation, a migration row (floored at the last `committed_at_ms`, 3.6)
/// and a backup name.
pub struct OpenOptions {
    /// The data directory; created with mode 0700 when absent.
    pub data_dir: std::path::PathBuf,
    /// The source of the store key.
    pub key: Box<dyn KeyProvider>,
    /// The raw wall clock in Unix milliseconds.
    pub now_ms: i64,
}

/// Why the store cannot be opened now. The broker answers in its unavailable mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// Another process holds the store's lock.
    InUse,
    /// The owner refused access to the Keychain item.
    KeychainRefused,
    /// The Keychain is locked or failed. Retried.
    KeychainLocked,
    /// `StoreKey::generate` failed (the system random source); no key and no file were created. Retried.
    KeyGeneration,
    /// The store file exists but its key does not.
    KeyMissing,
    /// The key does not open the store file.
    WrongKey,
    /// The store was created with a key of another kind.
    KeyKindMismatch {
        /// The kind the store records.
        stored: KeyKind,
    },
    /// The store file is a plaintext SQLite database (macOS).
    NotEncrypted,
    /// The store's schema is newer than this broker supports.
    NewerSchema {
        /// The store's schema version.
        found: u32,
        /// The newest schema version this broker supports.
        supported: u32,
    },
    /// A passing I/O condition stopped the start-up checks: SQLite answered `CANTOPEN`, `IOERR`,
    /// `FULL` or `NOMEM`, or the store file could not be read. The store is not judged. Retried.
    Io,
    /// `check` names the first failing check (fixed text, e.g. "ledger chain", "backup").
    Integrity {
        /// The first failing check.
        check: &'static str,
    },
}

impl Unavailable {
    /// `KeychainRefused` -> `KeychainRefused`; every other -> `Locked`.
    #[must_use]
    pub fn store_reason(&self) -> fabric_desktop_contracts::error::StoreReason {
        use fabric_desktop_contracts::error::StoreReason;
        match self {
            Unavailable::KeychainRefused => StoreReason::KeychainRefused,
            _ => StoreReason::Locked,
        }
    }

    /// True for `InUse`, `KeychainLocked`, `KeyGeneration` and `Io` only.
    #[must_use]
    pub fn retry_may_help(&self) -> bool {
        matches!(
            self,
            Unavailable::InUse
                | Unavailable::KeychainLocked
                | Unavailable::KeyGeneration
                | Unavailable::Io
        )
    }

    /// The owner message: cause and recovery, built from the reason alone (3.4).
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Unavailable::InUse => "Another Fabric Calendar broker is using the store. Nothing was \
                 changed; it tries again when the page next asks."
                .to_owned(),
            Unavailable::KeychainRefused => "Access to the store key in the Keychain was refused. \
                 Nothing was changed; allow access and start Fabric Calendar again."
                .to_owned(),
            Unavailable::KeychainLocked => {
                "The Keychain is locked or did not answer, so the store \
                 key cannot be read. Nothing was changed; it tries again when the page next asks."
                    .to_owned()
            }
            Unavailable::KeyGeneration => "The broker could not get random bytes from the system \
                 to create the store key. Nothing was written; it tries again when the page next \
                 asks."
                .to_owned(),
            Unavailable::KeyMissing => "The store exists but its key is missing from the \
                 Keychain. Nothing was changed; restore the key or move the store aside."
                .to_owned(),
            Unavailable::WrongKey => "The store key does not open the store. Nothing was \
                 changed; restore the right key or move the store aside."
                .to_owned(),
            Unavailable::KeyKindMismatch { stored } => format!(
                "The store was created with the {} key and this broker uses another. Nothing was \
                 changed; start the broker with the key the store was created with.",
                stored.as_str()
            ),
            Unavailable::NotEncrypted => "The store file is not encrypted. Nothing was changed; \
                 move the file aside so the broker can create an encrypted store."
                .to_owned(),
            Unavailable::NewerSchema { found, supported } => format!(
                "The store was written by a newer Fabric Calendar (schema {found}); this build \
                 reads schema {supported}. Nothing was changed."
            ),
            Unavailable::Io => "The store file could not be read or written just now (the disk \
                 may be full or the file not readable). Nothing was changed; it tries again when \
                 the page next asks."
                .to_owned(),
            Unavailable::Integrity { check } => format!(
                "The store failed its start-up check ({check}). Nothing was changed; restore a \
                 backup or move the store aside."
            ),
        }
    }

    /// A short fixed label for a log line.
    fn label(&self) -> &'static str {
        match self {
            Unavailable::InUse => "in use",
            Unavailable::KeychainRefused => "keychain refused",
            Unavailable::KeychainLocked => "keychain locked",
            Unavailable::KeyGeneration => "key generation failed",
            Unavailable::KeyMissing => "key missing",
            Unavailable::WrongKey => "wrong key",
            Unavailable::KeyKindMismatch { .. } => "key of another kind",
            Unavailable::NotEncrypted => "not encrypted",
            Unavailable::NewerSchema { .. } => "newer schema",
            Unavailable::Io => "i/o failure",
            Unavailable::Integrity { .. } => "integrity check failed",
        }
    }
}

/// Why [`Store::open`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Exit 78: failed SQLCipher build assertions or an unusable data directory. `check` is fixed text; `path` is the
    /// data directory when it is the cause.
    Config {
        /// The failed check.
        check: &'static str,
        /// The data directory, when it is the cause.
        path: Option<std::path::PathBuf>,
    },
    /// The store cannot be opened now; the broker runs in its unavailable mode.
    Unavailable(Unavailable),
}

impl std::fmt::Display for OpenError {
    /// Fixed text, the check, the path.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Config { check, path: None } => {
                write!(f, "store configuration refused: {check}")
            }
            OpenError::Config {
                check,
                path: Some(path),
            } => write!(
                f,
                "store configuration refused: {check} ({})",
                path.display()
            ),
            OpenError::Unavailable(Unavailable::Integrity { check }) => {
                write!(f, "store unavailable: integrity check failed ({check})")
            }
            OpenError::Unavailable(Unavailable::NewerSchema { found, supported }) => write!(
                f,
                "store unavailable: newer schema {found}, this build reads {supported}"
            ),
            OpenError::Unavailable(reason) => write!(f, "store unavailable: {}", reason.label()),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<Unavailable> for OpenError {
    fn from(reason: Unavailable) -> OpenError {
        OpenError::Unavailable(reason)
    }
}

/// The open store: its read-write connection and its lock.
pub struct Store {
    connection: Connection,
    _lock: std::fs::File,
    data_dir: PathBuf,
    store_id: String,
    key_kind: KeyKind,
    migrations: &'static [Migration],
}

impl Store {
    /// Section 3.4 steps 1-11. Writes nothing except a creation (migration 1) or a migration after its backup.
    /// A migration row's `committed_at_ms` is `max(options.now_ms, the last committed_at_ms)`, so a wall clock that
    /// went back between two runs never makes the ledger's time decrease (D34, 3.6).
    ///
    /// # Errors
    ///
    /// [`OpenError::Config`] for a failed build assertion or an unusable data directory;
    /// [`OpenError::Unavailable`] otherwise.
    pub fn open(options: OpenOptions) -> Result<Store, OpenError> {
        Store::open_with_migrations(options, MIGRATIONS)
    }

    /// Same with another migration list; tests rehearse N -> N+1 and a failing migration 1.
    ///
    /// # Errors
    ///
    /// As [`Store::open`].
    pub fn open_with_migrations(
        options: OpenOptions,
        migrations: &'static [Migration],
    ) -> Result<Store, OpenError> {
        let OpenOptions {
            data_dir,
            key,
            now_ms,
        } = options;
        if !crate::migrate::check_list(migrations) {
            return Err(OpenError::Config {
                check: "migration list",
                path: None,
            });
        }
        let supported = migrations.last().map_or(0, |m| m.to);

        // 1. The data directory.
        prepare_data_dir(&data_dir)?;
        // 2. The lock, for the life of the store.
        let lock = lock(&data_dir)?;
        // 3. The key.
        let database = data_dir.join(crate::DATABASE_FILE);
        let existed = std::fs::symlink_metadata(&database).is_ok();
        let store_key = obtain_key(key.as_ref(), existed)
            .map_err(|e| kind_refusal(&database, key.kind(), e))?;
        // 4. The probe of an existing file: nothing is written before step 6.
        let (found, objects) = if existed {
            refuse_plaintext(&database)?;
            probe(&database, &store_key).map_err(|e| kind_refusal(&database, key.kind(), e))?
        } else {
            (0, 0)
        };
        // 5. A newer schema.
        if found > supported {
            return Err(Unavailable::NewerSchema { found, supported }.into());
        }
        let created = found > 0;
        if !created && objects > 0 {
            return Err(Unavailable::Integrity {
                check: "unversioned schema",
            }
            .into());
        }
        // 6. The read-write connection.
        if !existed {
            create_private_file(&database).map_err(|_| OpenError::Config {
                check: "data directory",
                path: Some(data_dir.clone()),
            })?;
        }
        let mut connection = open_read_write(&database, &store_key)?;

        if created {
            // 8. The checks of a created store.
            check_created(&connection, key.kind())?;
            // 9. Backup, then migrate.
            if found < supported {
                let at_ms = floored(&connection, now_ms)?;
                crate::backup::take(&connection, &data_dir, &store_key, found, at_ms, migrations)
                    .map_err(|e| fault_or(&e, "backup"))?;
                for migration in migrations.iter().filter(|m| m.to > found) {
                    let at_ms = floored(&connection, now_ms)?;
                    crate::migrate::run(&mut connection, migration, migration.to - 1, at_ms, None)
                        .map_err(|e| fault_or(&e, "migration"))?;
                }
            }
        } else {
            // 7. Create: every migration, no backup (there is nothing to save).
            let store_id =
                crate::ledger::random_id("store-").map_err(|_| Unavailable::KeyGeneration)?;
            let mut creation = Some((store_id, key.kind()));
            for migration in migrations {
                let at_ms = floored(&connection, now_ms)?;
                crate::migrate::run(
                    &mut connection,
                    migration,
                    migration.to - 1,
                    at_ms,
                    creation.take(),
                )
                .map_err(|e| fault_or(&e, "migration"))?;
            }
        }
        // 10. The invariants of the current schema.
        check_invariants(&connection)?;
        // 11.
        let meta = ReadTx::new(&connection)
            .meta()
            .map_err(|e| fault_or(&e, "meta"))?;
        Ok(Store {
            connection,
            _lock: lock,
            data_dir,
            store_id: meta.store_id,
            key_kind: meta.key_kind,
            migrations,
        })
    }

    /// The store's id, fixed at creation.
    #[must_use]
    pub fn store_id(&self) -> &str {
        &self.store_id
    }

    /// The kind of key the store was created with.
    #[must_use]
    pub fn key_kind(&self) -> KeyKind {
        self.key_kind
    }

    /// The data directory.
    #[must_use]
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// The read-write connection, for tests that look under the store.
    #[cfg(test)]
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    /// One consistent read transaction.
    ///
    /// # Errors
    ///
    /// A [`StoreFault`] from the store or from `f`.
    pub fn read<T>(
        &self,
        f: impl FnOnce(&ReadTx<'_>) -> Result<T, StoreFault>,
    ) -> Result<T, StoreFault> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| sql_fault("begin read", &e))?;
        let value = f(&ReadTx::new(&tx))?;
        tx.finish().map_err(|e| sql_fault("end read", &e))?;
        Ok(value)
    }

    /// BEGIN IMMEDIATE; f; one ledger row if any mutation; COMMIT. `Refused` or `Fault` from `f` rolls back.
    ///
    /// # Errors
    ///
    /// `f`'s refusal, or a [`StoreFault`]; either leaves the store unchanged.
    pub fn write<T, E>(
        &mut self,
        meta: OpMeta,
        f: impl FnOnce(&mut WriteTx<'_>) -> Result<T, TxError<E>>,
    ) -> Result<Committed<T>, TxError<E>> {
        crate::ledger::check_operation_type(meta.operation_type)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| sql_fault("begin write", &e))?;
        let run_id = ReadTx::new(&tx).current_run()?;
        let mut write = WriteTx {
            tx,
            mutations: Vec::new(),
            notices: Vec::new(),
            now_ms: meta.now_ms,
            run_id,
            poisoned: None,
        };
        let value = f(&mut write)?;
        // A fault the closure went on after may have ended the transaction: append and commit nothing.
        write.usable()?;
        let WriteTx {
            tx,
            mutations,
            notices,
            run_id,
            ..
        } = write;
        let ledger_seq = if mutations.is_empty() {
            None
        } else {
            let run = ReadTx::new(&tx).current_run()?.or(run_id);
            Some(crate::ledger::append(&tx, &meta, &mutations, run)?)
        };
        tx.commit().map_err(|e| sql_fault("commit", &e))?;
        Ok(Committed {
            value,
            ledger_seq,
            notices,
        })
    }

    /// One transaction holding audit rows only; appends no ledger row (D24).
    ///
    /// # Errors
    ///
    /// A [`StoreFault`]; the store is then unchanged.
    pub fn write_audit(&mut self, now_ms: i64, entries: Vec<AuditEntry>) -> Result<(), StoreFault> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| sql_fault("begin audit", &e))?;
        let run_id = ReadTx::new(&tx).current_run()?;
        for entry in &entries {
            crate::tx::insert_audit(&tx, now_ms, run_id, entry)?;
        }
        tx.commit().map_err(|e| sql_fault("commit audit", &e))
    }

    /// Recomputes the chain from seq 1 (gap-free, hashes, non-decreasing `committed_at_ms`, head); returns the head seq.
    ///
    /// # Errors
    ///
    /// [`StoreFault::Chain`] at the first row that fails, or a fault of the store.
    pub fn verify_chain(&self) -> Result<u64, StoreFault> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| sql_fault("begin read", &e))?;
        let head = crate::ledger::verify_chain(&tx)?;
        tx.finish().map_err(|e| sql_fault("end read", &e))?;
        Ok(head)
    }

    /// Rebuilds the projection in memory from the ledger and compares every projection table.
    ///
    /// # Errors
    ///
    /// A [`StoreFault`] naming the first difference, or a fault of the store.
    pub fn verify_replay(&self) -> Result<(), StoreFault> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| sql_fault("begin read", &e))?;
        crate::ledger::verify_replay(&tx, self.migrations)?;
        tx.finish().map_err(|e| sql_fault("end read", &e))
    }

    /// Closes the connection and releases the lock.
    ///
    /// # Errors
    ///
    /// A [`StoreFault`] when the connection does not close cleanly.
    pub fn close(self) -> Result<(), StoreFault> {
        let Store { connection, .. } = self;
        connection.close().map_err(|(_, e)| sql_fault("close", &e))
    }
}

/// Step 1: create the directory (0700 on Unix) or check the mode of an existing one.
fn prepare_data_dir(data_dir: &Path) -> Result<(), OpenError> {
    let refuse = |check: &'static str| OpenError::Config {
        check,
        path: Some(data_dir.to_path_buf()),
    };
    match std::fs::metadata(data_dir) {
        Ok(metadata) if metadata.is_dir() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(refuse("data directory mode"));
                }
            }
            Ok(())
        }
        Ok(_) => Err(refuse("data directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt as _;
                builder.mode(0o700);
            }
            builder
                .create(data_dir)
                .map_err(|_| refuse("data directory"))
        }
        Err(_) => Err(refuse("data directory")),
    }
}

/// Opens a file for reading and writing, creating it with mode 0600 on Unix.
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

/// Creates an empty file with mode 0600, so SQLite's files (which copy its mode) are private too.
pub(crate) fn create_private_file(path: &Path) -> std::io::Result<()> {
    private_file(path).map(drop)
}

/// Step 2: the process-lifetime lock; the OS releases it when the process dies.
fn lock(data_dir: &Path) -> Result<std::fs::File, OpenError> {
    let file = private_file(&data_dir.join(crate::LOCK_FILE)).map_err(|_| OpenError::Config {
        check: "lock file",
        path: Some(data_dir.to_path_buf()),
    })?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(Unavailable::InUse.into()),
        Err(std::fs::TryLockError::Error(_)) => Err(OpenError::Config {
            check: "lock file",
            path: Some(data_dir.to_path_buf()),
        }),
    }
}

/// Step 3 (3.2): an existing file needs its key; with no file, a key left by a crash is reused,
/// and only when there is none is a new key generated and stored, before the file exists.
fn obtain_key(provider: &dyn KeyProvider, existed: bool) -> Result<StoreKey, OpenError> {
    match provider.read() {
        Ok(Some(key)) => Ok(key),
        Ok(None) if existed => Err(Unavailable::KeyMissing.into()),
        Ok(None) => {
            let key = StoreKey::generate().map_err(|e| OpenError::from(e.unavailable()))?;
            provider
                .create(&key)
                .map_err(|e| OpenError::from(e.unavailable()))?;
            Ok(key)
        }
        Err(error) => Err(error.unavailable().into()),
    }
}

/// 3.2: a test-key store opened by a provider of another kind is `KeyKindMismatch`, not the
/// `KeyMissing` or `WrongKey` the provider's own key meets at step 3 or 4 (the Keychain key never
/// equals the test key, so step 8 alone cannot see it). The test key is public: trying it reveals
/// nothing, and the file is opened read-only.
fn kind_refusal(database: &Path, kind: KeyKind, error: OpenError) -> OpenError {
    let refusal_of_the_key = matches!(
        error,
        OpenError::Unavailable(Unavailable::KeyMissing | Unavailable::WrongKey)
    );
    if refusal_of_the_key && kind != KeyKind::Test && test_keyed(database) {
        Unavailable::KeyKindMismatch {
            stored: KeyKind::Test,
        }
        .into()
    } else {
        error
    }
}

/// True when the test key opens `database` and its `meta` records the test key.
fn test_keyed(database: &Path) -> bool {
    let Ok(connection) = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let test = key_connection(&connection, &StoreKey::from_bytes(crate::key::TEST_KEY)).is_ok()
        && crate::tx::meta_value(&connection, "key_kind")
            .is_ok_and(|kind| kind.as_deref() == Some(KeyKind::Test.as_str()));
    let _ = connection.close();
    test
}

/// The file assertion of 3.1 (macOS): a plaintext SQLite header is refused.
fn refuse_plaintext(database: &Path) -> Result<(), OpenError> {
    #[cfg(target_os = "macos")]
    {
        use std::io::Read as _;
        let mut header = [0_u8; 16];
        let mut file = std::fs::File::open(database).map_err(|_| Unavailable::Io)?;
        let mut read = 0;
        while read < header.len() {
            match file.read(&mut header[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(Unavailable::Io.into()),
            }
        }
        if read == header.len() && &header == b"SQLite format 3\0" {
            return Err(Unavailable::NotEncrypted.into());
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = database;
    Ok(())
}

/// A failure while keying or checking a connection.
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "only SQLCipher on macOS refuses a key or a build; plain SQLite takes none"
    )
)]
pub(crate) enum Refusal {
    WrongKey,
    Io,
    Config(&'static str),
}

impl From<Refusal> for OpenError {
    fn from(refusal: Refusal) -> OpenError {
        match refusal {
            Refusal::WrongKey => Unavailable::WrongKey.into(),
            Refusal::Io => Unavailable::Io.into(),
            Refusal::Config(check) => OpenError::Config { check, path: None },
        }
    }
}

/// `SQLITE_NOTADB`: the key does not decrypt the file.
fn is_not_a_database(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(rusqlite::ErrorCode::NotADatabase)
}

/// A passing I/O condition: SQLite's `CANTOPEN`, `IOERR`, `FULL` or `NOMEM` family. At open it
/// is [`Unavailable::Io`], retried, never `Integrity` (whose recovery is to move the store aside).
fn is_passing_io(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(
            rusqlite::ErrorCode::CannotOpen
                | rusqlite::ErrorCode::SystemIoFailure
                | rusqlite::ErrorCode::DiskFull
                | rusqlite::ErrorCode::OutOfMemory
        )
    )
}

/// A failed start-up statement: [`Unavailable::Io`] for a passing I/O condition, else `Integrity`.
fn io_or(error: &rusqlite::Error, check: &'static str) -> OpenError {
    if is_passing_io(error) {
        Unavailable::Io.into()
    } else {
        Unavailable::Integrity { check }.into()
    }
}

/// The same for a [`StoreFault`]: only an SQLite code of those families is `Io`; a file error
/// of the store's own (a file where the backup directory belongs) stays `Integrity`.
fn fault_or(fault: &StoreFault, check: &'static str) -> OpenError {
    const PASSING: [i32; 4] = [
        rusqlite::ffi::SQLITE_CANTOPEN,
        rusqlite::ffi::SQLITE_IOERR,
        rusqlite::ffi::SQLITE_FULL,
        rusqlite::ffi::SQLITE_NOMEM,
    ];
    match fault {
        StoreFault::Sql { code, .. } if PASSING.contains(&(code & 0xff)) => Unavailable::Io.into(),
        _ => Unavailable::Integrity { check }.into(),
    }
}

/// The first column of the first row of a statement, as text, if it returns a row.
fn first_text(connection: &Connection, sql: &str) -> rusqlite::Result<Option<String>> {
    let mut statement = connection.prepare(sql)?;
    let mut rows = statement.query([])?;
    match rows.next()? {
        None => Ok(None),
        Some(row) => Ok(Some(match row.get::<_, rusqlite::types::Value>(0)? {
            rusqlite::types::Value::Text(text) => text,
            rusqlite::types::Value::Integer(n) => n.to_string(),
            _ => String::new(),
        })),
    }
}

/// Every first column of a statement's rows, as text.
#[cfg(target_os = "macos")]
fn all_text(connection: &Connection, sql: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Keys the connection (macOS) and runs the build assertions of 3.1 before any other statement.
/// The key statement lives in zeroed-on-drop memory and its error is never formatted.
pub(crate) fn key_connection(connection: &Connection, key: &StoreKey) -> Result<(), Refusal> {
    #[cfg(target_os = "macos")]
    {
        let literal = key.sql_literal();
        let mut statement = zeroize::Zeroizing::new(String::with_capacity(96));
        statement.push_str("PRAGMA key = \"");
        statement.push_str(&literal);
        statement.push_str("\";");
        connection
            .execute_batch(&statement)
            .map_err(|_| Refusal::WrongKey)?;
        drop(statement);
        let check = |sql: &str| {
            first_text(connection, sql).map_err(|e| {
                if is_not_a_database(&e) {
                    Refusal::WrongKey
                } else if is_passing_io(&e) {
                    Refusal::Io
                } else {
                    Refusal::Config("sqlcipher build")
                }
            })
        };
        if check("PRAGMA cipher_version")?.is_none_or(|v| v.is_empty()) {
            return Err(Refusal::Config("cipher_version"));
        }
        if check("PRAGMA cipher_provider")?.as_deref() != Some("commoncrypto") {
            return Err(Refusal::Config("cipher_provider"));
        }
        let options = all_text(connection, "PRAGMA compile_options").map_err(|e| {
            if is_not_a_database(&e) {
                Refusal::WrongKey
            } else if is_passing_io(&e) {
                Refusal::Io
            } else {
                Refusal::Config("compile_options")
            }
        })?;
        if !options.iter().any(|o| o == "HAS_CODEC") || !options.iter().any(|o| o == "TEMP_STORE=3")
        {
            return Err(Refusal::Config("compile_options"));
        }
        // SQLCipher's log level is process-wide: no log line, before the first page is read.
        let level = check("PRAGMA cipher_log_level = NONE")?;
        if level.is_some_and(|l| !l.eq_ignore_ascii_case("NONE")) {
            return Err(Refusal::Config("cipher_log_level"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (connection, key);
    Ok(())
}

/// Step 4: a read-only connection reads `user_version` and the number of schema objects.
fn probe(database: &Path, key: &StoreKey) -> Result<(u32, i64), OpenError> {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| probe_error(&e))?;
    key_connection(&connection, key)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| probe_error(&e))?;
    let objects: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
        .map_err(|e| probe_error(&e))?;
    connection.close().map_err(|(_, e)| probe_error(&e))?;
    let version = u32::try_from(version).map_err(|_| Unavailable::Integrity {
        check: "user_version",
    })?;
    Ok((version, objects))
}

/// The refusal of a failed probe.
fn probe_error(error: &rusqlite::Error) -> OpenError {
    if is_not_a_database(error) {
        Unavailable::WrongKey.into()
    } else if matches!(sql_fault("probe", error), StoreFault::Busy) {
        Unavailable::InUse.into()
    } else {
        io_or(error, "probe")
    }
}

/// Step 6: the read-write connection with the pragmas of 3.1, each asserted.
fn open_read_write(database: &Path, key: &StoreKey) -> Result<Connection, OpenError> {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| probe_error(&e))?;
    key_connection(&connection, key)?;
    set_pragmas(&connection)?;
    Ok(connection)
}

/// The pragmas of 3.1 after the key, in order, each read back.
fn set_pragmas(connection: &Connection) -> Result<(), OpenError> {
    let run = |sql: &str| {
        first_text(connection, sql).map_err(|e| -> OpenError {
            if is_not_a_database(&e) {
                Unavailable::WrongKey.into()
            } else if is_passing_io(&e) {
                Unavailable::Io.into()
            } else {
                OpenError::Config {
                    check: "pragma",
                    path: None,
                }
            }
        })
    };
    let expect = |sql: &str, wanted: &str, check: &'static str| -> Result<(), OpenError> {
        if run(sql)?.as_deref() == Some(wanted) {
            Ok(())
        } else {
            Err(OpenError::Config { check, path: None })
        }
    };
    #[cfg(target_os = "macos")]
    {
        run("PRAGMA cipher_memory_security = ON")?;
        expect(
            "PRAGMA cipher_memory_security",
            "1",
            "cipher_memory_security",
        )?;
    }
    run("PRAGMA temp_store = MEMORY")?;
    expect("PRAGMA temp_store", "2", "temp_store")?;
    run("PRAGMA secure_delete = ON")?;
    expect("PRAGMA secure_delete", "1", "secure_delete")?;
    expect("PRAGMA journal_mode = WAL", "wal", "journal_mode")?;
    run("PRAGMA synchronous = FULL")?;
    expect("PRAGMA synchronous", "2", "synchronous")?;
    run("PRAGMA fullfsync = ON")?;
    expect("PRAGMA fullfsync", "1", "fullfsync")?;
    run("PRAGMA checkpoint_fullfsync = ON")?;
    expect("PRAGMA checkpoint_fullfsync", "1", "checkpoint_fullfsync")?;
    run("PRAGMA foreign_keys = ON")?;
    expect("PRAGMA foreign_keys", "1", "foreign_keys")?;
    run("PRAGMA busy_timeout = 250")?;
    expect("PRAGMA busy_timeout", "250", "busy_timeout")?;
    Ok(())
}

/// The ledger time of a migration row: `max(now_ms, the last committed_at_ms)` (3.6, D34).
fn floored(connection: &Connection, now_ms: i64) -> Result<i64, OpenError> {
    let exists: bool = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'ledger'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| io_or(&e, "ledger"))?;
    if !exists {
        return Ok(now_ms);
    }
    let last = ReadTx::new(connection)
        .last_committed_ms()
        .map_err(|e| fault_or(&e, "ledger"))?;
    Ok(last.map_or(now_ms, |last| last.max(now_ms)))
}

/// Step 8: key kind, page and HMAC integrity, foreign keys, and the ledger chain.
fn check_created(connection: &Connection, kind: KeyKind) -> Result<(), OpenError> {
    let stored = ReadTx::new(connection)
        .meta()
        .map_err(|e| fault_or(&e, "meta"))?
        .key_kind;
    if stored != kind {
        return Err(Unavailable::KeyKindMismatch { stored }.into());
    }
    let integrity = first_text(connection, "PRAGMA integrity_check")
        .map_err(|e| io_or(&e, "integrity_check"))?;
    if integrity.as_deref() != Some("ok") {
        return Err(Unavailable::Integrity {
            check: "integrity_check",
        }
        .into());
    }
    #[cfg(target_os = "macos")]
    {
        let rows = first_text(connection, "PRAGMA cipher_integrity_check")
            .map_err(|e| io_or(&e, "cipher_integrity_check"))?;
        if rows.is_some() {
            return Err(Unavailable::Integrity {
                check: "cipher_integrity_check",
            }
            .into());
        }
    }
    let foreign = first_text(connection, "PRAGMA foreign_key_check")
        .map_err(|e| io_or(&e, "foreign_key_check"))?;
    if foreign.is_some() {
        return Err(Unavailable::Integrity {
            check: "foreign_key_check",
        }
        .into());
    }
    crate::ledger::verify_chain(connection).map_err(|e| fault_or(&e, "ledger chain"))?;
    Ok(())
}

/// Step 10: queries of the current schema that must return no row (3.4).
const INVARIANTS: [(&str, &str); 7] = [
    (
        "request state",
        "SELECT 1 FROM requests WHERE state NOT IN ('received', 'validating', \
         'proving_classification', 'allowed', 'pending_approval', 'blocked', 'undecided', \
         'approved', 'denied', 'expired', 'cancelled', 'stale', 'proving_fresh_permit', 'queued', \
         'dispatching', 'applied', 'reconciled', 'retry_wait', 'outcome_unknown', \
         'terminal_failure', 'reconciling', 'absence_confirmed', 'needs_owner_resolution', \
         'resolved_as_applied', 'resolved_as_absent', 'completed') \
         OR terminal != (state IN ('blocked', 'undecided', 'denied', 'expired', 'cancelled', \
         'stale', 'reconciled', 'terminal_failure', 'absence_confirmed', \
         'resolved_as_applied', 'resolved_as_absent', 'completed')) LIMIT 1",
    ),
    (
        "pending request without a card",
        "SELECT 1 FROM requests WHERE state = 'pending_approval' AND card IS NULL LIMIT 1",
    ),
    (
        "two live sessions of one agent",
        "SELECT 1 FROM sessions WHERE ended_at_ms IS NULL GROUP BY agent_id \
         HAVING count(*) > 1 LIMIT 1",
    ),
    (
        "session grant without its session",
        "SELECT 1 FROM grants g WHERE g.term_kind = 'session' AND NOT EXISTS \
         (SELECT 1 FROM sessions s WHERE s.session_id = g.session_id) LIMIT 1",
    ),
    (
        "issued authorization of an earlier run",
        "SELECT 1 FROM authorizations WHERE state = 'issued' \
         AND run_id < (SELECT max(run_id) FROM runs) LIMIT 1",
    ),
    (
        "relied grant of another agent",
        "SELECT 1 FROM requests r, json_each(r.relied_grant_ids) j WHERE NOT EXISTS \
         (SELECT 1 FROM grants g WHERE g.grant_id = j.value AND g.agent_id = r.agent_id) LIMIT 1",
    ),
    (
        "active profile",
        "SELECT 1 FROM profile_state p WHERE p.id = 1 AND NOT ( \
         (p.active_profile_id IN ('careful', 'flexible') AND p.active_version = 1) \
         OR EXISTS (SELECT 1 FROM profile_versions v WHERE v.profile_id = p.active_profile_id \
         AND v.version = p.active_version))",
    ),
];

/// Step 10: every invariant query returns no row.
fn check_invariants(connection: &Connection) -> Result<(), OpenError> {
    for (check, sql) in INVARIANTS {
        let found = first_text(connection, sql).map_err(|e| io_or(&e, check))?;
        if found.is_some() {
            return Err(Unavailable::Integrity { check }.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::key::{KeyError, TEST_KEY};
    use crate::mutation::Mutation;
    use crate::testing::{self, T0, TempDir};

    /// A key source whose answers the test sets, counting `create` calls.
    #[derive(Clone)]
    struct Scripted {
        kind: KeyKind,
        key: Arc<Mutex<Option<[u8; 32]>>>,
        read_error: Option<KeyError>,
        creates: Arc<Mutex<usize>>,
    }

    impl Scripted {
        fn new(key: Option<[u8; 32]>) -> Scripted {
            Scripted {
                kind: KeyKind::Test,
                key: Arc::new(Mutex::new(key)),
                read_error: None,
                creates: Arc::new(Mutex::new(0)),
            }
        }

        fn creates(&self) -> usize {
            *self.creates.lock().unwrap()
        }
    }

    impl KeyProvider for Scripted {
        fn kind(&self) -> KeyKind {
            self.kind
        }

        fn read(&self) -> Result<Option<StoreKey>, KeyError> {
            if let Some(error) = self.read_error {
                return Err(error);
            }
            Ok(self.key.lock().unwrap().map(StoreKey::from_bytes))
        }

        fn create(&self, key: &StoreKey) -> Result<(), KeyError> {
            *self.creates.lock().unwrap() += 1;
            *self.key.lock().unwrap() = Some(*key.bytes());
            Ok(())
        }
    }

    fn open_with(dir: &TempDir, key: Scripted) -> Result<Store, OpenError> {
        Store::open(OpenOptions {
            data_dir: dir.data(),
            key: Box::new(key),
            now_ms: T0,
        })
    }

    fn text(connection: &Connection, sql: &str) -> String {
        first_text(connection, sql).unwrap().unwrap_or_default()
    }

    #[test]
    fn the_read_write_connection_holds_every_pragma() {
        let dir = TempDir::new();
        let store = testing::store(&dir);
        let c = &store.connection;
        assert_eq!(text(c, "PRAGMA journal_mode"), "wal");
        assert_eq!(text(c, "PRAGMA synchronous"), "2");
        assert_eq!(text(c, "PRAGMA temp_store"), "2");
        assert_eq!(text(c, "PRAGMA secure_delete"), "1");
        assert_eq!(text(c, "PRAGMA fullfsync"), "1");
        assert_eq!(text(c, "PRAGMA checkpoint_fullfsync"), "1");
        assert_eq!(text(c, "PRAGMA foreign_keys"), "1");
        assert_eq!(text(c, "PRAGMA busy_timeout"), "250");
        #[cfg(target_os = "macos")]
        {
            assert_eq!(text(c, "PRAGMA cipher_provider"), "commoncrypto");
            assert_eq!(text(c, "PRAGMA cipher_memory_security"), "1");
            assert_eq!(text(c, "PRAGMA cipher_log_level"), "NONE");
            let options = all_text(c, "PRAGMA compile_options").unwrap();
            assert!(options.iter().any(|o| o == "HAS_CODEC"));
            assert!(options.iter().any(|o| o == "TEMP_STORE=3"));
        }
    }

    #[test]
    fn a_new_store_is_created_at_the_newest_schema_with_one_ledger_row() {
        let dir = TempDir::new();
        let store = testing::store(&dir);
        let meta = store.read(|r| r.meta()).unwrap();
        assert_eq!(meta.schema_version, crate::SCHEMA_VERSION);
        assert_eq!(meta.key_kind, KeyKind::Test);
        assert_eq!(meta.ledger_head_seq, 1);
        assert_eq!(meta.notice_sequence, 0);
        assert!(store.store_id().starts_with("store-"));
        assert_eq!(store.verify_chain().unwrap(), 1);
        store.verify_replay().unwrap();
        let (operation_type, at): (String, i64) = store
            .connection
            .query_row(
                "SELECT operation_type, committed_at_ms FROM ledger WHERE seq = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((operation_type.as_str(), at), ("schema.migrated", T0));
    }

    #[test]
    fn a_store_reopens_with_its_state() {
        let dir = TempDir::new();
        let store = testing::world(&dir);
        let id = store.store_id().to_owned();
        store.close().unwrap();
        let store = testing::store(&dir);
        assert_eq!(store.store_id(), id);
        assert_eq!(store.read(|r| r.agents()).unwrap().len(), 2);
        store.verify_replay().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempDir::new();
        let store = testing::store(&dir);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.data_dir()), 0o700);
        assert_eq!(mode(&store.data_dir().join(crate::DATABASE_FILE)), 0o600);
        assert_eq!(mode(&store.data_dir().join(crate::LOCK_FILE)), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_data_directory_open_to_others_is_a_configuration_error() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempDir::new();
        std::fs::create_dir(dir.data()).unwrap();
        std::fs::set_permissions(dir.data(), std::fs::Permissions::from_mode(0o750)).unwrap();
        let error = Store::open(testing::options(&dir, T0)).err().unwrap();
        assert_eq!(
            error,
            OpenError::Config {
                check: "data directory mode",
                path: Some(dir.data())
            }
        );
    }

    #[test]
    fn a_second_open_of_the_same_directory_is_in_use() {
        let dir = TempDir::new();
        let _store = testing::store(&dir);
        let error = Store::open(testing::options(&dir, T0)).err().unwrap();
        assert_eq!(error, OpenError::Unavailable(Unavailable::InUse));
    }

    #[test]
    fn an_existing_key_without_a_file_is_reused_and_never_created_again() {
        let dir = TempDir::new();
        let key = Scripted::new(Some([7; 32]));
        open_with(&dir, key.clone()).unwrap().close().unwrap();
        assert_eq!(key.creates(), 0);
        open_with(&dir, key.clone()).unwrap();
        assert_eq!(key.creates(), 0);
    }

    #[test]
    fn no_key_and_no_file_creates_the_key_once() {
        let dir = TempDir::new();
        let key = Scripted::new(None);
        open_with(&dir, key.clone()).unwrap().close().unwrap();
        assert_eq!(key.creates(), 1);
        assert!(key.key.lock().unwrap().is_some());
        open_with(&dir, key.clone()).unwrap();
        assert_eq!(key.creates(), 1);
    }

    #[test]
    fn a_file_without_its_key_is_key_missing_and_no_key_is_created() {
        let dir = TempDir::new();
        testing::store(&dir).close().unwrap();
        let before = std::fs::read(dir.data().join(crate::DATABASE_FILE)).unwrap();
        let key = Scripted::new(None);
        let error = open_with(&dir, key.clone()).err().unwrap();
        assert_eq!(error, OpenError::Unavailable(Unavailable::KeyMissing));
        assert_eq!(key.creates(), 0);
        let after = std::fs::read(dir.data().join(crate::DATABASE_FILE)).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn a_keychain_failure_is_its_unavailable_reason() {
        for (error, reason) in [
            (
                KeyError::Refused { status: -128 },
                Unavailable::KeychainRefused,
            ),
            (
                KeyError::Locked { status: -25308 },
                Unavailable::KeychainLocked,
            ),
            (KeyError::Failed { status: -4 }, Unavailable::KeychainLocked),
            (KeyError::Random, Unavailable::KeyGeneration),
        ] {
            let dir = TempDir::new();
            let mut key = Scripted::new(None);
            key.read_error = Some(error);
            let refused = open_with(&dir, key.clone()).err().unwrap();
            assert_eq!(refused, OpenError::Unavailable(reason));
            assert_eq!(key.creates(), 0);
            assert!(!dir.data().join(crate::DATABASE_FILE).exists());
        }
    }

    #[test]
    fn a_store_refuses_a_key_of_another_kind() {
        let dir = TempDir::new();
        testing::store(&dir).close().unwrap();
        let mut key = Scripted::new(Some(TEST_KEY));
        key.kind = KeyKind::Keychain;
        let error = open_with(&dir, key).err().unwrap();
        assert_eq!(
            error,
            OpenError::Unavailable(Unavailable::KeyKindMismatch {
                stored: KeyKind::Test
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_wrong_key_is_unavailable_and_the_file_is_never_plaintext() {
        let dir = TempDir::new();
        testing::store(&dir).close().unwrap();
        let file = std::fs::read(dir.data().join(crate::DATABASE_FILE)).unwrap();
        assert_ne!(&file[..16], b"SQLite format 3\0");
        let error = open_with(&dir, Scripted::new(Some([9; 32]))).err().unwrap();
        assert_eq!(error, OpenError::Unavailable(Unavailable::WrongKey));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_plaintext_store_file_is_refused() {
        let dir = TempDir::new();
        std::fs::create_dir(dir.data()).unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(dir.data(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let plain = Connection::open(dir.data().join(crate::DATABASE_FILE)).unwrap();
        plain.execute_batch("CREATE TABLE t (x INTEGER);").unwrap();
        plain.close().unwrap();
        let error = Store::open(testing::options(&dir, T0)).err().unwrap();
        assert_eq!(error, OpenError::Unavailable(Unavailable::NotEncrypted));
    }

    /// 3.4, D9: a passing I/O condition at open is `Io`, retried, and its message never
    /// tells the owner to move an intact store aside; a real integrity failure stays `Integrity`.
    #[test]
    fn a_passing_io_failure_at_open_is_retried_not_an_integrity_failure() {
        let failure =
            |code: i32| rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        for code in [
            rusqlite::ffi::SQLITE_CANTOPEN,
            rusqlite::ffi::SQLITE_IOERR,
            rusqlite::ffi::SQLITE_IOERR_SHORT_READ,
            rusqlite::ffi::SQLITE_FULL,
            rusqlite::ffi::SQLITE_NOMEM,
        ] {
            let io = OpenError::Unavailable(Unavailable::Io);
            assert_eq!(probe_error(&failure(code)), io, "probe, code {code}");
            assert_eq!(io_or(&failure(code), "integrity_check"), io, "code {code}");
            let fault = sql_fault("read", &failure(code));
            assert_eq!(fault_or(&fault, "meta"), io, "fault, code {code}");
        }
        let file_fault = StoreFault::Io {
            op: "backup",
            kind: std::io::ErrorKind::NotADirectory,
        };
        assert_eq!(
            fault_or(&file_fault, "backup"),
            OpenError::Unavailable(Unavailable::Integrity { check: "backup" })
        );
        assert_eq!(
            probe_error(&failure(rusqlite::ffi::SQLITE_CORRUPT)),
            OpenError::Unavailable(Unavailable::Integrity { check: "probe" })
        );
        assert_eq!(
            fault_or(
                &StoreFault::Chain {
                    seq: 2,
                    what: "hash"
                },
                "ledger chain"
            ),
            OpenError::Unavailable(Unavailable::Integrity {
                check: "ledger chain"
            })
        );
        assert_eq!(
            probe_error(&failure(rusqlite::ffi::SQLITE_BUSY)),
            OpenError::Unavailable(Unavailable::InUse)
        );
        assert!(Unavailable::Io.retry_may_help());
        assert!(!Unavailable::Integrity { check: "probe" }.retry_may_help());
        assert_eq!(
            Unavailable::Io.store_reason(),
            fabric_desktop_contracts::error::StoreReason::Locked
        );
        let message = Unavailable::Io.message();
        assert!(message.contains("tries again"), "{message}");
        assert!(!message.contains("aside"), "{message}");
    }

    /// The same through `Store::open`: a store file that cannot be read now is `Io`, nothing is
    /// changed, and once it can be read again the same store opens.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_store_file_is_unavailable_until_it_can_be_read() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempDir::new();
        let store = testing::store(&dir);
        let id = store.store_id().to_owned();
        store.close().unwrap();
        let database = dir.data().join(crate::DATABASE_FILE);
        let before = std::fs::read(&database).unwrap();
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&database).is_ok() {
            // A superuser reads any file: the condition cannot be made here.
            std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
            return;
        }
        let error = Store::open(testing::options(&dir, T0)).err().unwrap();
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(error, OpenError::Unavailable(Unavailable::Io));
        assert_eq!(std::fs::read(&database).unwrap(), before);
        let store = Store::open(testing::options(&dir, T0)).unwrap();
        assert_eq!(store.store_id(), id);
        store.verify_replay().unwrap();
    }

    #[test]
    fn a_newer_schema_is_refused_and_the_file_is_unchanged() {
        let dir = TempDir::new();
        let store = testing::store(&dir);
        store
            .connection
            .execute_batch("PRAGMA user_version = 2")
            .unwrap();
        store.close().unwrap();
        let before = std::fs::read(dir.data().join(crate::DATABASE_FILE)).unwrap();
        let error = Store::open(testing::options(&dir, T0)).err().unwrap();
        assert_eq!(
            error,
            OpenError::Unavailable(Unavailable::NewerSchema {
                found: 2,
                supported: 1
            })
        );
        let after = std::fs::read(dir.data().join(crate::DATABASE_FILE)).unwrap();
        assert_eq!(before, after);
    }

    static TWO: [Migration; 2] = [
        Migration {
            to: 1,
            sql: include_str!("../migrations/0001_initial.sql"),
        },
        Migration {
            to: 2,
            sql: "CREATE TABLE extra (x INTEGER NOT NULL) STRICT;",
        },
    ];

    static GAP: [Migration; 2] = [
        Migration {
            to: 1,
            sql: include_str!("../migrations/0001_initial.sql"),
        },
        Migration {
            to: 3,
            sql: "CREATE TABLE extra (x INTEGER NOT NULL) STRICT;",
        },
    ];

    static FAILING_TWO: [Migration; 2] = [
        Migration {
            to: 1,
            sql: include_str!("../migrations/0001_initial.sql"),
        },
        Migration {
            to: 2,
            sql: "CREATE TABLE extra (x INTEGER NOT NULL) STRICT; SELECT no_such_function();",
        },
    ];

    static FAILING_ONE: [Migration; 1] = [Migration {
        to: 1,
        sql: "CREATE TABLE half (x INTEGER) STRICT; SELECT no_such_function();",
    }];

    #[test]
    fn a_migration_row_is_dated_no_earlier_than_the_last_commit() {
        let dir = TempDir::new();
        let mut store = Store::open(testing::options(&dir, T0 + 10_000)).unwrap();
        testing::apply_all(
            &mut store,
            T0 + 10_000,
            vec![Mutation::RunStarted {
                run_id: 1,
                at_ms: T0 + 10_000,
            }],
        )
        .unwrap();
        store.close().unwrap();
        let store = Store::open_with_migrations(testing::options(&dir, T0), &TWO).unwrap();
        let (operation_type, at): (String, i64) = store
            .connection
            .query_row(
                "SELECT operation_type, committed_at_ms FROM ledger ORDER BY seq DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (operation_type.as_str(), at),
            ("schema.migrated", T0 + 10_000)
        );
        assert_eq!(store.read(|r| r.meta()).unwrap().schema_version, 2);
        assert_eq!(store.verify_chain().unwrap(), 3);
        store.verify_replay().unwrap();
        let backups: Vec<String> = std::fs::read_dir(dir.data().join(crate::BACKUP_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        let name = crate::backup::file_name(1, store.store_id(), 2, T0 + 10_000);
        assert_eq!(backups, vec![name]);
        store.close().unwrap();
        Store::open_with_migrations(testing::options(&dir, T0), &TWO)
            .unwrap()
            .verify_chain()
            .unwrap();
    }

    #[test]
    fn the_backup_opens_with_the_store_key_and_verifies() {
        let dir = TempDir::new();
        testing::world(&dir).close().unwrap();
        let store = Store::open_with_migrations(testing::options(&dir, T0 + 1), &TWO).unwrap();
        let name = crate::backup::file_name(1, store.store_id(), 2, T0 + 1);
        let path = dir.data().join(crate::BACKUP_DIR).join(name);
        let backup = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert!(key_connection(&backup, &StoreKey::from_bytes(TEST_KEY)).is_ok());
        assert_eq!(crate::apply::user_version(&backup).unwrap(), 1);
        assert_eq!(crate::ledger::verify_chain(&backup).unwrap(), 2);
        crate::ledger::verify_replay(&backup, &TWO).unwrap();
        let agents: i64 = backup
            .query_row("SELECT count(*) FROM agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(agents, 2);
    }

    #[test]
    fn migrations_run_in_order_and_a_gap_is_refused() {
        assert!(crate::migrate::check_list(&TWO));
        assert!(!crate::migrate::check_list(&GAP));
        assert!(!crate::migrate::check_list(&[]));
        let dir = TempDir::new();
        let error = Store::open_with_migrations(testing::options(&dir, T0), &GAP)
            .err()
            .unwrap();
        assert_eq!(
            error,
            OpenError::Config {
                check: "migration list",
                path: None
            }
        );
        let store = Store::open_with_migrations(testing::options(&dir, T0), &TWO).unwrap();
        let rows: Vec<String> = store
            .connection
            .prepare("SELECT payload FROM ledger ORDER BY seq")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains(r#""from":0,"op":"schema_migrated","to":1"#));
        assert!(rows[0].contains(r#""op":"store_created""#));
        assert!(rows[1].contains(r#""from":1,"op":"schema_migrated","to":2"#));
    }

    #[test]
    fn a_failed_migration_changes_nothing() {
        let dir = TempDir::new();
        testing::store(&dir).close().unwrap();
        let error = Store::open_with_migrations(testing::options(&dir, T0), &FAILING_TWO)
            .err()
            .unwrap();
        assert_eq!(
            error,
            OpenError::Unavailable(Unavailable::Integrity { check: "migration" })
        );
        let store = testing::store(&dir);
        assert_eq!(store.read(|r| r.meta()).unwrap().schema_version, 1);
        assert_eq!(store.verify_chain().unwrap(), 1);
    }

    #[test]
    fn a_store_left_without_tables_is_created_on_the_next_open() {
        let dir = TempDir::new();
        let error = Store::open_with_migrations(testing::options(&dir, T0), &FAILING_ONE)
            .err()
            .unwrap();
        assert_eq!(
            error,
            OpenError::Unavailable(Unavailable::Integrity { check: "migration" })
        );
        let store = testing::store(&dir);
        assert_eq!(store.verify_chain().unwrap(), 1);
    }
}
